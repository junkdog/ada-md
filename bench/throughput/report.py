#!/usr/bin/env python3
"""report.py OUT [OUT2]: the vault's ada-throughput tables from a run.zsh record set (BEN-009).

MB/s are source bytes (10^6) a second of the measuring thread's CPU time; cyc/B and ins/B are the core's
counters per source byte. A stage's share of a ceiling is its MB/s over the ceiling's. With OUT2, a run of
the same build, it prints how far each cell moved between the two instead (the first run's review: are the
medians stable).
"""
import json
import os
import sys

STAGES = {"whole": ("parse", "layout", "print"), "syntax": ("parse", "layout", "print"), "lexer": ("tokens", "highlight")}
C1 = {"parse": "c1-parse", "layout": "c1-layout", "print": "c1-print", "tokens": "c1-lex", "highlight": "c1-lex"}


def sig(x, n=3):
    """n significant figures, as the vault's tables give them, with thousands separated."""
    if x == 0:
        return "0"
    digits = max(0, n - 1 - int(f"{abs(x):e}".split("e")[1]))
    return f"{x:,.{digits}f}"


def load(out):
    cells, meta = {}, {}
    with open(os.path.join(out, "cells.jsonl")) as f:
        for line in f:
            c = json.loads(line)
            if "meta" in c:
                meta = c["meta"]
                continue
            cells[(c["input"], c["stage"])] = c
    return cells, meta


def mbs(c):
    return c["bytes"] / c["cpu_ns"] * 1e3


def inputs(cells, group):
    seen = []
    for (i, _), c in cells.items():
        if c["group"] == group and i not in seen:
            seen.append(i)
    return seen


def row(*cols):
    print("| " + " | ".join(str(c).replace("|", "\\|") for c in cols) + " |")


def stages(cells, group):
    lexer = group == "lexer"
    ref = "syntect" if lexer else "c2"
    head = ["input", "stage", "MB/s", "cyc/B", "ins/B", "IPC", "C1 MB/s", "of C1", "syntect MB/s" if lexer else "C2 MB/s", "× syntect" if lexer else "of C2"]
    row(*head)
    row(*["---"] * len(head))
    for i in inputs(cells, group):
        for s in STAGES[group]:
            c = cells.get((i, s))
            if not c:
                continue
            b = c["bytes"]
            c1, r = cells.get((i, C1[s])), cells.get((i, ref))
            # C2 bounds parse only; syntect stands beside both lexer stages
            r = r if (lexer or s == "parse") else None
            share = (lambda x: f"{mbs(c) / mbs(x):.0%}") if not lexer else None
            row(i, s, sig(mbs(c)), f"{c['cycles'] / b:.2f}", f"{c['instructions'] / b:.2f}", f"{c['instructions'] / c['cycles']:.2f}",
                sig(mbs(c1)) if c1 else "", f"{mbs(c) / mbs(c1):.1%}" if c1 else "",
                sig(mbs(r)) if r else "",
                (f"{mbs(c) / mbs(r):.1f}×" if lexer else share(r)) if r else "")
    print()


def ceilings(cells):
    """The ceilings, and pulldown-cmark's HTML renderer beside them: a reference, not a ceiling."""
    head = ["input", "bytes", "C0 read", "C1 parse / lex", "C1 layout", "C1 print", "C2 events", "HTML ref", "print out bytes"]
    row(*head)
    row(*["---"] * len(head))
    for group in ("whole", "syntax", "lexer"):
        for i in inputs(cells, group):
            get = lambda s: sig(mbs(cells[(i, s)])) if (i, s) in cells else ""
            c0 = cells[(i, "c0")]
            name = f"{i} ({c0['notes']} notes)" if "notes" in c0 else i
            out = cells.get((i, "print"), {}).get("out_bytes")
            row(name, f"{c0['bytes']:,}", get("c0"), get("c1-parse") or get("c1-lex"), get("c1-layout"), get("c1-print"), get("c2"),
                get("html"), f"{out:,}" if out else "")
    print()


def e2e(out, cells):
    files = sorted(f for f in os.listdir(out) if f.startswith("e2e.") and f.endswith(".json"))
    if not files:
        return
    head = ["input", "wall ms", "CPU ms", "MB/s wall", "MB/s CPU", "cyc/B", "ins/B", "peak RSS MiB"]
    row(*head)
    row(*["---"] * len(head))
    # the inputs' sizes, as run.zsh found them; a record set from before e2e.bytes has its cells' only
    sizes = {}
    if os.path.exists(os.path.join(out, "e2e.bytes")):
        sizes = dict(line.split() for line in open(os.path.join(out, "e2e.bytes")))
    order = list(sizes) or [f[4:-5] for f in files]
    for i in order:
        f = f"e2e.{i}.json"
        if f not in files:
            continue
        s = json.load(open(os.path.join(out, f)))["results"][0]["summary"]
        b = int(sizes[i]) if i in sizes else next(c["bytes"] for (j, _), c in cells.items() if j == i)
        wall, cpu = s["time_wall_clock"]["median"], s["time_cpu"]["median"]
        row(i, sig(wall * 1e3), sig(cpu * 1e3), sig(b / wall / 1e6), sig(b / cpu / 1e6),
            f"{s['cpu_cycles']['median'] / b:.2f}", f"{s['instructions']['median'] / b:.2f}",
            sig(s["memory_peak_resident"]["median"] / 2**20))
    print()


def compare(a, b):
    head = ["input", "stage", "CPU", "wall", "cycles", "instructions"]
    row(*head)
    row(*["---"] * len(head))
    worst = {k: (1, None) for k in ("cpu_ns", "wall_ns", "cycles", "instructions")}
    for key, x in a.items():
        y = b.get(key)
        if not y:
            continue
        ratios = {k: y[k] / x[k] for k in worst}
        for k, r in ratios.items():
            if abs(r - 1) > abs(worst[k][0] - 1):
                worst[k] = (r, key)
        row(*key, *(f"{ratios[k]:.3f}" for k in ("cpu_ns", "wall_ns", "cycles", "instructions")))
    print()
    for k, (r, key) in worst.items():
        print(f"- furthest {k}: {r:.3f} ({' '.join(key) if key else '-'})")


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    out = sys.argv[1]
    cells, meta = load(out)
    if len(sys.argv) == 3:
        compare(cells, load(sys.argv[2])[0])
        return
    print(open(os.path.join(out, "meta.txt")).read().rstrip())
    print(f"classifier {meta.get('classifier')}, {meta.get('samples')} samples of ≥ {meta.get('sample_ms')} ms, width {meta.get('width')}\n")
    for group, title in (("whole", "Whole documents"), ("syntax", "Constructs"), ("lexer", "Lexers")):
        if inputs(cells, group):
            print(f"### {title}\n")
            stages(cells, group)
    print("### Ceilings (MB/s of source)\n")
    ceilings(cells)
    print("### End to end\n")
    e2e(out, cells)


if __name__ == "__main__":
    main()
