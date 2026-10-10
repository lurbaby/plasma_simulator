//! Headless driver: runs the simulation natively and talks JSON lines over
//! stdin/stdout, so an external process (e.g. a Python disruption predictor)
//! can step the plasma and adjust actuators mid-pulse.
//!
//! Usage: `headless <device> <preset|program.json> [seed]`
//!
//! Commands (one JSON object per line on stdin; one reply per line on stdout):
//!   {"cmd":"step","dt":0.005,"n":1}          → compact state after n steps
//!   {"cmd":"set","channel":"ip","value":2.0} → {"ok":true}
//!   {"cmd":"clear","channel":"ip"}           → {"ok":true} (channel "*" clears all)
//!   {"cmd":"snapshot"}                       → full SimulationSnapshot
//!   {"cmd":"program"}                        → current PulseProgram + scenario, channel_names
//!
//! Program may also be `generated:SEED:SHOT` — the dataset/test-run discharge.
//!   {"cmd":"quit"}

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};
use tok_sym_core::devices;
use tok_sym_core::simulation::{
    ProgramOverrides, PulseProgram, Simulation, SimulationSnapshot, SimulationStatus,
};

fn compact(s: &SimulationSnapshot) -> Value {
    json!({
        "time": s.time, "status": s.status, "disrupted": s.disrupted,
        "ip": s.ip, "bt": s.bt, "ne_bar": s.ne_bar, "f_greenwald": s.f_greenwald,
        "q95": s.q95, "beta_n": s.beta_n, "p_input": s.p_input, "p_rad": s.p_rad,
        "w_th": s.w_th, "te0": s.te0, "li": s.li, "in_hmode": s.in_hmode,
        "impurity_fraction": s.impurity_fraction,
        "prog_ip": s.prog_ip, "prog_bt": s.prog_bt, "prog_ne": s.prog_ne,
        "disruption_risk": s.disruption_risk,
        "risk_greenwald": s.risk_greenwald, "risk_beta": s.risk_beta,
        "risk_q95": s.risk_q95, "risk_radiation": s.risk_radiation,
        "risk_locked_mode": s.risk_locked_mode,
        "overrides": s.overrides, "diagnostics": s.diagnostics, "channels": s.channels,
        "t_trigger": s.t_trigger, "t_tq": s.t_tq,
    })
}

/// Spread small user seeds over all 64 bits: xorshift's first draws from a
/// seed like 1 are ≈ 0, which would trigger a disruption on the first step.
fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: {} <device> <hmode|lmode|density_limit|program.json> [seed]", args[0]);
        std::process::exit(2);
    }
    let device = devices::get_device(&args[1]).unwrap_or_else(|| {
        eprintln!("unknown device {}", args[1]);
        std::process::exit(2)
    });
    // "generated:SEED:SHOT" is shot SHOT of the dataset family seeded SEED —
    // the same discharge `fusionsim gen`/`score` produce.
    let mut scenario = String::from("interactive");
    let mut shot_seed: Option<u64> = None;
    let program = match args[2].as_str() {
        g if g.starts_with("generated:") => {
            let parts: Vec<u64> = g.split(':').skip(1).filter_map(|x| x.parse().ok()).collect();
            let spec = tok_sym_core::generator::generate_shot(&device, parts.first().copied().unwrap_or(1), parts.get(1).copied().unwrap_or(0));
            scenario = spec.scenario.clone();
            shot_seed = Some(spec.seed);
            spec.program
        }
        "hmode" => PulseProgram::standard_hmode(&device),
        "lmode" => PulseProgram::lmode(&device),
        "density_limit" => PulseProgram::density_limit(&device),
        path => serde_json::from_str(&std::fs::read_to_string(path).expect("read program"))
            .expect("parse program JSON"),
    };
    let seed: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let mut sim = Simulation::new(device, program);
    sim.seed_disruption(splitmix64(shot_seed.unwrap_or(seed)));
    sim.compute_geometry = false;
    sim.start();
    let mut last = sim.step(0.0);

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in io::stdin().lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        if line.trim().is_empty() { continue; }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(e) => json!({"error": format!("bad JSON: {e}")}),
            Ok(cmd) => match cmd["cmd"].as_str().unwrap_or("") {
                "step" => {
                    let dt = cmd["dt"].as_f64().unwrap_or(0.005);
                    let n = cmd["n"].as_u64().unwrap_or(1);
                    for _ in 0..n {
                        last = sim.step(dt);
                        if sim.status != SimulationStatus::Running { break; }
                    }
                    compact(&last)
                }
                "set" => {
                    let ch = cmd["channel"].as_str().unwrap_or("");
                    match cmd["value"].as_f64() {
                        None => json!({"error": "value must be a number"}),
                        Some(v) => match sim.set_override(ch, v) {
                            Ok(()) => json!({"ok": true}),
                            Err(e) => json!({"error": e}),
                        },
                    }
                }
                "clear" => {
                    let ch = cmd["channel"].as_str().unwrap_or("*");
                    if ch == "*" {
                        sim.overrides = ProgramOverrides::default();
                        json!({"ok": true})
                    } else {
                        match sim.clear_override(ch) {
                            Ok(()) => json!({"ok": true}),
                            Err(e) => json!({"error": e}),
                        }
                    }
                }
                "snapshot" => serde_json::to_value(&last).unwrap_or(Value::Null),
                "program" => {
                    let mut v = serde_json::to_value(&sim.program).unwrap_or(Value::Null);
                    v["scenario"] = json!(scenario);
                    v["channel_names"] = json!(tok_sym_core::channels::CHANNEL_NAMES.to_vec());
                    v
                }
                "quit" => break,
                other => json!({"error": format!("unknown cmd {other:?}")}),
            },
        };
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() { break; }
    }
}
