//! WASM bindings — exposes the simulation engine to JavaScript.
//!
//! The API is kept simple: all complex data crosses the boundary as JSON strings.
//! This avoids needing to define dozens of wasm-bindgen structs and is fast enough
//! for our ~60 Hz update rate (serialising a SimulationSnapshot is ~50 µs).

use wasm_bindgen::prelude::*;

use crate::devices;
use crate::simulation::{PulseProgram, Simulation, SimulationSnapshot};

/// Generate a u64 seed from JS `Math.random()`.
fn js_random_seed() -> u64 {
    // Math.random() gives a f64 in [0,1) with ~52 bits of entropy.
    // Scale to u64 range.
    (js_sys::Math::random() * u64::MAX as f64) as u64 | 1 // ensure non-zero
}

// ---------------------------------------------------------------------------
// Device catalogue
// ---------------------------------------------------------------------------

/// Return a JSON array of all available devices.
#[wasm_bindgen]
pub fn get_devices_json() -> String {
    let devs = devices::all_devices();
    serde_json::to_string(&devs).unwrap_or_else(|_| "[]".into())
}

/// Return a single device as JSON, or empty string if not found.
#[wasm_bindgen]
pub fn get_device_json(id: &str) -> String {
    match devices::get_device(id) {
        Some(d) => serde_json::to_string(&d).unwrap_or_default(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Preset pulse programs
// ---------------------------------------------------------------------------

/// Return the standard H-mode pulse program as JSON.
#[wasm_bindgen]
pub fn preset_hmode_json(device_id: &str) -> String {
    match devices::get_device(device_id) {
        Some(d) => {
            let prog = PulseProgram::standard_hmode(&d);
            serde_json::to_string(&prog).unwrap_or_default()
        }
        None => String::new(),
    }
}

/// Return the L-mode pulse program as JSON.
#[wasm_bindgen]
pub fn preset_lmode_json(device_id: &str) -> String {
    match devices::get_device(device_id) {
        Some(d) => {
            let prog = PulseProgram::lmode(&d);
            serde_json::to_string(&prog).unwrap_or_default()
        }
        None => String::new(),
    }
}

/// Return the density-limit scenario as JSON.
#[wasm_bindgen]
pub fn preset_density_limit_json(device_id: &str) -> String {
    match devices::get_device(device_id) {
        Some(d) => {
            let prog = PulseProgram::density_limit(&d);
            serde_json::to_string(&prog).unwrap_or_default()
        }
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Simulation handle
// ---------------------------------------------------------------------------

/// Opaque handle to a running simulation, held by the JS side.
#[wasm_bindgen]
pub struct SimHandle {
    sim: Simulation,
}

#[wasm_bindgen]
impl SimHandle {
    /// Create a new simulation from a device id and a JSON pulse program.
    /// Returns `None` (throws in JS) if inputs are invalid.
    #[wasm_bindgen(constructor)]
    pub fn new(device_id: &str, program_json: &str) -> Result<SimHandle, JsValue> {
        let device = devices::get_device(device_id)
            .ok_or_else(|| JsValue::from_str(&format!("Unknown device: {device_id}")))?;

        let program: PulseProgram = serde_json::from_str(program_json)
            .map_err(|e| JsValue::from_str(&format!("Bad program JSON: {e}")))?;

        let mut sim = Simulation::new(device, program);
        sim.seed_disruption(js_random_seed());
        Ok(SimHandle { sim })
    }

    /// Create from a device id + preset name ("hmode", "lmode", "density_limit").
    #[wasm_bindgen]
    pub fn from_preset(device_id: &str, preset: &str) -> Result<SimHandle, JsValue> {
        let device = devices::get_device(device_id)
            .ok_or_else(|| JsValue::from_str(&format!("Unknown device: {device_id}")))?;

        let program = match preset {
            "hmode" => PulseProgram::standard_hmode(&device),
            "lmode" => PulseProgram::lmode(&device),
            "density_limit" => PulseProgram::density_limit(&device),
            _ => return Err(JsValue::from_str(&format!("Unknown preset: {preset}"))),
        };

        let mut sim = Simulation::new(device, program);
        sim.seed_disruption(js_random_seed());
        Ok(SimHandle { sim })
    }

    /// Advance the simulation by `dt` seconds and return the snapshot as JSON.
    /// Automatically transitions from Ready → Running on the first step.
    #[wasm_bindgen]
    pub fn step(&mut self, dt: f64) -> String {
        // Auto-start if still in Ready state
        if self.sim.status == crate::simulation::SimulationStatus::Ready {
            self.sim.start();
        }
        let snap = self.sim.step(dt);
        serde_json::to_string(&snap).unwrap_or_else(|_| "{}".into())
    }

    /// Reset the simulation to t = 0.
    #[wasm_bindgen]
    pub fn reset(&mut self) {
        self.sim.reset();
        self.sim.seed_disruption(js_random_seed());
    }

    /// Current simulation time in seconds.
    #[wasm_bindgen]
    pub fn time(&self) -> f64 {
        self.sim.time
    }

    /// Whether the simulation is still running.
    #[wasm_bindgen]
    pub fn is_running(&self) -> bool {
        matches!(
            self.sim.status,
            crate::simulation::SimulationStatus::Running
                | crate::simulation::SimulationStatus::Ready
        )
    }

    /// Get the current status as a string.
    #[wasm_bindgen]
    pub fn status(&self) -> String {
        format!("{:?}", self.sim.status)
    }

    /// Override the device's fuel mass number (e.g. 2.0 for DD, 2.5 for DT).
    /// Must be called before starting the simulation (before the first `step`).
    #[wasm_bindgen]
    pub fn set_mass_number(&mut self, mass: f64) {
        self.sim.device.mass_number = mass;
    }

    /// Override one actuator channel mid-pulse. Channels: "ip" (MA), "bt" (T),
    /// "ne_target" (10²⁰ m⁻³), "p_nbi"/"p_ech"/"p_ich" (MW), "kappa", "delta",
    /// "d2_puff"/"neon_puff" (10²⁰ particles/s). Takes effect on the next `step`.
    #[wasm_bindgen]
    pub fn set_override(&mut self, channel: &str, value: f64) -> Result<(), JsValue> {
        self.sim.set_override(channel, value).map_err(|e| JsValue::from_str(&e))
    }

    /// Return one channel to its programmed waveform.
    #[wasm_bindgen]
    pub fn clear_override(&mut self, channel: &str) -> Result<(), JsValue> {
        self.sim.clear_override(channel).map_err(|e| JsValue::from_str(&e))
    }

    /// Return every channel to its programmed waveform.
    #[wasm_bindgen]
    pub fn clear_overrides(&mut self) {
        self.sim.overrides = Default::default();
    }

    /// Active overrides as JSON (`null` = following the program).
    #[wasm_bindgen]
    pub fn overrides_json(&self) -> String {
        serde_json::to_string(&self.sim.overrides).unwrap_or_else(|_| "{}".into())
    }

    /// Replace the pulse program mid-pulse (physics state is kept).
    #[wasm_bindgen]
    pub fn set_program_json(&mut self, program_json: &str) -> Result<(), JsValue> {
        self.sim.program = serde_json::from_str(program_json)
            .map_err(|e| JsValue::from_str(&format!("Bad program JSON: {e}")))?;
        Ok(())
    }

    /// Re-seed the disruption RNG (for reproducible runs). Small seeds are
    /// scrambled first — xorshift's first draws from e.g. 1 are ≈ 0.
    #[wasm_bindgen]
    pub fn seed(&mut self, seed: u64) {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        self.sim.seed_disruption(z ^ (z >> 31));
    }

    /// Get the device wall outline as JSON array of [r, z] points.
    #[wasm_bindgen]
    pub fn wall_outline_json(&self) -> String {
        let wall: Vec<[f64; 2]> = self.sim.device.wall_outline.iter()
            .map(|(r, z)| [*r, *z])
            .collect();
        serde_json::to_string(&wall).unwrap_or_else(|_| "[]".into())
    }
}

// ---------------------------------------------------------------------------
// Standalone snapshot helper (for when JS wants to deserialise typed data)
// ---------------------------------------------------------------------------

/// Parse a snapshot JSON string and return just the disruption risk (0..1).
/// Useful for the disruption gauge without parsing the full object in JS.
#[wasm_bindgen]
pub fn snapshot_disruption_risk(json: &str) -> f64 {
    serde_json::from_str::<SimulationSnapshot>(json)
        .map(|s| s.disruption_risk)
        .unwrap_or(0.0)
}
