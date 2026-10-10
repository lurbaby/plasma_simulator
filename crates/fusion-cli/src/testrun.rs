//! A test run: generated discharges streamed through a model, then scored.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tok_sym_core::channels::CHANNEL_NAMES;
use tok_sym_core::generator::generate_shot;

use crate::mech::{slim, MECH};
use crate::model::ModelProcess;
use crate::run::{channel_cells, jet, opt, run_shot};
use crate::scoring::{score, Rule, ShotScore};

pub struct TestRunConfig<'a> {
    pub model_dir: &'a Path,
    pub entry: &'a str,
    pub python: &'a str,
    pub model_name: Option<String>,
    pub shots: usize,
    pub seed: u64,
    pub rule: Rule,
    pub step_timeout: Duration,
    /// Where to write measured.csv (+ column p), shots.csv and run.json.
    pub save_dir: Option<&'a Path>,
}

fn percentile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// Run the test. `progress(done)` after every shot; return `false` to stop.
pub fn test_run(cfg: &TestRunConfig, mut progress: impl FnMut(usize) -> bool) -> Result<Value, String> {
    let wall = Instant::now();
    let device = jet();
    let mut model = ModelProcess::start(cfg.model_dir, cfg.entry, cfg.python, cfg.shots)?;
    let mut measured = match cfg.save_dir {
        Some(d) => {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            let mut w = BufWriter::new(File::create(d.join("measured.csv")).map_err(|e| e.to_string())?);
            let mut h = String::from("shot_id,t,p");
            for n in CHANNEL_NAMES {
                h.push(',');
                h.push_str(n);
            }
            // Filled only when the model answers with mechanism shares.
            h.push_str(",model_lambda");
            for m in MECH {
                h.push_str(&format!(",share_{m}"));
            }
            writeln!(w, "{h}").map_err(|e| e.to_string())?;
            Some(w)
        }
        None => None,
    };
    let mut scores: Vec<ShotScore> = Vec::new();
    let mut lat_ms: Vec<f64> = Vec::new();
    let mut over_budget = 0usize;
    for i in 0..cfg.shots {
        let spec = generate_shot(&device, cfg.seed, i as u64);
        model.shot_start(i as u64, spec.duration, &spec.scenario)?;
        let mut stair: Vec<(f64, f64)> = Vec::new();
        let mut stair_info: Vec<Option<Value>> = Vec::new();
        // The model's split where its own λ peaked: shares are noise at low λ.
        let mut peak: Option<(f64, Value)> = None;
        let mut pmax = -1.0f64;
        let mut err: Option<String> = None;
        let outcome = run_shot(&device, &spec, |s| {
            let t = (s.time * 1e4).round() / 1e4;
            match model.step(t, &s.channels, cfg.step_timeout) {
                Ok((ans, dt)) => {
                    let p = ans.p;
                    if let Some(info) = ans.info.as_ref() {
                        if let (Some(lam), Some(sl)) = (info["risk_total"].as_f64(), slim(info)) {
                            if peak.as_ref().map_or(true, |(l, _)| lam > *l) {
                                peak = Some((lam, sl));
                            }
                        }
                    }
                    let ms = dt.as_secs_f64() * 1e3;
                    if ms > 5.0 {
                        over_budget += 1;
                    }
                    lat_ms.push(ms);
                    if p > pmax {
                        pmax = p;
                        stair.push((t, p));
                        stair_info.push(ans.info.as_ref().and_then(slim));
                    }
                    if let Some(w) = measured.as_mut() {
                        let mut line = format!("{i},{t:.4},{p}");
                        channel_cells(&mut line, &s.channels);
                        let info = ans.info.as_ref();
                        let lam = info.and_then(|v| v["risk_total"].as_f64());
                        line.push_str(&format!(",{}", lam.map(|x| x.to_string()).unwrap_or_default()));
                        for m in MECH {
                            let x = info.and_then(|v| v["share_pct"][m].as_f64());
                            line.push_str(&format!(",{}", x.map(|x| format!("{x:.3}")).unwrap_or_default()));
                        }
                        let _ = writeln!(w, "{line}");
                    }
                    true
                }
                Err(e) => {
                    err = Some(e);
                    false
                }
            }
        });
        if let Some(e) = err {
            return Err(format!("розряд {i}: {e}"));
        }
        model.shot_end(i as u64)?;
        scores.push(ShotScore {
            shot_id: i as u64,
            scenario: spec.scenario.clone(),
            disrupted: outcome.disrupted,
            t_tq: outcome.t_tq,
            t_end: stair.last().map(|x| x.0).or(Some(outcome.t_end)),
            n_steps: outcome.n_steps,
            spontaneous: outcome.spontaneous,
            wall_contact: outcome.wall_contact,
            t_alarm: None,
            warning_s: None,
            verdict: String::new(),
            staircase: stair,
            stair_info,
            true_cause: outcome.true_cause.clone(),
            predicted_cause: None,
            peak_cause: peak.map(|(_, v)| v),
            alarm_cause: None,
        });
        if !progress(i + 1) {
            return Err("скасовано".into());
        }
    }
    let (summary, curve) = score(&mut scores, &cfg.rule);
    let steps = lat_ms.len();
    let report = json!({
        "model": cfg.model_name.clone().unwrap_or_else(|| model.label.clone()),
        "run": {"shots": cfg.shots, "seed": cfg.seed, "noise_seed": null, "shift": false, "config": "lsn",
                "salted": false, "engine": "fusion-sim (noise-free)"},
        "summary": summary,
        "curve": curve,
        "rule": cfg.rule,
        "timing": {"steps": steps, "latency_p50_ms": percentile(&mut lat_ms.clone(), 0.5),
                   "latency_p99_ms": percentile(&mut lat_ms.clone(), 0.99),
                   "latency_max_ms": lat_ms.iter().cloned().fold(0.0, f64::max),
                   "over_budget_steps": over_budget, "budget_ms": 5.0, "wall_s": wall.elapsed().as_secs_f64()},
        "shots": scores,
    });
    if let Some(d) = cfg.save_dir {
        if let Some(mut w) = measured {
            w.flush().map_err(|e| e.to_string())?;
        }
        write_shots_csv(&d.join("shots.csv"), &report).map_err(|e| e.to_string())?;
        std::fs::write(d.join("run.json"), serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    }
    Ok(report)
}

/// shots.csv of a run: labels, alarm moment and verdict.
pub fn write_shots_csv(path: &Path, report: &Value) -> std::io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    writeln!(w, "shot_id,scenario,disrupted,t_tq_s,spontaneous,wall_contact,t_alarm_s,warning_s,verdict,n_steps")?;
    let shots: Vec<ShotScore> = serde_json::from_value(report["shots"].clone()).unwrap_or_default();
    for s in shots {
        writeln!(w, "{},{},{},{},{},{},{},{},{},{}", s.shot_id, s.scenario, s.disrupted, opt(s.t_tq), s.spontaneous,
            s.wall_contact, opt(s.t_alarm), opt(s.warning_s), s.verdict, s.n_steps)?;
    }
    w.flush()
}

