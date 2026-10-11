//! The fixer: choose a correction by simulating each candidate ahead.
//!
//! At a decision the engine is copied once per candidate action; each copy
//! applies the action and runs `horizon` seconds with the dice switched off
//! (it integrates λ instead) and the programme held at its current value, so
//! it cannot see scheduled disturbances or the planned ramp-down. Each
//! candidate gets
//!
//! * `p` — probability of a disruption within the horizon, 1 − exp(−∫λ dt);
//! * `loss` — performance lost against the plasma with no corrections,
//!   max(0, 1 − ⟨W⟩/⟨W⟩_prog) + β·Σ|scaleᵢ − 1|. Gains are not rewarded (else
//!   "add heating" would win for its own sake), and the β term — a small price
//!   for every multiplier away from the programme — makes the fixer give the
//!   programme back once it is safe;
//! * `cost` = p + α·loss.
//!
//! The cheapest wins. Without the loss term "switch everything off" would
//! always win. The copy knows the true hidden state, so this is the upper
//! bound of what a controller can do — a teacher for a model that must
//! decide from the instruments alone.

use serde::{Deserialize, Serialize};

use crate::simulation::{ProgramScales, Simulation, SimulationStatus};

/// Candidate actions, in a fixed order (the column order of decision logs).
pub const ACTIONS: [&str; 8] = [
    "none",       // keep everything as it is
    "ne_-10",     // density target and D2 puff ×0.9   (Greenwald)
    "ne_-20",     // density target and D2 puff ×0.8   (Greenwald)
    "neon_off",   // stop neon seeding                 (radiation)
    "ip_-10",     // plasma current ×0.9, total ≥ 0.9  (kink)
    "heat_+20",   // NBI/ECH/ICRH ×1.2, total ≤ 1.5    (radiation)
    "heat_-20",   // NBI/ECH/ICRH ×0.8, total ≥ 0.5    (Troyon)
    "relax",      // every multiplier halfway back to 1 (give performance back)
];

/// Scales after applying action `a` to `s`.
pub fn apply_action(a: usize, s: &ProgramScales) -> ProgramScales {
    let mut n = *s;
    match ACTIONS[a] {
        "ne_-10" => {
            n.ne_target = (n.ne_target * 0.9).max(0.4);
            n.d2_puff = (n.d2_puff * 0.9).max(0.0);
        }
        "ne_-20" => {
            n.ne_target = (n.ne_target * 0.8).max(0.4);
            n.d2_puff = (n.d2_puff * 0.8).max(0.0);
        }
        "neon_off" => n.neon_puff = 0.0,
        // Below 0.9 of the programmed current the alarm models take the drop
        // for a programmed ramp-down and go quiet; never go further.
        "ip_-10" => n.ip = (n.ip * 0.9).max(0.9),
        "heat_+20" => n.heating = (n.heating * 1.2).min(1.5),
        "heat_-20" => n.heating = (n.heating * 0.8).max(0.5),
        "relax" => {
            let back = |x: f64| x + 0.5 * (1.0 - x);
            n = ProgramScales {
                ip: back(n.ip),
                ne_target: back(n.ne_target),
                d2_puff: back(n.d2_puff),
                neon_puff: back(n.neon_puff),
                heating: back(n.heating),
            };
        }
        _ => {}
    }
    n
}

/// What one look-ahead copy saw.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollout {
    /// Probability of a disruption within the horizon (1 − e^{−∫λ dt}).
    pub p: f64,
    /// Mean stored energy over the horizon, MJ (performance).
    pub w_mean: f64,
    /// Fraction of the horizon spent in H-mode.
    pub hmode: f64,
    /// Peak λ over the horizon, 1/s.
    pub lambda_max: f64,
}

/// Run a copy of `sim` with `scales` for `horizon` seconds at step `dt`.
pub fn rollout(sim: &Simulation, scales: ProgramScales, horizon: f64, dt: f64) -> Rollout {
    let mut c = sim.clone();
    c.compute_geometry = false;
    c.compute_channels = false;
    c.stochastic = false;
    c.freeze_program_at = Some(sim.time);
    c.scales = scales;
    let n = (horizon / dt).round().max(1.0) as usize;
    let (mut integral, mut w, mut h, mut lmax) = (0.0, 0.0, 0.0, 0.0f64);
    let mut steps = 0usize;
    for _ in 0..n {
        let s = c.step(dt);
        if s.status != SimulationStatus::Running {
            break;
        }
        steps += 1;
        // A forced disruption (wall contact) in the copy is certain.
        if s.disrupted {
            integral = f64::INFINITY;
            break;
        }
        integral += s.disruption_risk * dt;
        lmax = lmax.max(s.disruption_risk);
        w += s.w_th;
        h += s.in_hmode as u8 as f64;
    }
    let k = steps.max(1) as f64;
    Rollout { p: 1.0 - (-integral).exp(), w_mean: w / k, hmode: h / k, lambda_max: lmax }
}

/// One candidate's verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub action: String,
    pub scales: ProgramScales,
    pub p: f64,
    pub loss: f64,
    pub cost: f64,
    pub w_mean: f64,
    pub hmode: f64,
    pub lambda_max: f64,
}

