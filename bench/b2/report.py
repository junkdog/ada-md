"""Aggregate B2 runs into markdown tables: median across runs of each run's median.

One number per cell, in the unit its header names, so that a table sorts by any column; a cell
without a number is empty, and what it lacks is listed under its table."""
import json, os, sys, statistics as st
from collections import defaultdict
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's directory out
import timeouts

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
        return ""
    if x >= 1000:
        return f"{x:,.0f}"
    if x >= 100:
        return f"{x:.0f}"
    return f"{x:.{d}f}"


def mib(x):
    return "" if x is None else f"{x:,.1f}"


def kb(x):
    return "" if x is None else f"{x / 1000:.1f}"


def table(header, rows, notes=()):
    out = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    out += [""] + [f"- {n}" for n in notes] if notes else []
    return "\n".join(out)


sec = sys.argv[2] if len(sys.argv) > 2 else "all"

# 1. startup
rows, notes = [], []
for v in V:
    row = [NAME[v]]
    for d in DOCS:
        f = agg(v, d, lambda r: r.get("first_ms"))
        if f is None and runs.get((v, d)):
            notes.append(f"{NAME[v]} {d.upper()}: never painted (90 s cap)")
        elif f"{v}-{d}" in timeouts.listed("b2") and not runs.get((v, d)):
            notes.append(f"{NAME[v]} {d.upper()}: not run (timeouts.py: it never paints in the 90 s cap)")
        row.append(ms(f))
    row += [ms(agg(v, d, lambda r: r.get("settle_ms"))) for d in DOCS]
    row += [ms(agg(v, d, lambda r: r.get("paint_cpu_ms"))) for d in DOCS]
    rows.append(row)
print("#### startup: first paint, settled, and CPU until settled (ms)\n")
print(table(["Viewer"] + [f"{k} {d.upper()}" for k in ("First", "Settled", "CPU") for d in DOCS], rows, notes))

# 2. memory, idle
rows = []
for v in V:
    row = [NAME[v]]
    for d in DOCS:
        row.append(mib(agg(v, d, lambda r: (r.get("hwm_kib") or 0) / 1024 or None)))
    for d in ("l", "xl"):
        row.append(ms(agg(v, d, lambda r: r.get("idle_cpu_ms"))))
        row.append(ms(agg(v, d, lambda r: r.get("idle_ctxsw_s")), 0))
    rows.append(row)
print("\n#### memory (peak RSS, MiB) and idle (CPU ms in 5 s, wakeups a second)\n")
print(table(["Viewer", "RSS S", "RSS M", "RSS L", "RSS XL", "Idle CPU L", "Wakeups L", "Idle CPU XL", "Wakeups XL"], rows))

# 3. interaction per doc
KINDS = ("line", "page", "end", "search", "resize", "reload")
for d in DOCS:
    rows, notes = [], []
    for v in V:
        row = [NAME[v]]
        for kind in KINDS:
            note = None
            if kind in ("end", "search", "reload"):
                wall = agg(v, d, lambda r: per_run(r, kind, "shown"))
                cpu = agg(v, d, lambda r: per_run(r, kind, "cpu"))
                m, t = misses(v, d, kind)
                if kind == "search" and v == "glow":
                    wall = cpu = None
                    note = "no search"
                elif t and m == t:
                    wall = cpu = None
                    note = "missed"
                elif m:
                    note = f"{m}/{t} missed"
                d1, d2 = ms(wall), ms(cpu)
            else:
                oc = kind in ("line", "page")
                wall = agg(v, d, lambda r: per_run(r, kind, "done", oc))
                cpu = agg(v, d, lambda r: per_run(r, kind, "cpu", oc))
                if kind == "resize":
                    ch = sum(sum(1 for x in (r.get("resize_changed") or []) if x) for r in runs.get((v, d), []))
                    if runs.get((v, d)) and ch == 0:
                        wall = cpu = None
                        note = "no repaint in 5 s"
                d1, d2 = ms(wall, 2), ms(cpu, 2)
            if note:
                notes.append(f"{NAME[v]} {kind}: {note}")
            row += [d1, d2]
        rows.append(row)
    print(f"\n#### interaction, {d.upper()} (ms)\n")
    print(table(["Viewer"] + [h for k in KINDS for h in (k.capitalize(), f"{k.capitalize()} CPU")], rows, notes))

# 4. held page-down
rows = []
for v in V:
    row = [NAME[v]]
    row += [ms(agg(v, d, lambda r: r.get("burst_lag_ms"))) for d in DOCS]
    row += [ms(agg(v, d, lambda r: r.get("burst_cpu_ms"))) for d in DOCS]
    rows.append(row)
print("\n#### held page-down: lag past one key interval after the last key, and CPU over the burst (ms)\n")
print(table(["Viewer"] + [f"{k} {d.upper()}" for k in ("Lag", "CPU") for d in DOCS], rows))

# 5. bytes per page and session totals
rows = []
for v in V:
    row = [NAME[v]]
    for d in ("s", "l"):
        row.append(kb(agg(v, d, lambda r: per_run(r, "page", "bytes", True))))
    for d in ("l", "xl"):
        c = agg(v, d, lambda r: r.get("total_cpu_ms")); w = agg(v, d, lambda r: r.get("total_wall_ms"))
        row += ["", "", ""] if c is None else [f"{c / 1000:.2f}", f"{w / 1000:.1f}", f"{100 * c / w:.1f}"]
    rows.append(row)
print("\n#### bytes per page (KB), and whole-session CPU and wall (s)\n")
print(table(["Viewer", "Page S", "Page L", "Session L CPU", "Session L wall", "Session L CPU %",
             "Session XL CPU", "Session XL wall", "Session XL CPU %"], rows))
