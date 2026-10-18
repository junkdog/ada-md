"""Two B2 runs side by side: every ranked cell of doctables.py, old against new, for spotting regressions.

    python3 -I bench/b2/runcmp.py OLD.jsonl NEW.jsonl [VIEWER] [THRESHOLD]

Prints VIEWER's cells (ada by default) whose new/old ratio is outside 1 ± THRESHOLD (0.10), beside how far the
other viewers moved on the same facet and document: when they all moved too, so did the machine (lanes, load,
kernel), not the viewer. Then each other viewer's median ratio over all its cells, the run-to-run drift."""
import json, os, statistics as st, sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import doctables as dt


def load(path):
    runs = defaultdict(list)
    for line in open(path):
        r = json.loads(line)
        v, d = r["name"].rsplit("-", 1)
        runs[(v, d)].append(r)
    return runs


def cells(runs):
    dt.runs = runs
    return {(v, d, f): x for v in dt.V for d in dt.DOCS for f in dt.RANKED if (x := dt.val(v, d, f)) is not None}


old, new = cells(load(sys.argv[1])), cells(load(sys.argv[2]))
me = sys.argv[3] if len(sys.argv) > 3 else "ada"
th = float(sys.argv[4]) if len(sys.argv) > 4 else 0.10
ratio = {k: new[k] / old[k] for k in old.keys() & new.keys() if old[k]}
others = lambda d, f: [r for (v, dd, ff), r in ratio.items() if v != me and dd == d and ff == f]
print(f"| facet | doc | old | new | new/old | others' new/old (median, range) |\n|---|---|---|---|---|---|")
for (v, d, f), r in sorted(ratio.items(), key=lambda kv: -kv[1]):
    if v != me or abs(r - 1) <= th:
        continue
    o = others(d, f)
    ctx = f"{st.median(o):.2f} ({min(o):.2f}–{max(o):.2f})" if o else "–"
    print(f"| {f} | {d.upper()} | {dt.fmt(old[(v, d, f)])} | {dt.fmt(new[(v, d, f)])} | {r:.2f} | {ctx} |")
print("\ndrift, each viewer's median new/old over its cells:")
for v in dt.V:
    rs = [r for (vv, _, _), r in ratio.items() if vv == v]
    if rs:
        print(f"  {v}: {st.median(rs):.3f} over {len(rs)} cells (p10 {st.quantiles(rs, n=10)[0]:.2f}, "
              f"p90 {st.quantiles(rs, n=10)[-1]:.2f})")
