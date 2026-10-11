//! Discharge generator reproducing the hackathon's toksym dataset family.
//!
//! Every shot is JET, DD fuel, the standard H-mode program time-scaled to a
//! duration of 12–20 s, with ten plateau levels drawn from fixed ranges
//! (the ranges toksym shows in its Dataset window). Current and density are
//! capped so no discharge starts past a stability limit (q95 ≥ 3.2,
//! f_GW ≤ 0.72). Mid-pulse disturbances — density, neon, low_q, heat_loss —
//! start at a random moment in the flat-top; a fifth of runs stay quiet, a
//! sixth get two. Distributions were estimated from 200 toksym shots
//! (see `HACKATHON.md`); the exact random streams differ from toksym's.

use serde::{Deserialize, Serialize};

use crate::devices::Device;
use crate::simulation::{PulseProgram, WaveformPoint};

/// Plateau parameter ranges (min, max), as shown in toksym's Dataset window.
pub const RANGES: [(&str, f64, f64); 10] = [
    ("bt", 2.0, 3.4),
    ("ip", 1.8, 4.0),
    ("ne", 0.3, 1.5),
    ("p_nbi", 4.0, 34.0),
    ("p_ech", 0.0, 6.0),
    ("p_ich", 0.0, 9.0),
    ("d2_puff", 0.3, 7.0),
    ("neon_puff", 0.0, 0.7),
    ("kappa", 1.55, 1.68),
    ("delta", 0.2, 0.4),
];

pub const DISTURBANCES: [&str; 4] = ["density", "neon", "low_q", "heat_loss"];

/// One generated discharge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotSpec {
    pub shot_id: u64,
    pub seed: u64,
    pub scenario: String,
    pub duration: f64,
    pub program: PulseProgram,
    /// The same programme without the unplanned disturbances: what the
    /// control system knows in advance.
    pub plan: PulseProgram,
}

/// splitmix64 — small, seedable, good enough for scenario draws.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
}

/// Per-shot seed derived from the master seed, so shot `i` of a set is the
/// same whatever the set size.
pub fn shot_seed(master_seed: u64, shot_id: u64) -> u64 {
    let mut r = Rng::new(master_seed ^ 0xA076_1D64_78BD_642F);
    for _ in 0..=shot_id % 7 {
        r.next_u64();
    }
    Rng::new(r.next_u64() ^ shot_id.wrapping_mul(0xE703_7ED1_A0B4_28DB)).next_u64() | 1
}

fn q95_shape_factor(kappa: f64, delta: f64) -> f64 {
    (1.0 + kappa * kappa * (1.0 + 2.0 * delta * delta - 1.2 * delta.powi(3))) / 2.0
}

/// Insert a disturbance into a flat-top waveform: from `t0` the value ramps
/// to `target` over `ramp` s and either stays there until the flat-top ends
/// or comes back to the plateau after `hold` s.
fn disturb(
    wave: &mut Vec<WaveformPoint>,
    flat_end: f64,
    t0: f64,
    ramp: f64,
    target: f64,
    ret: Option<(f64, f64)>,
    hold_to_end: bool,
) {
    let base = PulseProgram::interpolate(wave, t0);
    let t1 = (t0 + ramp).min(flat_end - 0.02);
    let mut pts: Vec<WaveformPoint> = wave.iter().copied().filter(|p| p.0 < t0).collect();
    pts.push((t0, base));
    pts.push((t1, target));
    let mut level = target;
    if let Some((hold, back)) = ret {
        let (t2, t3) = (t1 + hold, t1 + hold + back);
        if t3 < flat_end - 0.02 {
            pts.push((t2, target));
            pts.push((t3, base));
            level = base;
        }
    }
    let last = pts.last().unwrap().0;
    let scale = if base.abs() > 1e-9 { level / base } else { 1.0 };
    for &(t, v) in wave.iter().filter(|p| p.0 > last) {
        if hold_to_end {
            // Gas valves keep the new setting until the pulse ends.
            pts.push((t, level));
        } else if t <= flat_end + 1e-9 {
            pts.push((t, level));
        } else {
            // Ramp-down follows the programmed shape from the new level.
            pts.push((t, v * scale));
        }
    }
    *wave = pts;
}

