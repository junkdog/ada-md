"""Aggregate B2 runs into markdown tables: median across runs of each run's median."""
import json, sys, statistics as st
from collections import defaultdict

V = ["ada", "treemd", "treemd-head", "leaf", "mdfried", "mdtui", "glow"]
NAME = {"ada": "**ada**", "treemd": "treemd 0.9.1", "treemd-head": "treemd HEAD", "leaf": "leaf", "mdfried": "mdfried",
        "mdtui": "md-tui", "glow": "glow"}
DOCS = ["s", "m", "l", "xl"]
runs = defaultdict(list)
for l in open(sys.argv[1]):
    r = json.loads(l)
    v, d = r["name"].rsplit("-", 1)
    runs[(v, d)].append(r)


def num(x):
    return isinstance(x, (int, float)) and not isinstance(x, bool)


def med(xs):
    xs = [x for x in xs if num(x)]
    return st.median(xs) if xs else None


def per_run(r, kind, key, only_changed=False):
    vals = r.get(f"{kind}_{key}") or []
    if only_changed:
        ch = r.get(f"{kind}_changed") or []
        vals = [v for v, c in zip(vals, ch) if c]
    return med(vals)


def agg(v, d, fn):
    return med([fn(r) for r in runs.get((v, d), [])])


def misses(v, d, kind, key="shown"):
    tot = sum(len(r.get(f"{kind}_{key}") or []) for r in runs.get((v, d), []))
    miss = sum(sum(1 for x in (r.get(f"{kind}_{key}") or []) if x is None) for r in runs.get((v, d), []))
    return miss, tot


def ms(x, d=1):
    if x is None:
        return "–"
    if x >= 1000:
        return f"{x / 1000:.2f} s"
    if x >= 100:
        return f"{x:.0f}"
    return f"{x:.{d}f}"


def mib(x):
    return "–" if x is None else (f"{x / 1024:.2f} GiB" if x >= 1024 else f"{x:.1f}")


def kb(x):
    return "–" if x is None else f"{x / 1000:.1f} KB"


def cell(wall, cpu, d=1):
    if wall is None:
        return "–"
    return f"{ms(wall, d)} ({ms(cpu, d)})" if cpu is not None else ms(wall, d)


def table(header, rows):
    out = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


sec = sys.argv[2] if len(sys.argv) > 2 else "all"

# 1. startup
rows = []
for v in V:
    row = [NAME[v]]
    for d in DOCS:
        f = agg(v, d, lambda r: r.get("first_ms"))
        capped = all(r.get("first_ms") is None for r in runs.get((v, d), [])) and runs.get((v, d))
        row.append("never (90 s cap)" if capped and d == "xl" else ms(f))
    for d in DOCS:
        row.append(cell(agg(v, d, lambda r: r.get("settle_ms")), agg(v, d, lambda r: r.get("paint_cpu_ms"))))
    rows.append(row)
print("#### startup\n")
print(table(["Viewer", "First S", "M", "L", "XL", "Settled S (CPU)", "M", "L", "XL"], rows))

# 2. memory, idle
rows = []
for v in V:
    row = [NAME[v]]
    for d in DOCS:
        row.append(mib(agg(v, d, lambda r: (r.get("hwm_kib") or 0) / 1024 or None)))
    for d in ("l", "xl"):
        row.append(f"{ms(agg(v, d, lambda r: r.get('idle_cpu_ms')))} / {ms(agg(v, d, lambda r: r.get('idle_ctxsw_s')), 0)}")
    rows.append(row)
print("\n#### memory and idle\n")
print(table(["Viewer", "Peak RSS S", "M", "L", "XL", "Idle L: CPU ms in 5 s / wakeups/s", "Idle XL"], rows))

# 3. interaction per doc
for d in DOCS:
    rows = []
    for v in V:
        row = [NAME[v]]
        for kind, oc in (("line", True), ("page", True), ("end", False), ("search", False), ("resize", False), ("reload", False)):
            if kind in ("end", "search", "reload"):
                wall = agg(v, d, lambda r: per_run(r, kind, "shown"))
                m, t = misses(v, d, kind)
                cpu = agg(v, d, lambda r: per_run(r, kind, "cpu"))
                c = cell(wall, cpu)
                if t and m == t:
                    c = "missed" if kind != "search" or v != "glow" else "–"
                elif m:
                    c += f" [{m}/{t} missed]"
                if kind == "search" and v == "glow":
                    c = "no search"
            else:
                wall = agg(v, d, lambda r: per_run(r, kind, "done", oc))
                cpu = agg(v, d, lambda r: per_run(r, kind, "cpu", oc))
                c = cell(wall, cpu, 2)
                if kind == "resize":
                    ch = sum(sum(1 for x in (r.get("resize_changed") or []) if x) for r in runs.get((v, d), []))
                    if runs.get((v, d)) and ch == 0:
                        c = "no repaint in 5 s"
            row.append(c)
        rows.append(row)
    print(f"\n#### interaction, {d.upper()}\n")
    print(table(["Viewer", "Line", "Page", "End", "Search", "Resize", "Reload"], rows))

# 4. held page-down
rows = []
for v in V:
    row = [NAME[v]]
    for d in DOCS:
        lag = agg(v, d, lambda r: r.get("burst_lag_ms"))
        cpu = agg(v, d, lambda r: r.get("burst_cpu_ms"))
        row.append(f"{ms(lag)} ({ms(cpu)})")
    rows.append(row)
print("\n#### held page-down: lag after the last key (CPU over the burst)\n")
print(table(["Viewer", "S", "M", "L", "XL"], rows))

# 5. bytes per page and session totals
rows = []
for v in V:
    row = [NAME[v]]
    for d in ("s", "l"):
        row.append(kb(agg(v, d, lambda r: per_run(r, "page", "bytes", True))))
    for d in ("l", "xl"):
        c = agg(v, d, lambda r: r.get("total_cpu_ms")); w = agg(v, d, lambda r: r.get("total_wall_ms"))
        row.append("–" if c is None else f"{ms(c)} / {ms(w)} ({100 * c / w:.1f}%)")
    rows.append(row)
print("\n#### bytes per page, and whole-session CPU over wall\n")
print(table(["Viewer", "Bytes per page S", "L", "Session L: CPU / wall", "Session XL"], rows))
