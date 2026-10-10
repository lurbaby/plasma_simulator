//! Running one generated discharge headless, and writing datasets.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use tok_sym_core::channels::CHANNEL_NAMES;
use tok_sym_core::devices::{self, Device};
use tok_sym_core::generator::{generate_shot, ShotSpec};
use tok_sym_core::simulation::{Simulation, SimulationSnapshot, SimulationStatus};

pub const DT: f64 = 0.005;

/// Disruptions whose intensity at the trigger was this close to the floor
/// (≈0.006 /s) count as spontaneous: nobody can predict them.
pub const SPONTANEOUS_RISK: f64 = 0.02;

/// How a shot ended.
#[derive(Debug, Clone, Default)]
pub struct ShotOutcome {
    pub n_steps: usize,
    pub t_end: f64,
    pub disrupted: bool,
    pub t_trigger: Option<f64>,
    pub t_tq: Option<f64>,
    pub spontaneous: bool,
    pub wall_contact: bool,
    /// The engine's split of λ into mechanisms at the trigger (see `mech.rs`).
    pub true_cause: Option<serde_json::Value>,
}

pub fn jet() -> Device {
    devices::get_device("jet").expect("JET is built in")
}

fn scramble(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Run a discharge step by step. `on_step` sees every recorded step (the
/// thermal-quench step itself is not recorded, as in toksym) and may return
/// `false` to stop early.
pub fn run_shot(
    device: &Device,
    spec: &ShotSpec,
    mut on_step: impl FnMut(&SimulationSnapshot) -> bool,
) -> ShotOutcome {
    let mut sim = Simulation::new(device.clone(), spec.program.clone());
    sim.compute_geometry = false;
    sim.seed_disruption(scramble(spec.seed));
    sim.start();
    let mut out = ShotOutcome::default();
    let n_max = (spec.duration / DT).round() as usize;
    for _ in 0..n_max {
        let snap = sim.step(DT);
        if out.true_cause.is_none() && snap.t_trigger.is_some() {
            out.true_cause = Some(crate::mech::true_split(&snap));
        }
        if snap.t_tq.is_some() || snap.status == SimulationStatus::Complete {
            break;
        }
        out.n_steps += 1;
        out.t_end = snap.time;
        if !on_step(&snap) || snap.status != SimulationStatus::Running {
            break;
        }
    }
    out.disrupted = sim.t_trigger.is_some();
    out.t_trigger = sim.t_trigger;
    out.t_tq = sim.t_tq;
    out.wall_contact = sim.wall_contact;
    out.spontaneous = out.disrupted && !sim.wall_contact && sim.risk_at_trigger < SPONTANEOUS_RISK;
    out
}

/// Optional truth column groups for datasets (toksym's plus two of ours).
pub const EXTRA_GROUPS: [&str; 8] =
    ["program", "transport", "pedestal", "impurity", "equilibrium", "profiles", "physics", "risk"];

pub fn extra_header(groups: &[String]) -> Vec<String> {
    let mut h: Vec<String> = Vec::new();
    for g in EXTRA_GROUPS.iter().filter(|g| groups.iter().any(|x| x == *g)) {
        match *g {
            "program" => h.extend(["prog_ip_MA", "prog_bt_T", "prog_ne_1e20", "prog_p_nbi_MW", "prog_p_ech_MW", "prog_p_ich_MW", "prog_kappa", "prog_delta"].map(String::from)),
            "transport" => h.extend(["ip_MA", "bt_T", "te0_keV", "ne_bar_1e20", "w_th_MJ", "tau_e_s", "p_input_MW", "p_ohmic_MW", "p_rad_MW", "p_loss_MW", "li"].map(String::from)),
            "pedestal" => h.extend(["in_hmode", "elm_active", "elm_type", "elm_suppressed", "ne_ped_1e20", "te_ped_keV", "ne_line"].map(String::from)),
            "impurity" => h.extend(["impurity_fraction", "d2_puff", "neon_puff"].map(String::from)),
            "equilibrium" => h.extend(["axis_r_m", "axis_z_m", "xpoint_r_m", "xpoint_z_m", "is_limited"].map(String::from)),
            "profiles" => {
                h.extend((0..51).map(|i| format!("te_r{i:02}_keV")));
                h.extend((0..51).map(|i| format!("ne_r{i:02}_1e20")));
            }
            "physics" => h.extend(["q95", "beta_n", "f_greenwald", "p_rad_frac"].map(String::from)),
            _ => h.extend(["disruption_risk", "risk_greenwald", "risk_beta", "risk_q95", "risk_radiation", "risk_locked_mode"].map(String::from)),
        }
    }
    h
}

fn num(buf: &mut String, x: f64) {
    if x == 0.0 {
        buf.push_str(",0");
    } else {
        let _ = write!(buf, ",{}", format!("{x:.6e}").replace("e0", ""));
    }
}

fn flag(buf: &mut String, b: bool) {
    buf.push_str(if b { ",true" } else { ",false" });
}

pub fn extra_cells(buf: &mut String, s: &SimulationSnapshot, groups: &[String]) {
    for g in EXTRA_GROUPS.iter().filter(|g| groups.iter().any(|x| x == *g)) {
        match *g {
            "program" => for x in [s.prog_ip, s.prog_bt, s.prog_ne, s.prog_p_nbi, s.prog_p_ech, s.prog_p_ich, s.prog_kappa, s.prog_delta] { num(buf, x) },
            "transport" => for x in [s.ip, s.bt, s.te0, s.ne_bar, s.w_th, s.tau_e, s.p_input, s.p_ohmic, s.p_rad, s.p_loss, s.li] { num(buf, x) },
            "pedestal" => {
                flag(buf, s.in_hmode);
                flag(buf, s.elm_active);
                num(buf, s.elm_type as f64);
                flag(buf, s.elm_suppressed);
                for x in [s.ne_ped, s.te_ped, s.ne_line] { num(buf, x) }
            }
            "impurity" => for x in [s.impurity_fraction, s.d2_puff, s.neon_puff] { num(buf, x) },
            "equilibrium" => {
                for x in [s.axis_r, s.axis_z, s.xpoint_r, s.xpoint_z] { num(buf, x) }
                flag(buf, s.is_limited);
            }
            "profiles" => {
                for i in 0..51 { num(buf, s.te_profile.get(i).copied().unwrap_or(0.0)) }
                for i in 0..51 { num(buf, s.ne_profile.get(i).copied().unwrap_or(0.0)) }
            }
            "physics" => {
                let frac = if s.p_input > 1.0 { s.p_rad / s.p_input } else { 0.0 };
                for x in [s.q95, s.beta_n, s.f_greenwald, frac] { num(buf, x) }
            }
            _ => for x in [s.disruption_risk, s.risk_greenwald, s.risk_beta, s.risk_q95, s.risk_radiation, s.risk_locked_mode] { num(buf, x) },
        }
    }
}

pub fn channel_cells(buf: &mut String, ch: &[f64]) {
    for &x in ch {
        num(buf, x);
    }
}

pub fn opt(x: Option<f64>) -> String {
    x.map(|v| format!("{v:.4}")).unwrap_or_default()
}

/// Summary of a generated dataset.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DatasetResult {
    pub dir: String,
    pub shots: usize,
    pub disrupted: usize,
    pub rows: usize,
    pub columns: usize,
}

/// Generate `shots` discharges into `out/measured.csv` and `out/shots.csv`.
/// `progress(done)` is called after each shot; returning `false` stops.
pub fn generate_dataset(
    out: &Path,
    shots: usize,
    master_seed: u64,
    stride: usize,
    extras: &[String],
    mut progress: impl FnMut(usize) -> bool,
) -> std::io::Result<DatasetResult> {
    std::fs::create_dir_all(out)?;
    let device = jet();
    let mut m = BufWriter::new(File::create(out.join("measured.csv"))?);
    let mut sh = BufWriter::new(File::create(out.join("shots.csv"))?);
    let extra_h = extra_header(extras);
    let mut header = String::from("shot_id,t");
    for n in CHANNEL_NAMES {
        header.push(',');
        header.push_str(n);
    }
    for n in &extra_h {
        header.push(',');
        header.push_str(n);
    }
    writeln!(m, "{header}")?;
    writeln!(sh, "shot_id,seed,master_seed,scenario,duration_s,disrupted,t_trigger_s,t_tq_s,spontaneous,wall_contact,n_steps,t_end_s")?;
    let stride = stride.max(1);
    let (mut rows, mut n_disrupted) = (0usize, 0usize);
    for i in 0..shots {
        let spec = generate_shot(&device, master_seed, i as u64);
        let mut k = 0usize;
        let mut err: Option<std::io::Error> = None;
        let outcome = run_shot(&device, &spec, |s| {
            k += 1;
            if (k - 1) % stride == 0 {
                let mut line = format!("{i},{:.4}", s.time);
                channel_cells(&mut line, &s.channels);
                extra_cells(&mut line, s, extras);
                if let Err(e) = writeln!(m, "{line}") {
                    err = Some(e);
                    return false;
                }
                rows += 1;
            }
            true
        });
        if let Some(e) = err {
            return Err(e);
        }
        n_disrupted += outcome.disrupted as usize;
        writeln!(
            sh,
            "{i},{},{master_seed},{},{:.4},{},{},{},{},{},{},{:.4}",
            spec.seed, spec.scenario, spec.duration, outcome.disrupted, opt(outcome.t_trigger), opt(outcome.t_tq),
            outcome.spontaneous, outcome.wall_contact, outcome.n_steps, outcome.t_end
        )?;
        if !progress(i + 1) {
            m.flush()?;
            sh.flush()?;
            return Ok(DatasetResult { dir: out.display().to_string(), shots: i + 1, disrupted: n_disrupted, rows, columns: 102 + extra_h.len() });
        }
    }
    m.flush()?;
    sh.flush()?;
    Ok(DatasetResult { dir: out.display().to_string(), shots, disrupted: n_disrupted, rows, columns: 102 + extra_h.len() })
}