/// The human summary toksym prints after a run.
pub fn print_summary(report: &Value) {
    let s = &report["summary"];
    let pct = |x: &Value| x.as_f64().map(|v| format!("{:.1} %", v * 100.0)).unwrap_or("—".into());
    eprintln!("Поріг:              {:.4} ({})", s["threshold"].as_f64().unwrap_or(1.0), s["threshold_source"].as_str().unwrap_or(""));
    eprintln!("Спіймано вчасно:    {} з {} ({})", s["tp"], s["n_disruptions_scored"], pct(&s["tpr"]));
    eprintln!("Пізні тривоги:      {}", s["late_alarms"]);
    eprintln!("Хибні тривоги:      {} з {} ({})", s["fp"], s["n_quiet"], pct(&s["fpr"]));
    eprintln!("Медіанне випередження: {}", s["median_warning_s"].as_f64().map(|v| format!("{v:.3} с")).unwrap_or("—".into()));
    eprintln!("Розрядів:           {} ({} зривів, з них {} спонтанних, {} об стінку)", s["n_shots"], s["n_disrupted"], s["n_spontaneous"], s["n_wall_contact"]);
    let t = &report["timing"];
    eprintln!("Крок моделі:        p50 {:.2} мс, p99 {:.2} мс, найгірший {:.2} мс",
        t["latency_p50_ms"].as_f64().unwrap_or(0.0), t["latency_p99_ms"].as_f64().unwrap_or(0.0), t["latency_max_ms"].as_f64().unwrap_or(0.0));
}
