//! Disruption mechanisms: the engine's true split of λ, and a model's.
//!
//! λ = 1 − Π(1 − rₖᵖ) with p = 3 for the four limits and p = 1 for the
//! locked-mode term, so contributions Cₖ = −ln(1 − rₖᵖ) add up; the share of
//! mechanism k is Cₖ / ΣC — the probability that a disruption starting now
//! is due to k. Same definition as ImpactNet's `share_pct`.

use serde_json::{json, Value};
use tok_sym_core::simulation::SimulationSnapshot;

pub const MECH: [&str; 5] = ["greenwald", "q95", "radiation", "troyon", "other"];

/// True mechanism shares (%) from the engine's risk factors, plus λ and the
/// dominant mechanism.
pub fn true_split(s: &SimulationSnapshot) -> Value {
    let r = [
        s.risk_greenwald.powi(3),
        s.risk_q95.powi(3),
        s.risk_radiation.powi(3),
        s.risk_beta.powi(3),
        s.risk_locked_mode,
    ];
    let c: Vec<f64> = r.iter().map(|x| -(1.0 - x.min(1.0 - 1e-9)).ln()).collect();
    let sum: f64 = c.iter().sum::<f64>().max(1e-30);
    let mut shares = serde_json::Map::new();
    for (k, v) in MECH.iter().zip(&c) {
        shares.insert(k.to_string(), json!(100.0 * v / sum));
    }
    json!({"share_pct": shares, "risk_total": s.disruption_risk, "dominant": dominant(&Value::Object(shares.clone()))})
}

/// The mechanism with the largest share in a `share_pct` object.
pub fn dominant(share_pct: &Value) -> Option<String> {
    share_pct.as_object()?.iter()
        .filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(k, _)| k)
}

/// Keep only what a report needs from a model's step answer.
pub fn slim(info: &Value) -> Option<Value> {
    let sp = info.get("share_pct")?;
    Some(json!({"share_pct": sp, "risk_total": info.get("risk_total"), "dominant": dominant(sp)}))
}