/// Draw shot `shot_id` of the set seeded by `master_seed`.
pub fn generate_shot(device: &Device, master_seed: u64, shot_id: u64) -> ShotSpec {
    let seed = shot_seed(master_seed, shot_id);
    let mut r = Rng::new(seed);
    let duration = (r.range(12.0, 20.0) / 0.005).round() * 0.005;
    let s = duration / 20.0;

    let bt = r.range(2.0, 3.4);
    let kappa = r.range(1.55, 1.68);
    let delta = r.range(0.2, 0.4);
    let a = device.a;
    let ip_cap = 5.0 * a * a * bt * q95_shape_factor(kappa, delta) / (device.r0 * 3.2);
    let ip = r.range(1.8, 4.0).min(ip_cap);
    let n_gw = device.greenwald_density(ip);
    let ne = r.range(0.3, 1.5).min(0.72 * n_gw);
    let p_nbi = r.range(4.0, 34.0);
    let p_ech = r.range(0.0, 6.0);
    let p_ich = r.range(0.0, 9.0);
    let d2 = r.range(0.3, 7.0);
    let neon = r.range(0.0, 0.7);

    let mut prog = PulseProgram {
        ip: vec![(0.0, 0.0), (1.0 * s, 0.3 * ip), (3.0 * s, ip), (16.0 * s, ip), (18.0 * s, 0.5 * ip), (duration, 0.0)],
        bt: vec![(0.0, bt), (duration, bt)],
        ne_target: vec![(0.0, 0.05), (2.0 * s, 0.5 * ne), (4.0 * s, ne), (16.0 * s, ne), (18.0 * s, 0.3 * ne), (duration, 0.05)],
        p_nbi: vec![(0.0, 0.0), (4.0 * s, 0.0), (5.0 * s, p_nbi), (15.0 * s, p_nbi), (16.0 * s, 0.0), (duration, 0.0)],
        p_ech: vec![(0.0, 0.0), (6.0 * s, 0.0), (7.0 * s, p_ech), (14.0 * s, p_ech), (15.0 * s, 0.0), (duration, 0.0)],
        p_ich: vec![(0.0, 0.0), (3.6 * s, 0.0), (4.0 * s, p_ich), (15.0 * s, p_ich), (16.0 * s, 0.0), (duration, 0.0)],
        kappa: vec![(0.0, 1.0), (3.0 * s, kappa), (16.0 * s, kappa), (duration, 1.0)],
        delta: vec![(0.0, 0.0), (3.0 * s, delta), (16.0 * s, delta), (duration, 0.0)],
        d2_puff: vec![(0.0, 0.0), (0.6 * s, d2), (4.0 * s, 0.5 * d2), (15.0 * s, 0.5 * d2), (18.0 * s, 0.0), (duration, 0.0)],
        neon_puff: vec![(0.0, 0.0), (6.0 * s, 0.0), (7.0 * s, neon), (14.0 * s, neon), (15.0 * s, 0.0), (duration, 0.0)],
        duration,
        config_override: None,
    };

    let plan = prog.clone();

    // How many disturbances: 1/5 none, 1/6 two, the rest one.
    let u = r.f();
    let n_dist = if u < 0.2 { 0 } else if u < 0.2 + 1.0 / 6.0 { 2 } else { 1 };
    let mut kinds: Vec<&str> = Vec::new();
    while kinds.len() < n_dist {
        let w = r.f();
        let k = if w < 0.32 { "heat_loss" } else if w < 0.61 { "neon" } else if w < 0.83 { "low_q" } else { "density" };
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }

    for &k in &kinds {
        let t0 = s * (3.0 + 13.0 * r.range(0.24, 0.70));
        let ret = if r.f() < 0.2 { Some((r.range(0.5, 2.5), r.range(0.3, 1.5))) } else { None };
        match k {
            "density" => {
                let target = r.range(0.6, 1.7) * n_gw;
                let ramp = r.range(0.5, 2.9);
                disturb(&mut prog.ne_target, 16.0 * s, t0.max(4.0 * s), ramp, target, ret, true);
            }
            "neon" => {
                let target = r.range(0.3, 2.2);
                let ramp = r.range(0.05, 2.9);
                let t0 = t0.clamp(7.0 * s, 13.5 * s);
                disturb(&mut prog.neon_puff, 14.0 * s, t0, ramp, target, ret, true);
            }
            "low_q" => {
                let q_target = r.range(2.0, 3.1);
                let ip_t = (5.0 * a * a * bt * q95_shape_factor(kappa, delta) / (device.r0 * q_target)).min(5.0).max(ip);
                let ramp = r.range(0.4, 2.8);
                disturb(&mut prog.ip, 16.0 * s, t0, ramp, ip_t, ret, false);
            }
            _ => {
                let target = p_nbi * r.range(0.025, 0.47);
                let ramp = r.range(0.02, 0.3);
                let t0 = t0.clamp(5.0 * s, 14.5 * s);
                disturb(&mut prog.p_nbi, 15.0 * s, t0, ramp, target, ret, false);
            }
        }
    }

    let scenario = if kinds.is_empty() { "none".to_string() } else { kinds.join("+") };
    ShotSpec { shot_id, seed, scenario, duration, program: prog, plan }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices;

    #[test]
    fn shots_are_reproducible_and_capped() {
        let jet = devices::get_device("jet").unwrap();
        for i in 0..50 {
            let a = generate_shot(&jet, 1, i);
            let b = generate_shot(&jet, 1, i);
            assert_eq!(a.scenario, b.scenario);
            assert!((12.0..=20.0).contains(&a.duration));
            let ip = PulseProgram::interpolate(&a.program.ip, 3.0 * a.duration / 20.0);
            let ne = PulseProgram::interpolate(&a.program.ne_target, 4.0 * a.duration / 20.0);
            assert!(ne <= 0.72 * jet.greenwald_density(ip) + 1e-9);
        }
    }
}
