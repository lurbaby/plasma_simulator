//! `fusionsim teach`: generated discharges under a fixer, logged for training
//! and evaluating a student model.
//!
//! Modes (`--mode`):
//! * `teacher` — the look-ahead fixer decides every `period` and its choice is applied;
//! * `label`   — the teacher decides and is logged but *not* applied, so the
//!   shot runs as without control: labels for the dangerous states the teacher
//!   itself steers away from;
//! * `student` — a student model (a submission folder, the usual protocol)
//!   sees the instruments every step and its `action` is applied at each
//!   decision; with `--audit` the teacher's choice in the same state is logged
//!   next to it;
//! * `none`    — no control (the baseline).
//!
//! Shots are seeded exactly as in `gen`/`score`, so modes compare pairwise.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;
use tok_sym_core::channels::CHANNEL_NAMES;
use tok_sym_core::fixer::{apply_action, decide, Decision, FixerConfig, ACTIONS};
use tok_sym_core::generator::generate_shot;
use tok_sym_core::simulation::{PulseProgram, Simulation, SimulationStatus};

use crate::mech::true_split;
use crate::model::ModelProcess;
use crate::run::{channel_cells, extra_cells, extra_header, jet, opt, scramble, DT, SPONTANEOUS_RISK};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    None,
    Teacher,
    Label,
    Student,
    /// The simple rule-based advisor of the control panel: when 1 − e^(−λ·1 с)
    /// exceeds 0.3, one fixed action against the nearest limit, no look-ahead.
    Rules,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Mode, String> {
        Ok(match s {
            "none" => Mode::None,
            "teacher" => Mode::Teacher,
            "label" => Mode::Label,
            "student" => Mode::Student,
            "rules" => Mode::Rules,
            _ => return Err(format!("невідомий --mode {s}: none | teacher | label | student | rules")),
        })
    }
    fn name(self) -> &'static str {
        match self {
            Mode::None => "none",
            Mode::Teacher => "teacher",
            Mode::Label => "label",
            Mode::Student => "student",
            Mode::Rules => "rules",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StudentSpec {
    pub dir: PathBuf,
    pub entry: String,
    pub python: String,
}

#[derive(Debug, Clone)]
pub struct TeachConfig {
    pub shots: usize,
    pub seed: u64,
    pub mode: Mode,
    pub fixer: FixerConfig,
    /// Seconds between decisions
    pub period: f64,
    pub stride: usize,
    pub extras: Vec<String>,
    pub jobs: usize,
    pub student: Option<StudentSpec>,
    /// In student mode, also log the teacher's decision in the same state.
    pub audit: bool,
}

struct ShotOut {
    measured: String,
    decisions: String,
    shot_line: String,
    disrupted: bool,
    spontaneous: bool,
    actions: usize,
    decisions_n: usize,
    agree: usize,
    w_mean: f64,
}

fn decision_line(i: usize, t: f64, chosen: usize, before: &tok_sym_core::simulation::ProgramScales, lam: f64,
                 teacher: Option<&Decision>) -> String {
    let mut line = format!(
        "{i},{t:.4},{chosen},{},{:.4},{:.4},{:.4},{:.4},{:.4},{lam:.6}",
        ACTIONS[chosen], before.ip, before.ne_target, before.d2_puff, before.neon_puff, before.heating
    );
    match teacher {
        Some(d) => {
            for v in &d.variants {
                line.push_str(&format!(",{:.6},{:.6},{:.6}", v.p, v.loss, v.cost));
            }
            line.push_str(&format!(",{},{}", d.best, ACTIONS[d.best]));
        }
        None => {
            for _ in 0..ACTIONS.len() {
                line.push_str(",,,");
            }
            line.push_str(",,");
        }
    }
    line.push('\n');
    line
}

/// The panel's simple advisor, expressed in fixer actions: nearest limit → one action.
fn rules_action(state: Option<(f64, f64, f64, f64, f64)>) -> usize {
    let Some((lam, fgw, q95, frad, bn)) = state else { return 0 };
    if 1.0 - (-lam).exp() <= 0.3 {
        return 0;
    }
    let scores = [fgw / 1.0, 2.2 / q95.max(0.5) / 0.9, frad / 0.8, bn / 2.8 / 0.85];
    let k = (0..4).max_by(|&a, &b| scores[a].partial_cmp(&scores[b]).unwrap()).unwrap();
    let name = ["ne_-20", "ip_-10", "neon_off", "heat_-20"][k];
    ACTIONS.iter().position(|a| *a == name).unwrap()
}

fn run_one(cfg: &TeachConfig, i: usize, student: Option<&mut ModelProcess>) -> Result<ShotOut, String> {
    let device = jet();
    let spec = generate_shot(&device, cfg.seed, i as u64);
    let mut sim = Simulation::new(device.clone(), spec.program.clone());
    sim.compute_geometry = false;
    sim.plan = Some(spec.plan.clone());
    sim.seed_disruption(scramble(spec.seed));
    sim.start();
    let mut student = student;
    if let Some(m) = student.as_deref_mut() {
        m.shot_start(i as u64, spec.duration, &spec.scenario)?;
    }
    let (mut measured, mut decisions) = (String::new(), String::new());
    let n_max = (spec.duration / DT).round() as usize;
    let (mut n_steps, mut t_end, mut w_sum) = (0usize, 0.0, 0.0);
    let (mut next_decision, mut n_dec, mut n_act, mut n_agree) = (0.0f64, 0usize, 0usize, 0usize);
    let mut peak_ip = 0.0f64;
    let mut true_cause = None;
    let mut student_action: Option<usize> = None;
    let mut last_state: Option<(f64, f64, f64, f64, f64)> = None; // λ, f_GW, q95, P_rad/P_in, βN
    for k in 0..n_max {
        // Decide before stepping, from the state the instruments just showed.
        let sched_ip = PulseProgram::interpolate(&spec.program.ip, sim.time);
        peak_ip = peak_ip.max(sched_ip);
        let rampdown = peak_ip > 0.5 && sched_ip < 0.9 * peak_ip;
        let decide_now = cfg.mode != Mode::None && sim.t_trigger.is_none() && !rampdown && sched_ip > 0.24
            && sim.time + 1e-9 >= next_decision;
        if decide_now {
            next_decision = sim.time + cfg.period;
            let before = sim.scales;
            let lam = sim.risk();
            let teacher = match cfg.mode {
                Mode::Teacher | Mode::Label => Some(decide(&sim, &cfg.fixer)),
                Mode::Student if cfg.audit => Some(decide(&sim, &cfg.fixer)),
                _ => None,
            };
            let chosen = match cfg.mode {
                Mode::Student => student_action.unwrap_or(0),
                Mode::Rules => rules_action(last_state),
                _ => teacher.as_ref().map(|d| d.best).unwrap_or(0),
            };
            if cfg.mode != Mode::Label {
                sim.scales = apply_action(chosen, &sim.scales);
            }
            n_dec += 1;
            n_act += (chosen != 0) as usize;
            n_agree += teacher.as_ref().map_or(0, |d| (d.best == chosen) as usize);
            decisions.push_str(&decision_line(i, sim.time, chosen, &before, lam, teacher.as_ref()));
        }
        let s = sim.step(DT);
        if true_cause.is_none() && s.t_trigger.is_some() {
            true_cause = Some(true_split(&s));
        }
        if s.t_tq.is_some() || s.status == SimulationStatus::Complete {
            break;
        }
        let frad = if s.p_input > 1.0 { s.p_rad / s.p_input } else { 0.0 };
        last_state = Some((s.disruption_risk, s.f_greenwald, s.q95, frad, s.beta_n));
        if let Some(m) = student.as_deref_mut() {
            let sc = s.scales;
            let extra = json!({"scales": {"ip": sc.ip, "ne_target": sc.ne_target, "d2_puff": sc.d2_puff,
                "neon_puff": sc.neon_puff, "heating": sc.heating}});
            let (ans, _) = m.step_extra(s.time, &s.channels, extra, Duration::from_secs(2))?;
            student_action = ans.info.as_ref()
                .and_then(|v| v["action"].as_str())
                .and_then(|a| ACTIONS.iter().position(|x| *x == a));
        }
        n_steps += 1;
        t_end = s.time;
        w_sum += s.w_th;
        if k % cfg.stride.max(1) == 0 {
            let mut line = format!("{i},{:.4}", s.time);
            channel_cells(&mut line, &s.channels);
            let sc = s.scales;
            line.push_str(&format!(",{:.4},{:.4},{:.4},{:.4},{:.4},{:.6}", sc.ip, sc.ne_target, sc.d2_puff, sc.neon_puff, sc.heating, s.disruption_risk));
            extra_cells(&mut line, &s, &cfg.extras);
            measured.push_str(&line);
            measured.push('\n');
        }
        if s.status != SimulationStatus::Running {
            break;
        }
    }
    if let Some(m) = student.as_deref_mut() {
        m.shot_end(i as u64)?;
    }
    let disrupted = sim.t_trigger.is_some();
    let spontaneous = disrupted && !sim.wall_contact && sim.risk_at_trigger < SPONTANEOUS_RISK;
    let cause = true_cause.as_ref().and_then(|c| c["dominant"].as_str().map(String::from)).unwrap_or_default();
    let w_mean = w_sum / n_steps.max(1) as f64;
    let shot_line = format!(
        "{i},{},{},{},{:.4},{},{},{},{},{},{},{},{:.4},{},{},{},{:.4}",
        spec.seed, cfg.seed, spec.scenario, spec.duration, disrupted, opt(sim.t_trigger), opt(sim.t_tq), spontaneous,
        sim.wall_contact, cause, n_steps, t_end, cfg.mode.name(), n_dec, n_act, w_mean
    );
    Ok(ShotOut { measured, decisions, shot_line, disrupted, spontaneous, actions: n_act, decisions_n: n_dec, agree: n_agree, w_mean })
}

pub fn teach(out: &Path, cfg: &TeachConfig, mut progress: impl FnMut(usize)) -> Result<String, String> {
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(out).map_err(io)?;
    let mut m = BufWriter::new(File::create(out.join("measured.csv")).map_err(io)?);
    let mut d = BufWriter::new(File::create(out.join("decisions.csv")).map_err(io)?);
    let mut sh = BufWriter::new(File::create(out.join("shots.csv")).map_err(io)?);
    let mut h = String::from("shot_id,t");
    for n in CHANNEL_NAMES {
        h.push(',');
        h.push_str(n);
    }
    h.push_str(",scale_ip,scale_ne_target,scale_d2_puff,scale_neon_puff,scale_heating,lambda_true");
    for e in extra_header(&cfg.extras) {
        h.push(',');
        h.push_str(&e);
    }
    writeln!(m, "{h}").map_err(io)?;
    // `best` = the action actually chosen (teacher's, or the student's in student mode);
    // the per-action columns and `teacher_best` are the teacher's evaluation.
    let mut dh = String::from("shot_id,t,best,best_action,scale_ip,scale_ne_target,scale_d2_puff,scale_neon_puff,scale_heating,lambda_true");
    for a in ACTIONS {
        dh.push_str(&format!(",p_{a},loss_{a},cost_{a}"));
    }
    dh.push_str(",teacher_best,teacher_best_action");
    writeln!(d, "{dh}").map_err(io)?;
    writeln!(sh, "shot_id,seed,master_seed,scenario,duration_s,disrupted,t_trigger_s,t_tq_s,spontaneous,wall_contact,true_cause,n_steps,t_end_s,mode,n_decisions,n_actions,w_mean_MJ").map_err(io)?;

    // Workers take shots round-robin; each keeps one student process alive.
    let jobs = cfg.jobs.max(1).min(cfg.shots.max(1));
    let (tx, rx) = mpsc::channel::<(usize, Result<ShotOut, String>)>();
    let mut buffer: std::collections::BTreeMap<usize, ShotOut> = Default::default();
    let (mut disr, mut spont, mut acts, mut decs, mut agree, mut w) = (0usize, 0usize, 0usize, 0usize, 0usize, 0.0);
    let mut written = 0usize;
    let mut failure: Option<String> = None;
    std::thread::scope(|sc| {
        for wkr in 0..jobs {
            let tx = tx.clone();
            sc.spawn(move || {
                let mut proc = match &cfg.student {
                    Some(st) if cfg.mode == Mode::Student => match ModelProcess::start(&st.dir, &st.entry, &st.python, cfg.shots) {
                        Ok(p) => Some(p),
                        Err(e) => {
                            let _ = tx.send((wkr, Err(e)));
                            return;
                        }
                    },
                    _ => None,
                };
                let mut k = wkr;
                while k < cfg.shots {
                    let r = run_one(cfg, k, proc.as_mut());
                    let stop = r.is_err();
                    if tx.send((k, r)).is_err() || stop {
                        return;
                    }
                    k += jobs;
                }
            });
        }
        drop(tx);
        for (k, r) in rx {
            match r {
                Ok(o) => {
                    buffer.insert(k, o);
                }
                Err(e) => {
                    failure.get_or_insert(format!("розряд {k}: {e}"));
                    continue;
                }
            }
            while let Some(o) = buffer.remove(&written) {
                let res = m.write_all(o.measured.as_bytes())
                    .and_then(|_| d.write_all(o.decisions.as_bytes()))
                    .and_then(|_| writeln!(sh, "{}", o.shot_line));
                if let Err(e) = res {
                    failure.get_or_insert(e.to_string());
                }
                disr += o.disrupted as usize;
                spont += o.spontaneous as usize;
                acts += o.actions;
                decs += o.decisions_n;
                agree += o.agree;
                w += o.w_mean;
                written += 1;
                if written % 10 == 0 || written == cfg.shots {
                    progress(written);
                }
            }
        }
    });
    m.flush().map_err(io)?;
    d.flush().map_err(io)?;
    sh.flush().map_err(io)?;
    if let Some(e) = failure {
        return Err(e);
    }
    let label = match cfg.mode {
        Mode::None => "без керування",
        Mode::Teacher => "вчитель",
        Mode::Label => "розмітка вчителем без дій",
        Mode::Student => "учень",
        Mode::Rules => "простий порадник",
    };
    let audit = if cfg.mode == Mode::Student && cfg.audit && decs > 0 {
        format!(", збіг з вчителем {:.1}%", 100.0 * agree as f64 / decs as f64)
    } else {
        String::new()
    };
    Ok(format!(
        "{} розрядів ({label}): зірвалось {disr} (з них спонтанних {spont}), рішень {decs}, дій {acts}{audit}, середня W {:.3} МДж",
        cfg.shots, w / cfg.shots.max(1) as f64
    ))
}
