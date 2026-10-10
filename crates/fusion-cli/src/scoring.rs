//! Test-run scoring with toksym's rules.
//!
//! The alarm in a shot is the first step where p exceeds the threshold. Each
//! shot gets a verdict: caught (alarm at least `lead` before the thermal
//! quench), late, missed, false_alarm (alarm on a shot that ran to the end),
//! quiet, or excluded (spontaneous disruption, wall contact, or quench not
//! recorded). The threshold is swept: the lowest one that maximises the
//! caught fraction while the false-alarm fraction stays within the budget.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const LEAD_TIMES: [f64; 5] = [0.01, 0.03, 0.1, 0.3, 1.0];
pub const BUDGETS: [f64; 6] = [0.02, 0.05, 0.1, 0.2, 0.3, 0.5];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotScore {
    pub shot_id: u64,
    pub scenario: String,
    pub disrupted: bool,
    pub t_tq: Option<f64>,
    pub t_end: Option<f64>,
    pub n_steps: usize,
    pub spontaneous: bool,
    pub wall_contact: bool,
    pub t_alarm: Option<f64>,
    pub warning_s: Option<f64>,
    pub verdict: String,
    /// (t, p) at every new running maximum of p.
    pub staircase: Vec<(f64, f64)>,
    /// The model's mechanism answer at each staircase point (models that send shares).
    #[serde(default)]
    pub stair_info: Vec<Option<Value>>,
    /// The engine's split of λ at the trigger: {share_pct, risk_total, dominant}.
    #[serde(default)]
    pub true_cause: Option<Value>,
    /// The model's split where its own λ peaked (best guess of the cause).
    #[serde(default)]
    pub peak_cause: Option<Value>,
    /// The model's predicted cause: at its λ peak, else at the alarm.
    #[serde(default)]
    pub predicted_cause: Option<Value>,
    /// The model's split at the alarm moment.
    #[serde(default)]
    pub alarm_cause: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rule {
    pub lead_time_s: f64,
    pub fpr_budget: f64,
    pub exclude_spontaneous: bool,
    pub exclude_wall_contact: bool,
    pub threshold: Option<f64>,
}

impl ShotScore {
    pub fn excluded(&self, rule: &Rule) -> bool {
        self.disrupted
            && ((rule.exclude_spontaneous && self.spontaneous)
                || (rule.exclude_wall_contact && self.wall_contact)
                || self.t_tq.is_none())
    }
    /// First moment p exceeds `thr`.
    pub fn alarm(&self, thr: f64) -> Option<f64> {
        self.staircase.iter().find(|(_, p)| *p > thr).map(|(t, _)| *t)
    }
}

#[derive(Debug, Clone, Default)]
struct Point {
    threshold: f64,
    tp: usize,
    fp: usize,
    late: usize,
    warnings: Vec<f64>,
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) })
}

fn evaluate(shots: &[ShotScore], rule: &Rule, lead: f64, thr: f64) -> Point {
    let mut p = Point { threshold: thr, ..Default::default() };
    for s in shots.iter().filter(|s| !s.excluded(rule)) {
        let alarm = s.alarm(thr);
        if s.disrupted {
            if let (Some(ta), Some(tq)) = (alarm, s.t_tq) {
                let w = tq - ta;
                if w >= lead {
                    p.tp += 1;
                    p.warnings.push(w);
                } else {
                    p.late += 1;
                }
            }
        } else if alarm.is_some() {
            p.fp += 1;
        }
    }
    p
}

/// Thresholds between consecutive distinct p values (every distinct alarm
/// pattern), plus 0 and 1.
fn candidates(shots: &[ShotScore]) -> Vec<f64> {
    let mut v: Vec<f64> = shots.iter().flat_map(|s| s.staircase.iter().map(|x| x.1)).collect();
    v.push(0.0);
    v.push(1.0);
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.dedup();
    let mut c = vec![0.0];
    for w in v.windows(2) {
        c.push(0.5 * (w[0] + w[1]));
    }
    c.push(1.0);
    c.dedup();
    c
}

fn counts(shots: &[ShotScore], rule: &Rule) -> (usize, usize) {
    let scored = shots.iter().filter(|s| s.disrupted && !s.excluded(rule)).count();
    let quiet = shots.iter().filter(|s| !s.disrupted).count();
    (scored, quiet)
}

