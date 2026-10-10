"""Python driver for the headless fusion-sim engine (crates/tok-sym-core/src/bin/headless.rs).

    sim = FusionSim("jet", "hmode", seed=1)
    s = sim.step(n=100)                 # 100 × 5 ms
    sim.set("ne_target", 0.6)           # override an actuator from now on
    sim.clear("ne_target")              # back to the programmed waveform

Channels: ip [MA], bt [T], ne_target [1e20 m^-3], p_nbi/p_ech/p_ich [MW],
kappa, delta, d2_puff/neon_puff [1e20 particles/s].
"""
import json
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "headless")


class FusionSim:
    def __init__(self, device="jet", program="hmode", seed=1, binary=BIN):
        self.proc = subprocess.Popen(
            [binary, device, program, str(seed)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
        )

    def _call(self, **cmd):
        self.proc.stdin.write(json.dumps(cmd) + "\n")
        self.proc.stdin.flush()
        reply = json.loads(self.proc.stdout.readline())
        if isinstance(reply, dict) and "error" in reply:
            raise RuntimeError(reply["error"])
        return reply

    def step(self, dt=0.005, n=1):
        return self._call(cmd="step", dt=dt, n=n)

    def set(self, channel, value):
        return self._call(cmd="set", channel=channel, value=float(value))

    def clear(self, channel="*"):
        return self._call(cmd="clear", channel=channel)

    def snapshot(self):
        return self._call(cmd="snapshot")

    def program(self):
        return self._call(cmd="program")

    def close(self):
        if self.proc.poll() is None:
            self.proc.stdin.write('{"cmd":"quit"}\n')
            self.proc.stdin.close()
            self.proc.wait(timeout=5)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
