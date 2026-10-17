"""Where ada ranks per facet and document, on wall time and on resources; lists who beats it."""
import json, os, sys, statistics as st
from collections import defaultdict
sys.path.insert(0, __file__.rsplit("/", 1)[0])

S = os.environ["B2_ROOT"]  # corpus/, tools/bin/, home/<viewer>/, b2/, b2a/
runs = defaultdict(list)
for l in open(f"{S}/b2/all.jsonl"):
    r = json.loads(l)
    v, d = r["name"].rsplit("-", 1)
    runs[(v, d)].append(r)
V = ["ada", "treemd", "treemd-head", "leaf", "mdfried", "mdtui", "glow"]


def num(x): return isinstance(x, (int, float)) and not isinstance(x, bool)
def med(xs):
    xs = [x for x in xs if num(x)]
    return st.median(xs) if xs else None


def pr(r, kind, key, oc):
    vals = r.get(f"{kind}_{key}") or []
    if oc:
        vals = [v for v, c in zip(vals, r.get(f"{kind}_changed") or []) if c]
    if key in ("shown",) and any(x is None for x in vals):
        return None  # a miss disqualifies the run
    return med(vals)


FACETS = {
    "first paint": lambda r: r.get("first_ms"),
    "settled": lambda r: r.get("settle_ms"),
    "startup CPU": lambda r: r.get("paint_cpu_ms"),
    "peak RSS": lambda r: r.get("hwm_kib"),
    "idle CPU": lambda r: r.get("idle_cpu_ms"),
    "idle wakeups": lambda r: r.get("idle_ctxsw_s"),
    "line wall": lambda r: pr(r, "line", "done", True), "line CPU": lambda r: pr(r, "line", "cpu", True),
    "page wall": lambda r: pr(r, "page", "done", True), "page CPU": lambda r: pr(r, "page", "cpu", True),
    "page bytes": lambda r: pr(r, "page", "bytes", True),
    "end wall": lambda r: pr(r, "end", "shown", False), "end CPU": lambda r: pr(r, "end", "cpu", False),
    "search wall": lambda r: pr(r, "search", "shown", False), "search CPU": lambda r: pr(r, "search", "cpu", False),
    "resize wall": lambda r: pr(r, "resize", "done", False) if any(r.get("resize_changed") or []) else None,
    "resize CPU": lambda r: pr(r, "resize", "cpu", False) if any(r.get("resize_changed") or []) else None,
    "reload wall": lambda r: pr(r, "reload", "shown", False), "reload CPU": lambda r: pr(r, "reload", "cpu", False),
    "held page-down lag": lambda r: r.get("burst_lag_ms"), "held page-down CPU": lambda r: r.get("burst_cpu_ms"),
}
out = []
for facet, fn in FACETS.items():
    for d in ["s", "m", "l", "xl"]:
        vals = {}
        for v in V:
            if d == "xl" and (v.startswith("treemd") or v == "mdtui"):
                continue  # treemd shows only the first `#` section of XL; md-tui never painted it
            x = med([fn(r) for r in runs.get((v, d), [])])
            if x is not None:
                vals[v] = x
        if "ada" not in vals:
            out.append((facet, d, "ada missing", ""))
            continue
        a = vals["ada"]
        better = sorted((x, v) for v, x in vals.items() if v != "ada" and x < a)
        rank = 1 + len(better)
        out.append((facet, d, rank, ", ".join(f"{v} {x:.3g} vs {a:.3g}" for x, v in better)))
for o in out:
    if o[2] != 1:
        print(*o, sep=" | ")
print("\nada first in", sum(1 for o in out if o[2] == 1), "of", len(out))
