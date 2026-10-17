"""The B2 tables of the vault notes, in their layout: feature-matrix-comparison's Performance (B2) tables, with
the × columns (each viewer's value divided by ada's), and ada-optimization's Where ada stands, with ranks.

    python3 -I bench/b2/doctables.py [fm|stands|all]

Reads $B2_ROOT/b2/all.jsonl and, for print mode, $B2_ROOT/b2a/*.json and bytes.txt. Resize is the first (cold)
resize of each run, unlike report.py's median over all ten. The best value of each column is bold. Cells that the
scripted runs can't measure (md-tui on XL, mdfried's XL search) come out empty; the notes fill them from separate
runs, with a footnote."""
import json, math, os, sys, statistics as st
from collections import defaultdict

S = os.environ["B2_ROOT"]
V = ["ada", "treemd", "treemd-head", "leaf", "mdfried", "mdtui", "glow"]
NAME = {"ada": "**ada**", "treemd": "treemd 0.9.1", "treemd-head": "treemd HEAD", "leaf": "leaf", "mdfried": "mdfried",
        "mdtui": "md-tui", "glow": "glow"}
DOCS = ["s", "m", "l", "xl"]
runs = defaultdict(list)
for line in open(f"{S}/b2/all.jsonl"):
    r = json.loads(line)
    v, d = r["name"].rsplit("-", 1)
    runs[(v, d)].append(r)


def num(x): return isinstance(x, (int, float)) and not isinstance(x, bool)


def med(xs):
    xs = [x for x in xs if num(x)]
    return st.median(xs) if xs else None


def pr(r, kind, key, only_changed=False, all_shown=False):
    vals = r.get(f"{kind}_{key}") or []
    if only_changed:
        vals = [v for v, c in zip(vals, r.get(f"{kind}_changed") or []) if c]
    if all_shown and any(x is None for x in vals):
        return None  # a miss disqualifies the run
    return med(vals)


def cold(r, key):
    ch = r.get("resize_changed") or []
    vals = r.get(f"resize_{key}") or []
    return vals[0] if ch and ch[0] and vals else None


# facet -> (per-run value, unit); the per-cell value is the median across runs
F = {
    "first": lambda r: r.get("first_ms"), "settled": lambda r: r.get("settle_ms"),
    "startup CPU": lambda r: r.get("paint_cpu_ms"),
    "rss": lambda r: (r.get("hwm_kib") or 0) / 1024 or None, "rss paint": lambda r: (r.get("hwm_paint_kib") or 0) / 1024 or None,
    "idle CPU": lambda r: r.get("idle_cpu_ms"), "wakeups": lambda r: r.get("idle_ctxsw_s"),
    "line": lambda r: pr(r, "line", "done", True), "line CPU": lambda r: pr(r, "line", "cpu", True),
    "page": lambda r: pr(r, "page", "done", True), "page CPU": lambda r: pr(r, "page", "cpu", True),
    "page bytes": lambda r: pr(r, "page", "bytes", True),
    "end": lambda r: pr(r, "end", "shown", all_shown=True), "end CPU": lambda r: pr(r, "end", "cpu"),
    "search": lambda r: pr(r, "search", "shown", all_shown=True), "search CPU": lambda r: pr(r, "search", "cpu"),
    "resize": lambda r: cold(r, "done"), "resize CPU": lambda r: cold(r, "cpu"),
    "reload": lambda r: pr(r, "reload", "shown", all_shown=True), "reload CPU": lambda r: pr(r, "reload", "cpu"),
    "lag": lambda r: r.get("burst_lag_ms"), "burst CPU": lambda r: r.get("burst_cpu_ms"),
    "session CPU": lambda r: r.get("total_cpu_ms"), "session wall": lambda r: r.get("total_wall_ms"),
}


def excluded(v, d, facet):
    if d == "xl" and (v.startswith("treemd") or v == "mdtui"):
        return True  # treemd shows only the first `#` section of XL; md-tui never painted it
    if v == "glow" and facet.split()[0] in ("search", "resize"):
        return True  # glow has no search, and its screen goes blank after a resize
    return False


def val(v, d, facet):
    if excluded(v, d, facet):
        return None
    if facet in ("end CPU", "search CPU", "reload CPU") and val(v, d, facet.split()[0]) is None:
        return None  # the sentinel never showed: the CPU measures something else
    return med([F[facet](r) for r in runs.get((v, d), [])])


def fmt(x, sig=3):
    """Three significant digits, thousands separated; no exponent."""
    if x is None:
        return ""
    if x == 0:
        return "0"
    if abs(x) >= 10 ** (sig - 1):
        return f"{x:,.0f}"
    return f"{x:.{sig - 1 - math.floor(math.log10(abs(x)))}f}"


