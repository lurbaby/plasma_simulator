/* tslint:disable */
/* eslint-disable */

/**
 * Opaque handle to a running simulation, held by the JS side.
 */
export class SimHandle {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Return one channel to its programmed waveform.
     */
    clear_override(channel: string): void;
    /**
     * Return every channel to its programmed waveform.
     */
    clear_overrides(): void;
    /**
     * Apply a fixer action by name (e.g. "ne_-10", "relax") to the current
     * multipliers — how a student model's decision reaches the plasma.
     * Returns false for an unknown name.
     */
    fixer_apply_action(name: string): boolean;
    /**
     * Start a fixer decision: copy the current state; candidates are then
     * evaluated one per `fixer_step()` call, so the work spreads over frames.
     */
    fixer_begin(horizon: number, dt: number, alpha: number): void;
    /**
     * Progress of the current decision: [done, total].
     */
    fixer_progress(): Uint32Array;
    /**
     * The finished decision as JSON (and forget the planner), or "" if none.
     */
    fixer_result(): string;
    /**
     * Evaluate the next candidate; returns true when the decision is ready.
     */
    fixer_step(): boolean;
    /**
     * Create from a device id + preset name ("hmode", "lmode", "density_limit").
     */
    static from_preset(device_id: string, preset: string): SimHandle;
    /**
     * Whether the simulation is still running.
     */
    is_running(): boolean;
    /**
     * Create a new simulation from a device id and a JSON pulse program.
     * Returns `None` (throws in JS) if inputs are invalid.
     */
    constructor(device_id: string, program_json: string);
    /**
     * Active overrides as JSON (`null` = following the program).
     */
    overrides_json(): string;
    /**
     * Reset the simulation to t = 0.
     */
    reset(): void;
    /**
     * Return every multiplier to 1 (the programme as written).
     */
    reset_scales(): void;
    /**
     * Re-seed the disruption RNG (for reproducible runs). Small seeds are
     * scrambled first — xorshift's first draws from e.g. 1 are ≈ 0.
     */
    seed(seed: bigint): void;
    /**
     * Override the device's fuel mass number (e.g. 2.0 for DD, 2.5 for DT).
     * Must be called before starting the simulation (before the first `step`).
     */
    set_mass_number(mass: number): void;
    /**
     * Override one actuator channel mid-pulse. Channels: "ip" (MA), "bt" (T),
     * "ne_target" (10²⁰ m⁻³), "p_nbi"/"p_ech"/"p_ich" (MW), "kappa", "delta",
     * "d2_puff"/"neon_puff" (10²⁰ particles/s). Takes effect on the next `step`.
     */
    set_override(channel: string, value: number): void;
    /**
     * Replace the pulse program mid-pulse (physics state is kept).
     */
    set_program_json(program_json: string): void;
    /**
     * Set the controller multipliers on the programme (JSON ProgramScales).
     */
    set_scales_json(json: string): void;
    /**
     * Get the current status as a string.
     */
    status(): string;
    /**
     * Advance the simulation by `dt` seconds and return the snapshot as JSON.
     * Automatically transitions from Ready → Running on the first step.
     */
    step(dt: number): string;
    /**
     * Current simulation time in seconds.
     */
    time(): number;
    /**
     * Get the device wall outline as JSON array of [r, z] points.
     */
    wall_outline_json(): string;
}

/**
 * Return a single device as JSON, or empty string if not found.
 */
export function get_device_json(id: string): string;

/**
 * Return a JSON array of all available devices.
 */
export function get_devices_json(): string;

/**
 * Return the density-limit scenario as JSON.
 */
export function preset_density_limit_json(device_id: string): string;

/**
 * Return the standard H-mode pulse program as JSON.
 */
export function preset_hmode_json(device_id: string): string;

/**
 * Return the L-mode pulse program as JSON.
 */
export function preset_lmode_json(device_id: string): string;

/**
 * Parse a snapshot JSON string and return just the disruption risk (0..1).
 * Useful for the disruption gauge without parsing the full object in JS.
 */
export function snapshot_disruption_risk(json: string): number;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_simhandle_free: (a: number, b: number) => void;
    readonly get_device_json: (a: number, b: number) => [number, number];
    readonly get_devices_json: () => [number, number];
    readonly preset_density_limit_json: (a: number, b: number) => [number, number];
    readonly preset_hmode_json: (a: number, b: number) => [number, number];
    readonly preset_lmode_json: (a: number, b: number) => [number, number];
    readonly simhandle_clear_override: (a: number, b: number, c: number) => [number, number];
    readonly simhandle_clear_overrides: (a: number) => void;
    readonly simhandle_fixer_apply_action: (a: number, b: number, c: number) => number;
    readonly simhandle_fixer_begin: (a: number, b: number, c: number, d: number) => void;
    readonly simhandle_fixer_progress: (a: number) => [number, number];
    readonly simhandle_fixer_result: (a: number) => [number, number];
    readonly simhandle_fixer_step: (a: number) => number;
    readonly simhandle_from_preset: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly simhandle_is_running: (a: number) => number;
    readonly simhandle_new: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly simhandle_overrides_json: (a: number) => [number, number];
    readonly simhandle_reset: (a: number) => void;
    readonly simhandle_reset_scales: (a: number) => void;
    readonly simhandle_seed: (a: number, b: bigint) => void;
    readonly simhandle_set_mass_number: (a: number, b: number) => void;
    readonly simhandle_set_override: (a: number, b: number, c: number, d: number) => [number, number];
    readonly simhandle_set_program_json: (a: number, b: number, c: number) => [number, number];
    readonly simhandle_set_scales_json: (a: number, b: number, c: number) => [number, number];
    readonly simhandle_status: (a: number) => [number, number];
    readonly simhandle_step: (a: number, b: number) => [number, number];
    readonly simhandle_time: (a: number) => number;
    readonly simhandle_wall_outline_json: (a: number) => [number, number];
    readonly snapshot_disruption_risk: (a: number, b: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
