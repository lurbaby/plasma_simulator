"""Closed loop without the browser: a toksym-style model steers the plasma.

Each 5 ms step the headless engine hands the model the same hundred channels
a test run would; when p crosses the threshold the controller changes an
actuator, chosen by the operational limit the instruments say is nearest.

    python3 tools/closed_loop.py --model ../models/submission_v8 --shots 20 --seed 3
    python3 tools/closed_loop.py --model ../models/submission_v8 --shots 20 --seed 3 --no-control

Prints, per shot, whether it disrupted and every action taken.
"""
import argparse
import importlib.util
import json
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from fusion_sim_client import FusionSim  # noqa: E402

CHANNELS = None


def load_model(folder):
    """Import model.py from a submission folder (with the toksym wrapper available)."""
    sys.path.insert(0, os.path.join(ROOT, "crates", "fusion-cli"))
    manifest = json.load(open(os.path.join(folder, "toksym.json")))
    cwd = os.getcwd()
    os.chdir(folder)
    sys.path.insert(0, folder)
    spec = importlib.util.spec_from_file_location("submission_model", os.path.join(folder, manifest["entry"]))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    model = mod.Model()
    os.chdir(cwd)
    return model


def nearest_limit(s):
    """Which limit is closest, from the measured channels (no truth used)."""
    ch = dict(zip(CHANNELS, s["channels"]))
    ip = max(ch["mag_ip_MA"], 1e-3)
    n_gw = ip / (math.pi * 0.95 ** 2)
    ne = ch["ifr_chord_core"] / (1.0231 * 1.62 * 1.9)  # chord → line-averaged density, roughly
    f_gw = ne / n_gw
    q = 3.2 * (ch["mag_bt_T"] / ip) / (2.6 / 2.4)      # q95 ∝ Bt/Ip, normalised at the generator's cap
    p_in = ch["nbi_power_MW"] + ch["ich_power_MW"] + ch["ech_power_MW"] + ch["mag_vloop_V"] * ip
    p_rad = sum(ch[f"bolo_{k:02d}"] for k in range(1, 9))
    rad = p_rad / p_in if p_in > 1 else 0.0
    scores = {"greenwald": f_gw / 1.0, "kink": (2.2 / max(q, 0.5)) / 0.9, "radiation": rad / 0.8}
    return max(scores, key=scores.get), ch


def act(sim, cause, ch, s):
    ip = max(ch["mag_ip_MA"], 0.3)
    if cause == "greenwald":
        ne = 0.7 * ip / (math.pi * 0.95 ** 2)   # back to 70 % of the Greenwald limit
        sim.set("ne_target", ne)
        sim.set("d2_puff", 0.0)
        return f"Greenwald → n̄e target {ne:.2f} (0.7 n_GW), D2 off"
    if cause == "kink":
        q_now = 3.2 * (ch["mag_bt_T"] / ip) / (2.6 / 2.4)
        ip_new = max(1.0, ip * min(0.95, q_now / 3.2))
        sim.set("ip", ip_new)
        return f"kink → Ip {ip_new:.2f} MA (q95 ≈ 3.2)"
    sim.set("neon_puff", 0.0)
    sim.set("p_nbi", min(34.0, ch["nbi_power_MW"] + 5.0))
    return "radiation → neon off, NBI +5 MW"


def main():
    global CHANNELS
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--shots", type=int, default=10)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--threshold", type=float, default=0.5)
    ap.add_argument("--hold", type=float, default=0.3, help="min seconds between actions")
    ap.add_argument("--no-control", action="store_true")
    a = ap.parse_args()
    sys.path.insert(0, os.path.join(ROOT, "crates", "fusion-cli"))
    model = load_model(os.path.abspath(a.model))
    disrupted = 0
    for shot in range(a.shots):
        with FusionSim("jet", f"generated:{a.seed}:{shot}") as sim:
            info = sim.program()
            CHANNELS = info["channel_names"]
            model.start_shot({"shot_id": shot, "dt": 0.005, "duration": info["duration"], "scenario": info["scenario"]})
            last_action, log = -1e9, []
            while True:
                s = sim.step()
                if s["status"] != "Running" or s.get("t_tq") is not None:
                    break
                chans = {n: v for n, v in zip(CHANNELS, s["channels"])}
                p = float(model.predict(round(s["time"], 4), chans))
                if not a.no_control and p > a.threshold and s["time"] - last_action >= a.hold:
                    cause, ch = nearest_limit(s)
                    log.append(f"  t={s['time']:.2f}s p={p:.2f} {act(sim, cause, ch, s)}")
                    last_action = s["time"]
            d = s.get("t_trigger") is not None
            disrupted += d
            print(f"shot {shot:3d} {info['scenario']:22s} {'DISRUPTED' if d else 'ok'}  actions={len(log)}")
            for line in log[:6]:
                print(line)
    print(f"\n{disrupted} of {a.shots} disrupted ({'no control' if a.no_control else 'controlled'})")


if __name__ == "__main__":
    main()
