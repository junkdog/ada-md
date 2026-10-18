"""The data behind the README's benchmark charts: bench/b2/plots/{interactive,print}.dat, for plots/b2.gnuplot.

    python3 -I bench/b2/plot.py COMMIT && gnuplot bench/b2/plots/b2.gnuplot

Reads the same records as doctables.py ($B2_ROOT/b2/all.jsonl, $B2_ROOT/b2a/) and takes its cell values, so a
chart shows exactly what the vault tables do. One block per panel (gnuplot's `index`), a row per document, a
column per viewer; `?` is a cell the run couldn't measure."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from doctables import DOCS, b2a, val

OUT = os.path.dirname(os.path.abspath(__file__)) + "/plots"
commit = sys.argv[1] if len(sys.argv) > 1 else "?"
# treemd HEAD rather than 0.9.1: it is the faster of the two, and 0.9.1's search lands on the wrong row
VIEWERS = ["ada", "leaf", "mdfried", "mdtui", "treemd-head", "glow"]
PANELS = [("first paint", "first", 1), ("CPU per line scroll", "line CPU", 1), ("search", "search", 1),
          ("resize, cold", "resize", 1), ("held page-down CPU", "burst CPU", 1), ("peak RSS", "rss", 1)]
PRINTERS = ["ada", "leaf", "mdcat", "rich", "mcat", "glow", "bat", "lowdown"]


def cell(x):
    return "?" if x is None else f"{x:.6g}"


def block(title, rows, names):
    out = [f"# {title}", "# doc " + " ".join(names)]
    out += [" ".join(r) for r in rows]
    return "\n".join(out)


blocks = []
for title, facet, scale in PANELS:
    rows = [[d.upper()] + [cell(None if (x := val(v, d, facet)) is None else x * scale) for v in VIEWERS] for d in DOCS]
    blocks.append(block(title, rows, VIEWERS))
with open(f"{OUT}/interactive.dat", "w") as f:
    f.write(f"# B2 at ada {commit}: medians, ms (peak RSS in MiB); written by bench/b2/plot.py\n")
    f.write("\n\n\n".join(blocks) + "\n")

res = b2a()
blocks = []
for title, key in (("print to a pipe, wall", "wall"), ("print to a pipe, peak RSS", "rss")):
    rows = [[d.upper()] + [cell(res[(d, n)][key] if (d, n) in res else None) for n in PRINTERS] for d in DOCS]
    blocks.append(block(title, rows, PRINTERS))
with open(f"{OUT}/print.dat", "w") as f:
    f.write(f"# B2 print mode at ada {commit}: hyperfine medians, ms (peak RSS in MiB); written by bench/b2/plot.py\n")
    f.write("\n\n\n".join(blocks) + "\n")
print(f"wrote {OUT}/interactive.dat and {OUT}/print.dat")
