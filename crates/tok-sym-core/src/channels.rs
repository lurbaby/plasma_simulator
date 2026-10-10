//! The hundred measured channels of the hackathon's toksym build, noise-free.
//!
//! Channel names and order match toksym's `measured.csv` exactly, so a model
//! trained on toksym data can be fed from this engine. The formulas were
//! recovered by fitting toksym output generated with an all-zero noise config
//! against its own truth columns (see `HACKATHON.md`). Most channels are exact;
//! shape-dependent ones (flux loops, PF coils, divertor probes, vessel
//! pressure) are power-law fits and agree to within a few percent (probes and
//! pressure to ~30 %).

use crate::simulation::SimulationSnapshot;

/// Channel names, in toksym order.
pub const CHANNEL_NAMES: [&str; 100] = [
    "mag_ip_MA", "mag_bt_T", "mag_diamag_MJ", "mag_vloop_V",
    "mag_mirnov_01", "mag_mirnov_02", "mag_mirnov_03", "mag_mirnov_04",
    "mag_mirnov_05", "mag_mirnov_06", "mag_mirnov_07", "mag_mirnov_08",
    "mag_saddle_lm",
    "mag_flux_loop_in", "mag_flux_loop_top", "mag_flux_loop_out", "mag_flux_loop_bot",
    "mag_halo_upper_kA", "mag_halo_lower_kA",
    "coil_pf_shape_kA", "coil_pf_vert_kA", "coil_pf_radial_kA",
    "coil_tf_current_kA", "coil_tf_voltage_V", "coil_cs_flux_Vs",
    "ifr_chord_core", "ifr_chord_mid_in", "ifr_chord_mid_out", "ifr_chord_edge", "ifr_chord_far_edge",
    "ece_01", "ece_02", "ece_03", "ece_04", "ece_05", "ece_06", "ece_07", "ece_08",
    "ece_09", "ece_10", "ece_11", "ece_12", "ece_13", "ece_14", "ece_15", "ece_16",
    "bolo_01", "bolo_02", "bolo_03", "bolo_04", "bolo_05", "bolo_06", "bolo_07", "bolo_08",
    "sxr_01", "sxr_02", "sxr_03", "sxr_04", "sxr_05", "sxr_06", "sxr_07", "sxr_08",
    "neutron_total", "neutron_collimated",
    "div_probe_isat_in", "div_probe_isat_out", "div_probe_isat_far",
    "div_tc_inner_C", "div_tc_outer_C", "div_tc_baffle_C", "div_tc_dome_C",
    "wall_tc_inboard_C", "wall_tc_outboard_C",
    "dalpha_div_inner", "dalpha_div_outer", "dalpha_midplane", "dalpha_xpoint",
    "gas_valve_d2_cmd", "gas_valve_ne_cmd", "gas_flow_d2_meas", "gas_flow_ne_meas",
    "vac_pressure_mbar", "vac_rga_mass2", "vac_rga_mass20", "cryo_pressure_mbar",
    "coil_tf_temp_K", "psu_dclink_V",
    "nbi_power_MW", "ich_power_MW", "ech_power_MW", "nbi_beam_energy_keV",
    "nbi_source_current_A", "nbi_source_2_current_A", "ich_antenna_voltage_kV",
    "prog_ip_MA", "prog_bt_T", "prog_ne_1e20", "prog_p_nbi_MW", "prog_p_ech_MW", "prog_p_ich_MW",
];

pub const N_CHANNELS: usize = CHANNEL_NAMES.len();

/// Interferometer chords: (impact parameter in ρ, scale relative to a·κ).
const IFR_CHORDS: [(f64, f64); 5] = [
    (0.16, 1.0231), (0.41, 1.0610), (0.41, 1.0610), (0.71, 1.0638), (0.87, 1.0707),
];
/// Soft X-ray chords 1..4 (5..8 mirror them): (impact parameter, scale / a·κ).
const SXR_CHORDS: [(f64, f64); 4] = [(0.71, 10.453), (0.50, 10.095), (0.31, 10.272), (0.06, 9.926)];
/// Bolometer chords 1..4 (5..8 mirror them), impact parameters.
const BOLO_CHORDS: [f64; 4] = [0.875, 0.625, 0.375, 0.125];