def table(header, rows):
    return "\n".join(["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
                     + ["| " + " | ".join(r) + " |" for r in rows])


def matrix(columns, viewers=V, times=True, bold=True):
    """columns: [(header, doc, facet, scale)]. One row per viewer; a × column after each value column when `times`,
    the viewer's value over ada's. The best value in each column is bold."""
    cells = {(v, i): (lambda x: x * c[3] if x is not None else None)(val(v, c[1], c[2]))
             for v in viewers for i, c in enumerate(columns)}
    best = {i: min([x for (v, j), x in cells.items() if j == i and x is not None], default=None)
            for i in range(len(columns))}
    rows = []
    for v in viewers:
        row = [NAME[v]]
        for i, c in enumerate(columns):
            x = cells[(v, i)]
            s = fmt(x)
            row.append(f"**{s}**" if bold and x is not None and x == best[i] else s)
            if times:
                a = cells[("ada", i)]
                row.append("" if v == "ada" or x is None or not a else fmt(x / a))
        rows.append(row)
    header = ["Viewer"] + [h for c in columns for h in ([c[0], f"{c[0]} ×"] if times else [c[0]])]
    return table(header, rows)


def fm():
    U = lambda d: d.upper()
    print("#### Startup (ms)\n")
    print(matrix([(f"{k} {U(d)}", d, f, 1) for k, f in (("First", "first"), ("Settled", "settled"),
                                                         ("CPU", "startup CPU")) for d in DOCS]))
    print("\n#### Memory and idle\n")
    cols = [(f"RSS {U(d)}", d, "rss", 1) for d in DOCS]
    cols += [c for d in ("l", "xl") for c in ((f"Idle CPU {U(d)}", d, "idle CPU", 1), (f"Wakeups {U(d)}", d, "wakeups", 1))]
    print(matrix(cols, times=False))
    print("\nPeak RSS at first paint (MiB):\n")
    print(matrix([(f"RSS {U(d)}", d, "rss paint", 1) for d in DOCS], times=False))
    for d in DOCS:
        print(f"\n#### Interaction {U(d)} (ms)\n")
        cols = [(h, d, f, 1) for k in ("line", "page", "end", "search", "resize", "reload")
                for h, f in ((k.capitalize(), k), (f"{k.capitalize()} CPU", f"{k} CPU"))]
        print(matrix(cols, viewers=[v for v in V if not excluded(v, d, "line")]))
    print("\n#### Held page-down (ms)\n")
    print(matrix([(f"Lag {U(d)}", d, "lag", 1) for d in DOCS], times=False, bold=False))
    print()
    print(matrix([(f"CPU {U(d)}", d, "burst CPU", 1) for d in DOCS]))
    print("\n#### Terminal bandwidth and whole-session CPU\n")
    rows = []
    for v in V:
        row = [NAME[v]] + [fmt((val(v, d, "page bytes") or 0) / 1000 or None) for d in ("s", "l")]
        for d in ("l", "xl"):
            c, w = val(v, d, "session CPU"), val(v, d, "session wall")
            row += ["", "", ""] if c is None or w is None else [fmt(c / 1000), fmt(w / 1000), fmt(100 * c / w)]
        rows.append(row)
    print(table(["Viewer", "Page S (KB)", "Page L (KB)", "Session L CPU (s)", "Session L wall (s)", "Session L CPU %",
                 "Session XL CPU (s)", "Session XL wall (s)", "Session XL CPU %"], rows))
    printmode()


def b2a():
    nbytes, res = {}, {}
    for line in open(f"{S}/b2a/bytes.txt"):
        d, n, b = line.split()
        nbytes[(d, n)] = int(b.split("=")[1])
    for d in DOCS:
        for r in json.load(open(f"{S}/b2a/{d}.json"))["results"]:
            m = r["measurements"]
            g = lambda k: st.median(x[k]["value"] for x in m)
            res[(d, r["name"])] = dict(n=len(m), wall=g("time_wall_clock") * 1e3, cpu=g("time_cpu") * 1e3,
                                       rss=g("memory_peak_resident") / 2**20, ins=g("instructions") / 1e6,
                                       out=nbytes[(d, r["name"])] / 1000 if (d, r["name"]) in nbytes else None)
    return res


PRINTERS = ["ada", "mdcat", "leaf", "glow", "lowdown"]


def printmode():
    res = b2a()
    printers = [n for n in PRINTERS if (DOCS[0], n) in res]
    hl = [n for n in printers if n != "lowdown"]  # lowdown doesn't highlight: never bold
    print("\n#### Print to a pipe (ms)\n")
    rows = []
    for n in printers:
        row = [n]
        for d in DOCS:
            for k in ("wall", "cpu"):
                x, a = res[(d, n)][k], res[(d, "ada")][k]
                s = fmt(x)
                row += [f"**{s}**" if n in hl and x == min(res[(d, m)][k] for m in hl) else s,
                        "" if n == "ada" else fmt(x / a)]
        rows.append(row)
    print(table(["Viewer"] + [h for d in DOCS for k in ("Wall", "CPU") for h in (f"{k} {d.upper()}", f"{k} {d.upper()} ×")], rows))
    print("\nPeak RSS (MiB), output (KB), instructions (M):\n")
    rows = []
    for n in printers:
        row = [n]
        for k in ("rss", "out", "ins"):
            for d in DOCS:
                x = res[(d, n)][k]
                s = fmt(x)
                lo = min((y for m in hl if (y := res[(d, m)][k]) is not None), default=None)
                row.append(f"**{s}**" if n in hl and x is not None and x == lo else s)
        rows.append(row)
    print(table(["Viewer"] + [f"{h} {d.upper()}" for h in ("RSS", "Output", "Instructions") for d in DOCS], rows))
    print("\nruns:", {f"{d}/{n}": r["n"] for (d, n), r in res.items() if n == "ada"})


# ada-optimization's Where ada stands: (label, wall facet, CPU facet); held page-down is ranked on CPU
STANDS = [("first paint", "first", "startup CPU"), ("line scroll", "line", "line CPU"), ("page scroll", "page", "page CPU"),
          ("jump to end", "end", "end CPU"), ("search", "search", "search CPU"), ("resize, cold", "resize", "resize CPU"),
          ("live reload", "reload", "reload CPU"), ("held page-down", "lag", "burst CPU")]
# every ranked cell: 21 facets (settled and bytes per page too) × 4 documents
RANKED = ["first", "settled", "startup CPU", "rss", "idle CPU", "wakeups", "line", "line CPU", "page", "page CPU",
          "page bytes", "end", "end CPU", "search", "search CPU", "resize", "resize CPU", "reload", "reload CPU",
          "lag", "burst CPU"]


def rank(d, facet):
    vals = {v: x for v in V if (x := val(v, d, facet)) is not None}
    a = vals.get("ada")
    if a is None:
        return None, []
    better = sorted((x, v) for v, x in vals.items() if v != "ada" and x < a)
    return 1 + len(better), better


def stands():
    rows = []
    for label, wf, cf in STANDS:
        for d in DOCS:
            others = {v: (val(v, d, wf), val(v, d, cf)) for v in V if v != "ada"}
            key = 1 if wf == "lag" else 0
            near = min((v for v in others if others[v][key] is not None), key=lambda v: others[v][key])
            aw, ac = val("ada", d, wf), val("ada", d, cf)
            nw, nc = others[near]
            rows.append([label, d.upper(), fmt(aw), fmt(ac), str(rank(d, wf)[0]), str(rank(d, cf)[0]), NAME[near],
                         fmt(nw), fmt(nc), "–" if wf == "lag" or not aw else fmt(nw / aw), fmt(nc / ac)])
    print("#### Time per action\n")
    print(table(["facet", "doc", "ada ms", "ada CPU ms", "wall rank", "CPU rank", "nearest other", "its ms",
                 "its CPU ms", "ada × wall", "ada × CPU"], rows))
    rows = []
    for label, f, scale in (("peak RSS (MiB)", "rss", 1), ("idle CPU (ms in 5 s)", "idle CPU", 1),
                            ("idle wakeups (a second)", "wakeups", 1), ("bytes per page (KB)", "page bytes", 1e-3)):
        for d in DOCS:
            others = {v: x for v in V if v != "ada" and (x := val(v, d, f)) is not None}
            near = min(others, key=others.get)
            rows.append([label, d.upper(), fmt(val("ada", d, f) * scale), str(rank(d, f)[0]), NAME[near],
                         fmt(others[near] * scale)])
    print("\n#### Resources\n")
    print(table(["measure", "doc", "ada", "rank", "nearest other", "its value"], rows))
    res = b2a()
    rows = []
    for d in DOCS:
        hl = ["ada", "mdcat", "leaf"]  # the highlighting printers measured together; glow is far behind
        a = res[(d, "ada")]
        near = min((n for n in hl if n != "ada"), key=lambda n: res[(d, n)]["wall"])
        o = res[(d, near)]
        rows.append([d.upper(), fmt(a["wall"]), fmt(a["cpu"]), str(1 + sum(res[(d, n)]["wall"] < a["wall"] for n in hl)),
                     fmt(a["rss"]), str(1 + sum(res[(d, n)]["rss"] < a["rss"] for n in hl)), near, fmt(o["wall"]),
                     fmt(o["cpu"]), fmt(o["rss"])])
    print("\n#### Print to a pipe\n")
    print(table(["doc", "ada ms", "ada CPU ms", "wall rank", "ada MiB", "RSS rank", "nearest other", "its ms",
                 "its CPU ms", "its MiB"], rows))
    print("\n#### Not first\n")
    first = total = 0
    for f in RANKED:
        for d in DOCS:
            k, better = rank(d, f)
            if k is None:
                continue
            total += 1
            first += k == 1
            if k != 1:
                print(f"- {f} {d.upper()}: rank {k}; " + ", ".join(f"{NAME[v]} {fmt(x)}" for x, v in better)
                      + f" against ada's {fmt(val('ada', d, f))}")
    print(f"\nada first in {first} of {total}")


if __name__ == "__main__":
    sec = sys.argv[1] if len(sys.argv) > 1 else "all"
    if sec in ("fm", "all"):
        fm()
    if sec in ("stands", "all"):
        print()
        stands()