fn pick(shots: &[ShotScore], rule: &Rule, lead: f64, budget: f64, cands: &[f64]) -> Point {
    let (_, n_quiet) = counts(shots, rule);
    let mut best: Option<Point> = None;
    for &thr in cands {
        let p = evaluate(shots, rule, lead, thr);
        let fpr = if n_quiet > 0 { p.fp as f64 / n_quiet as f64 } else { 0.0 };
        if fpr > budget + 1e-12 {
            continue;
        }
        if best.as_ref().map_or(true, |b| p.tp > b.tp) {
            best = Some(p);
        }
    }
    match best {
        Some(b) if b.tp > 0 => b,
        _ => evaluate(shots, rule, lead, 1.0),
    }
}

/// Apply the rule: fill verdicts in place and build the report summary,
/// threshold curves and reference operating points.
pub fn score(shots: &mut [ShotScore], rule: &Rule) -> (Value, Value) {
    let cands = candidates(shots);
    let (n_scored, n_quiet) = counts(shots, rule);
    let chosen = match rule.threshold {
        Some(t) => evaluate(shots, rule, rule.lead_time_s, t),
        None => pick(shots, rule, rule.lead_time_s, rule.fpr_budget, &cands),
    };
    let thr = chosen.threshold;
    for s in shots.iter_mut() {
        let ex = s.excluded(rule);
        s.t_alarm = s.alarm(thr);
        let idx = s.staircase.iter().position(|(_, p)| *p > thr).or(s.staircase.len().checked_sub(1));
        s.alarm_cause = s.t_alarm.and(idx).and_then(|i| s.stair_info.get(i).cloned().flatten());
        s.predicted_cause = s.peak_cause.clone().or_else(|| idx.and_then(|i| s.stair_info.get(i).cloned().flatten()));
        s.warning_s = match (s.t_alarm, s.t_tq) {
            (Some(a), Some(q)) => Some(q - a),
            _ => None,
        };
        s.verdict = if ex {
            "excluded"
        } else if s.disrupted {
            match s.warning_s {
                Some(w) if w >= rule.lead_time_s => "caught",
                Some(_) => "late",
                None => "missed",
            }
        } else if s.t_alarm.is_some() {
            "false_alarm"
        } else {
            "quiet"
        }
        .to_string();
    }
    let rate = |a: usize, n: usize| if n > 0 { a as f64 / n as f64 } else { 0.0 };
    let mechanisms = mechanism_summary(shots, rule);
    let mut w = chosen.warnings.clone();
    let summary = json!({
        "threshold": thr,
        "tpr": rate(chosen.tp, n_scored),
        "fpr": rate(chosen.fp, n_quiet),
        "tp": chosen.tp,
        "fn": n_scored - chosen.tp,
        "fp": chosen.fp,
        "tn": n_quiet - chosen.fp,
        "late_alarms": chosen.late,
        "median_warning_s": median(&mut w),
        "n_disruptions_scored": n_scored,
        "n_quiet": n_quiet,
        "threshold_source": if rule.threshold.is_some() { "fixed" } else { "swept to budget" },
        "n_shots": shots.len(),
        "n_disrupted": shots.iter().filter(|s| s.disrupted).count(),
        "n_spontaneous": shots.iter().filter(|s| s.disrupted && s.spontaneous).count(),
        "n_wall_contact": shots.iter().filter(|s| s.wall_contact).count(),
        "mechanisms": mechanisms,
    });

    // Curves: ~200 thresholds per lead time; reference points per budget.
    let step = (cands.len() / 200).max(1);
    let sub: Vec<f64> = cands.iter().copied().step_by(step).chain(std::iter::once(1.0)).collect();
    let mut curves = serde_json::Map::new();
    let mut reference = serde_json::Map::new();
    for &lead in &LEAD_TIMES {
        let pts: Vec<Value> = sub
            .iter()
            .map(|&t| {
                let mut p = evaluate(shots, rule, lead, t);
                json!({"threshold": t, "tpr": rate(p.tp, n_scored), "fpr": rate(p.fp, n_quiet),
                       "warning_median_s": median(&mut p.warnings), "caught": p.tp})
            })
            .collect();
        curves.insert(format!("{lead}"), Value::Array(pts));
        let mut per = serde_json::Map::new();
        for &b in &BUDGETS {
            let mut p = pick(shots, rule, lead, b, &cands);
            per.insert(format!("{b}"), json!({"threshold": p.threshold, "tpr": rate(p.tp, n_scored),
                "fpr": rate(p.fp, n_quiet), "warning_median_s": median(&mut p.warnings), "caught": p.tp}));
        }
        reference.insert(format!("{lead}"), Value::Object(per));
    }
    let curve = json!({"lead_times": LEAD_TIMES, "curves": curves, "reference": reference});
    (summary, curve)
}