/// Stateful channel synthesiser: some instruments integrate over the pulse
/// (central-solenoid flux, thermocouples).
#[derive(Debug, Clone)]
pub struct Channels {
    cs_volt_seconds: f64,
    thermocouples: [f64; 6],
    halo_ip: Option<f64>,
    t_trigger: Option<f64>,
}

impl Default for Channels {
    fn default() -> Self {
        Self::new()
    }
}

/// Linear interpolation of a profile sampled uniformly on ρ ∈ [0, 1].
fn profile_at(profile: &[f64], rho: f64) -> f64 {
    if profile.is_empty() {
        return 0.0;
    }
    let n = profile.len() - 1;
    let x = (rho.clamp(0.0, 1.0)) * n as f64;
    let i = (x.floor() as usize).min(n.saturating_sub(1));
    let f = x - i as f64;
    if n == 0 {
        return profile[0];
    }
    profile[i] * (1.0 - f) + profile[i + 1] * f
}

/// ∫ f(ρ) ds along a straight chord with impact parameter `p` through a
/// unit circle (ρ = sqrt(p² + s²)).
fn chord_integral(f: &dyn Fn(f64) -> f64, p: f64) -> f64 {
    const N: usize = 60;
    let smax = (1.0 - p * p).max(0.0).sqrt();
    if smax == 0.0 {
        return 0.0;
    }
    let ds = 2.0 * smax / N as f64;
    let mut sum = 0.0;
    for i in 0..=N {
        let s = -smax + i as f64 * ds;
        let w = if i == 0 || i == N { 0.5 } else { 1.0 };
        sum += w * f((p * p + s * s).sqrt());
    }
    sum * ds
}

impl Channels {
    pub fn new() -> Self {
        Channels {
            cs_volt_seconds: 0.0,
            thermocouples: [120.0, 160.0, 200.0, 240.0, 280.0, 320.0],
            halo_ip: None,
            t_trigger: None,
        }
    }

