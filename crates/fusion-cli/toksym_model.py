"""Wrapper that turns a plain class into a test-run model (toksym protocol 1).

    class Model:
        def start_shot(self, meta): ...        # new discharge
        def predict(self, t, channels): ...    # disruption probability, 0…1

    if __name__ == "__main__":
        from toksym_model import serve
        serve(Model())

`channels` maps channel name → float; a failed instrument gives nan.
`predict` may return a dict {"p": .., "share_pct": {..}, "risk_total": ..} when
`model.rich_answers` is True (set by fusionsim, never by toksym).
Print with `log(...)` (lines starting with '#'), never bare print().
"""
import json
import sys

PROTOCOL = 1


def log(*parts):
    """Print something for yourself without breaking the protocol."""
    print("#", *parts, flush=True)


def _clean(x):
    """JSON-safe copy: numpy scalars → float, nan/inf → None, keys → str."""
    if isinstance(x, dict):
        return {str(k): _clean(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [_clean(v) for v in x]
    if isinstance(x, bool) or x is None or isinstance(x, str):
        return x
    try:
        f = float(x)
    except (TypeError, ValueError):
        return str(x)
    return f if f == f and abs(f) != float("inf") else None


def serve(model, stdin=None, stdout=None):
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout

    def say(line):
        stdout.write(f"{line}\n")
        stdout.flush()

    names = []
    for raw in stdin:
        raw = raw.strip()
        if not raw:
            continue
        ev = json.loads(raw)
        kind = ev.get("type")
        if kind == "hello":
            if ev.get("protocol") != PROTOCOL:
                raise SystemExit(f"protocol {ev.get('protocol')} instead of {PROTOCOL}")
            # fusionsim may accept a JSON answer with mechanism shares; toksym never
            # sends "rich", so a model that checks the flag stays toksym-compatible.
            model.rich_answers = bool(ev.get("rich"))
            say(json.dumps({"type": "ready"}))
        elif kind == "shot_start":
            names = list(ev["channels"])
            if hasattr(model, "start_shot"):
                model.start_shot({k: ev[k] for k in ("shot_id", "dt", "duration", "scenario")})
        elif kind == "step":
            channels = {n: (float("nan") if v is None else float(v)) for n, v in zip(names, ev["v"])}
            ans = model.predict(ev["t"], channels)
            if isinstance(ans, dict):
                say(json.dumps(_clean(ans)))
            else:
                say(repr(float(ans)))
        elif kind == "shot_end":
            if hasattr(model, "end_shot"):
                model.end_shot(ev["shot_id"])
        elif kind == "bye":
            return