/// A fixer decision: every candidate and the chosen one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub t: f64,
    pub variants: Vec<Variant>,
    pub best: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FixerConfig {
    /// Look-ahead, s
    pub horizon: f64,
    /// Step inside the copies, s (coarser than the 5 ms engine step = faster)
    pub dt: f64,
    /// Weight of performance loss against disruption probability
    pub alpha: f64,
}

/// Price of each multiplier's distance from the programme (inside `loss`).
pub const BETA: f64 = 0.02;

fn deviation(s: &ProgramScales) -> f64 {
    (s.ip - 1.0).abs() + (s.ne_target - 1.0).abs() + (s.d2_puff - 1.0).abs() + (s.neon_puff - 1.0).abs() + (s.heating - 1.0).abs()
}

impl Default for FixerConfig {
    fn default() -> Self {
        FixerConfig { horizon: 1.0, dt: 0.02, alpha: 0.5 }
    }
}

/// An incremental decision: one candidate per `step()`, so a real-time loop
/// (the browser) can spread the work over frames. The engine state is copied
/// once, at `new()`; the plasma keeps running meanwhile, and the decision is
/// applied when it is ready — like a real controller's latency.
pub struct Planner {
    base: Simulation,
    cfg: FixerConfig,
    rolls: Vec<(ProgramScales, Rollout)>,
    w_ref: Option<f64>,
}

impl Planner {
    pub fn new(sim: &Simulation, cfg: FixerConfig) -> Self {
        let mut base = sim.clone();
        base.compute_geometry = false;
        base.compute_channels = false;
        Planner { base, cfg, rolls: Vec::with_capacity(ACTIONS.len()), w_ref: None }
    }

    /// Candidates evaluated so far, out of `total()`.
    pub fn done(&self) -> usize {
        self.rolls.len() + self.w_ref.is_some() as usize
    }

    pub fn total(&self) -> usize {
        ACTIONS.len() + 1
    }

    /// Evaluate the next candidate. Returns true when the decision is ready.
    pub fn step(&mut self) -> bool {
        if self.w_ref.is_none() {
            // Reference: the plasma as programmed (no corrections at all).
            self.w_ref = Some(rollout(&self.base, ProgramScales::default(), self.cfg.horizon, self.cfg.dt).w_mean);
        } else if self.rolls.len() < ACTIONS.len() {
            let a = self.rolls.len();
            let sc = apply_action(a, &self.base.scales);
            self.rolls.push((sc, rollout(&self.base, sc, self.cfg.horizon, self.cfg.dt)));
        }
        self.done() >= self.total()
    }

    /// The decision (call when `step()` returned true).
    pub fn finish(&self) -> Decision {
        let w_ref = self.w_ref.unwrap_or(0.0).max(1e-6);
        let variants: Vec<Variant> = self
            .rolls
            .iter()
            .zip(ACTIONS)
            .map(|((sc, r), name)| {
                let loss = (1.0 - r.w_mean / w_ref).max(0.0) + BETA * deviation(sc);
                Variant {
                    action: name.to_string(),
                    scales: *sc,
                    p: r.p,
                    loss,
                    cost: r.p + self.cfg.alpha * loss,
                    w_mean: r.w_mean,
                    hmode: r.hmode,
                    lambda_max: r.lambda_max,
                }
            })
            .collect();
        // Ties (e.g. no risk anywhere) go to the earlier action, i.e. "none".
        let mut best = 0;
        for (i, v) in variants.iter().enumerate() {
            if v.cost < variants[best].cost - 1e-9 {
                best = i;
            }
        }
        Decision { t: self.base.time, variants, best }
    }
}

/// Evaluate every candidate action on copies of `sim` and pick the cheapest.
pub fn decide(sim: &Simulation, cfg: &FixerConfig) -> Decision {
    let mut p = Planner::new(sim, *cfg);
    while !p.step() {}
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices;
    use crate::generator::generate_shot;

    #[test]
    fn copies_do_not_disturb_the_original() {
        let jet = devices::get_device("jet").unwrap();
        let spec = generate_shot(&jet, 1, 0);
        let mut a = Simulation::new(jet.clone(), spec.program.clone());
        a.compute_geometry = false;
        a.seed_disruption(7);
        a.start();
        let mut b = a.clone();
        for _ in 0..800 {
            a.step(0.005);
        }
        let _ = decide(&a, &FixerConfig { horizon: 0.3, ..Default::default() });
        for _ in 0..800 {
            b.step(0.005);
        }
        let (sa, sb) = (a.step(0.005), b.step(0.005));
        assert_eq!(sa.w_th, sb.w_th);
        assert_eq!(sa.disrupted, sb.disrupted);
    }

    #[test]
    fn cutting_density_lowers_greenwald_risk() {
        let jet = devices::get_device("jet").unwrap();
        let spec = generate_shot(&jet, 1, 0);
        let mut sim = Simulation::new(jet, spec.program);
        sim.compute_geometry = false;
        sim.stochastic = false;
        sim.start();
        for _ in 0..1600 {
            sim.step(0.005);
        }
        // Force a dangerous density and check the fixer's ranking agrees with physics.
        sim.scales.ne_target = 1.8;
        for _ in 0..200 {
            sim.step(0.005);
        }
        let d = decide(&sim, &FixerConfig::default());
        assert!(d.variants[2].p <= d.variants[0].p, "ne_-20 should not raise the risk");
    }
}