/// Predicted vs true dominant mechanism over scored disruptions, and the mean
/// shares. `null` when the model sends no shares.
fn mechanism_summary(shots: &[ShotScore], rule: &Rule) -> Value {
    let mut n = 0usize;
    let mut agree = 0usize;
    let mut confusion = serde_json::Map::new();
    let mut mean_true = [0.0f64; 5];
    let mut mean_pred = [0.0f64; 5];
    for s in shots.iter().filter(|s| s.disrupted && !s.excluded(rule)) {
        let (Some(t), Some(p)) = (&s.true_cause, &s.predicted_cause) else { continue };
        let (Some(td), Some(pd)) = (t["dominant"].as_str(), p["dominant"].as_str()) else { continue };
        n += 1;
        agree += (td == pd) as usize;
        let row = confusion.entry(td.to_string()).or_insert_with(|| json!({}));
        let c = row[pd].as_u64().unwrap_or(0) + 1;
        row[pd] = json!(c);
        for (k, m) in crate::mech::MECH.iter().enumerate() {
            mean_true[k] += t["share_pct"][m].as_f64().unwrap_or(0.0);
            mean_pred[k] += p["share_pct"][m].as_f64().unwrap_or(0.0);
        }
    }
    if n == 0 {
        return Value::Null;
    }
    let avg = |a: [f64; 5]| -> Value {
        Value::Object(crate::mech::MECH.iter().zip(a).map(|(m, x)| (m.to_string(), json!(x / n as f64))).collect())
    };
    json!({"n": n, "agree": agree, "accuracy": agree as f64 / n as f64, "confusion": confusion,
           "mean_true_share_pct": avg(mean_true), "mean_predicted_share_pct": avg(mean_pred)})
}

/// Re-score a saved report under a different rule.
pub fn rescore(report: &mut Value, lead: Option<f64>, budget: Option<f64>, threshold: Option<f64>) -> Result<(), String> {
    let mut shots: Vec<ShotScore> = serde_json::from_value(report["shots"].clone()).map_err(|e| format!("shots: {e}"))?;
    let mut rule: Rule = serde_json::from_value(report["rule"].clone()).map_err(|e| format!("rule: {e}"))?;
    if let Some(l) = lead {
        rule.lead_time_s = l;
    }
    if let Some(b) = budget {
        rule.fpr_budget = b;
    }
    rule.threshold = threshold;
    let (summary, curve) = score(&mut shots, &rule);
    report["summary"] = summary;
    report["curve"] = curve;
    report["rule"] = serde_json::to_value(rule).unwrap();
    report["shots"] = serde_json::to_value(shots).unwrap();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(id: u64, disrupted: bool, tq: Option<f64>, stair: Vec<(f64, f64)>) -> ShotScore {
        ShotScore {
            stair_info: vec![], true_cause: None, predicted_cause: None, peak_cause: None, alarm_cause: None,
            shot_id: id, scenario: "x".into(), disrupted, t_tq: tq, t_end: Some(10.0), n_steps: 2000,
            spontaneous: false, wall_contact: false, t_alarm: None, warning_s: None, verdict: String::new(), staircase: stair,
        }
    }

    #[test]
    fn sweep_picks_lowest_threshold_within_budget() {
        let rule = Rule { lead_time_s: 0.1, fpr_budget: 0.0, exclude_spontaneous: true, exclude_wall_contact: true, threshold: None };
        let mut shots = vec![
            shot(0, true, Some(5.0), vec![(0.005, 0.0), (4.0, 0.9)]),
            shot(1, false, None, vec![(0.005, 0.0), (3.0, 0.5)]),
            shot(2, true, Some(5.0), vec![(0.005, 0.0), (4.95, 0.95)]),
        ];
        let (summary, _) = score(&mut shots, &rule);
        assert_eq!(summary["tp"], 1);
        assert_eq!(summary["fp"], 0);
        let thr = summary["threshold"].as_f64().unwrap();
        assert!(thr > 0.5 && thr < 0.9);
        assert_eq!(shots[0].verdict, "caught");
        assert_eq!(shots[1].verdict, "quiet");
        assert_eq!(shots[2].verdict, "late");
    }
}