    /// Compute all channels for one step. `minor_radius` is the device's a (m).
    pub fn step(&mut self, s: &SimulationSnapshot, dt: f64, minor_radius: f64) -> [f64; N_CHANNELS] {
        let mut v = [0.0; N_CHANNELS];
        let ip = s.ip.max(0.0);
        let w = s.w_th.max(0.0);
        let ne = s.ne_bar.max(0.0);
        let t = s.time;
        let v_loop = if ip > 1e-3 { s.p_ohmic / ip } else { 0.0 };
        let psol = (s.p_input - s.p_rad).max(0.05);
        let li = s.li.max(0.1);
        let kappa_a = s.prog_kappa.max(1.0) * minor_radius;

        // Disruption bookkeeping: the precursor clock and the halo current
        // are fixed at the trigger.
        if s.disrupted && self.t_trigger.is_none() {
            self.t_trigger = Some(t);
            self.halo_ip = Some(ip);
        }
        let lm = s.locked_mode_amp.max(0.0);

        v[0] = ip;
        v[1] = s.bt;
        v[2] = w;
        v[3] = v_loop;
        // Mirnov coils: rotating precursor mode, or an ELM burst.
        for k in 0..8 {
            let kf = k as f64;
            v[4 + k] = if let (Some(t0), true) = (self.t_trigger, lm > 0.0) {
                lm * (50.0 * (t - t0) + 0.316 + kf * std::f64::consts::FRAC_PI_4).sin()
            } else if s.elm_active {
                let amp = if s.elm_type == 2 { 0.075 } else { 0.16 };
                amp * (kf * 3.0 * std::f64::consts::FRAC_PI_4).sin()
            } else {
                0.0
            };
        }
        v[12] = lm;
        let wpos = w.max(1e-3);
        v[13] = 0.5254 * ip.powf(0.795) * wpos.powf(0.102);
        v[14] = 0.4369 * ip.powf(1.034);
        v[15] = 0.3165 * ip.powf(1.277) * wpos.powf(-0.136).min(10.0);
        v[16] = 0.4030 * ip.powf(0.962);
        let halo = self.halo_ip.unwrap_or(0.0);
        v[17] = 0.35 * halo;
        v[18] = 0.245 * halo;
        v[19] = 4.661 * ip.powf(0.485) * wpos.powf(0.138) * li.powf(0.215);
        v[20] = 1.1357 * ip.powf(1.482) * wpos.powf(0.14) * li.powf(0.256) * s.bt.max(0.1).powf(-0.297);
        v[21] = 1.793 * ip;
        v[22] = 18.2 * s.bt;
        v[23] = 0.90816 * s.bt;
        self.cs_volt_seconds += v_loop * dt;
        v[24] = -(1.7733 * ip + 0.4086 * self.cs_volt_seconds);

        let ne_p = &s.ne_profile;
        let te_p = &s.te_profile;
        let ne_at = |r: f64| profile_at(ne_p, r);
        for (i, &(p, k)) in IFR_CHORDS.iter().enumerate() {
            v[25 + i] = k * kappa_a * chord_integral(&ne_at, p);
        }
        for i in 0..16 {
            v[30 + i] = profile_at(te_p, 0.064 * i as f64);
        }
        // Bolometers share P_rad by chord-integrated n²/√T emissivity.
        let bolo_eps = |r: f64| {
            let n = profile_at(ne_p, r);
            n * n / profile_at(te_p, r).max(1e-3).sqrt()
        };
        let bolo_raw: Vec<f64> = BOLO_CHORDS.iter().map(|&p| chord_integral(&bolo_eps, p)).collect();
        let bolo_sum: f64 = 2.0 * bolo_raw.iter().sum::<f64>();
        for i in 0..4 {
            let share = if bolo_sum > 0.0 { bolo_raw[i] / bolo_sum } else { 0.0 };
            v[46 + i] = s.p_rad * share;
            v[53 - i] = s.p_rad * share;
        }
        let sxr_eps = |r: f64| {
            let n = profile_at(ne_p, r);
            n * n * profile_at(te_p, r).max(1e-3).sqrt()
        };
        for (i, &(p, k)) in SXR_CHORDS.iter().enumerate() {
            let x = k * kappa_a * chord_integral(&sxr_eps, p);
            v[54 + i] = x;
            v[61 - i] = x;
        }
        let n_total = (0.01 * ne * ne * s.te0 * s.te0).max(1e-10);
        v[62] = n_total;
        v[63] = 0.28 * n_total;
        let isat = 0.02463 * ne.max(0.01).powf(0.44) * psol.powf(0.581) * ip.max(1e-3).powf(0.047);
        v[64] = isat;
        v[65] = 0.75 * isat;
        v[66] = 0.5625 * isat;
        // Thermocouples relax towards a level set by the power crossing the
        // separatrix (τ ≈ 0.31 s); gains fall off ×0.85 away from the strike point.
        let alpha = 1.0 - (-dt / 0.31).exp();
        for k in 0..6 {
            let target = 120.0 + 40.0 * k as f64 + 16.76 * 0.85f64.powi(k as i32) * psol * (ip > 0.05) as u8 as f64;
            self.thermocouples[k] += (target - self.thermocouples[k]) * alpha;
            v[67 + k] = self.thermocouples[k];
        }
        let elm_mult = if s.elm_active {
            if s.elm_type == 2 { 2.5 } else { 8.0 }
        } else {
            1.0
        };
        let ne_da = ne.max(0.01);
        v[73] = 0.5 * ne_da * elm_mult;
        v[74] = 0.5 * ne_da * elm_mult;
        v[75] = 0.175 * ne_da * elm_mult;
        v[76] = 0.5 * ne_da * elm_mult;
        v[77] = s.d2_puff;
        v[78] = s.neon_puff;
        v[79] = 0.97 * s.d2_puff;
        v[80] = 0.95 * s.neon_puff;
        v[81] = 1.3e-5 * s.d2_puff.max(0.0).powf(0.853) * ne.max(0.01).powf(-0.074) * psol.powf(0.175);
        v[82] = 8.0e-6 * s.d2_puff;
        v[83] = 2.0e-6 * s.neon_puff;
        v[84] = 9.0e-7 + 2.0e-8 * t;
        v[85] = 4.5 + 0.02 * t;
        v[86] = 3300.0 - 4.0 * ip;
        v[87] = s.prog_p_nbi;
        v[88] = s.prog_p_ich;
        v[89] = s.prog_p_ech;
        v[90] = 100.0;
        v[91] = 0.28 * s.prog_p_nbi;
        v[92] = 0.26 * s.prog_p_nbi;
        v[93] = 18.0 * s.prog_p_ich.max(0.0).sqrt();
        v[94] = s.prog_ip;
        v[95] = s.prog_bt;
        v[96] = s.prog_ne;
        v[97] = s.prog_p_nbi;
        v[98] = s.prog_p_ech;
        v[99] = s.prog_p_ich;
        v
    }
}
