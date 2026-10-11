//! The shadow plasma: proof that an intervention prevented a disruption.
//!
//! At the first intervention the engine is copied — random-number state
//! included — and the copy runs on, in lock-step with the real plasma, but
//! without any of the corrections. Both see the same random draws, so if the
//! shadow disrupts and the real plasma does not, it is the same discharge with
//! the same luck: the disruption was prevented, not dodged by chance. The
//! accumulated risk of both (P = 1 − exp(−∫λ dt)) is kept too, for pulses where
//! the dice happened to spare the shadow.

use serde::Serialize;

use crate::simulation::{Simulation, SimulationSnapshot, SimulationStatus};

const MECH: [&str; 5] = ["greenwald", "q95", "radiation", "troyon", "other"];

pub struct Shadow {
    sim: Simulation,
    t0: f64,
    int_shadow: f64,
    int_real: f64,
    t_trigger: Option<f64>,
    cause: Option<&'static str>,
    lambda_at_trigger: f64,
    real_t_trigger: Option<f64>,
    finished: bool,
}

/// What the shadow says so far.
#[derive(Debug, Clone, Serialize)]
pub struct ShadowStatus {
    /// Time the shadow was split off (the first intervention), s
    pub t0: f64,
    pub time: f64,
    /// Shadow (no intervention): disruption trigger time and mechanism
    pub t_trigger: Option<f64>,
    pub cause: Option<&'static str>,
    pub lambda_at_trigger: f64,
    /// Probability of a disruption since t0 from the accumulated λ
    pub p_shadow: f64,
    pub p_real: f64,
    /// Real plasma's trigger time, if it disrupted after t0
    pub real_t_trigger: Option<f64>,
    /// The shadow pulse has ended (completed or disrupted)
    pub finished: bool,
}

fn dominant(s: &SimulationSnapshot) -> &'static str {
    let r = [s.risk_greenwald.powi(3), s.risk_q95.powi(3), s.risk_radiation.powi(3), s.risk_beta.powi(3), s.risk_locked_mode];
    let k = (0..5).max_by(|&a, &b| r[a].partial_cmp(&r[b]).unwrap()).unwrap();
    MECH[k]
}

impl Shadow {
    /// Split a shadow off the real plasma *before* the intervention is applied.
    pub fn start(real: &Simulation) -> Self {
        let mut sim = real.clone();
        sim.compute_geometry = false;
        sim.compute_channels = false;
        Shadow {
            sim, t0: real.time, int_shadow: 0.0, int_real: 0.0, t_trigger: None, cause: None,
            lambda_at_trigger: 0.0, real_t_trigger: None, finished: false,
        }
    }

    /// Advance in lock-step with the real plasma, which just produced `real`.
    pub fn step(&mut self, dt: f64, real: &SimulationSnapshot) {
        if self.real_t_trigger.is_none() {
            match real.t_trigger {
                Some(t) if t >= self.t0 => self.real_t_trigger = Some(t),
                _ if !real.disrupted => self.int_real += real.disruption_risk * dt,
                _ => {}
            }
        }
        if self.finished {
            return;
        }
        let s = self.sim.step(dt);
        if self.t_trigger.is_none() {
            if s.t_trigger.is_some() {
                self.t_trigger = s.t_trigger;
                self.cause = Some(dominant(&s));
                self.lambda_at_trigger = self.sim.risk_at_trigger;
            } else {
                self.int_shadow += s.disruption_risk * dt;
            }
        }
        if s.status != SimulationStatus::Running {
            self.finished = true;
        }
    }

    pub fn status(&self) -> ShadowStatus {
        ShadowStatus {
            t0: self.t0,
            time: self.sim.time,
            t_trigger: self.t_trigger,
            cause: self.cause,
            lambda_at_trigger: self.lambda_at_trigger,
            p_shadow: if self.t_trigger.is_some() { 1.0 } else { 1.0 - (-self.int_shadow).exp() },
            p_real: if self.real_t_trigger.is_some() { 1.0 } else { 1.0 - (-self.int_real).exp() },
            real_t_trigger: self.real_t_trigger,
            finished: self.finished,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices;
    use crate::generator::generate_shot;

    #[test]
    fn untouched_shadow_repeats_the_real_plasma() {
        // Without any intervention the shadow must live exactly the same pulse.
        let jet = devices::get_device("jet").unwrap();
        for shot in 0..8 {
            let spec = generate_shot(&jet, 3, shot);
            let mut sim = Simulation::new(jet.clone(), spec.program);
            sim.compute_geometry = false;
            sim.seed_disruption(spec.seed | 1);
            sim.start();
            for _ in 0..400 {
                sim.step(0.005);
            }
            let mut sh = Shadow::start(&sim);
            let mut real_trigger = None;
            for _ in 0..3000 {
                let s = sim.step(0.005);
                sh.step(0.005, &s);
                real_trigger = real_trigger.or(s.t_trigger);
                if s.status != SimulationStatus::Running {
                    break;
                }
            }
            assert_eq!(sh.status().t_trigger, real_trigger, "shot {shot}");
        }
    }
}
