"""The data behind the benchmark charts: bench/b2/plots/{interactive,print}.dat, for plots/b2.gnuplot and the
site, and previewer.dat, images.dat and mermaid.dat (bench/diagrams/compare's, stamped), for the site only.

    python3 -I bench/b2/plot.py COMMIT [PREVIEWER [IMAGES]] && gnuplot bench/b2/plots/b2.gnuplot

Reads the same records as doctables.py ($B2_ROOT/b2/all.jsonl, $B2_ROOT/b2a/, and $B2_ROOT/b2p/ and b2i/, merged
with $B2P_DIRS and $B2I_DIRS) and takes its cell values, so a chart shows exactly what the vault tables do. One block per panel
(gnuplot's `index`), a row per document or file, a column per viewer; `?` is a cell the run couldn't measure,
`cap` a run b2p.zsh stopped at its cap. interactive.dat's header also counts how often ada is first over every
ranked measurement ("ada first in 79 of 84"), which site/perf.py puts in the hero. PREVIEWER stamps previewer.dat (default COMMIT), for a rerun of some files
at another commit merged over the full run: "137570c, logs e9ec3b7"; IMAGES stamps images.dat the same way. The records of an ada-only rerun (rerun.zsh, whose
OUT holds ada-ref) carry the other tools' cells from the full run under it: B2_OTHERS="COMMIT DATE" names that run
for the site's stamp, and is required there."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from doctables import DOCS, IMAGE_FLAGS, IMAGES, PREV_FILES, RANKED, S, b2a, loader, rank, val

OUT = os.path.dirname(os.path.abspath(__file__)) + "/plots"
commit = sys.argv[1] if len(sys.argv) > 1 else "?"
prev_stamp = sys.argv[2] if len(sys.argv) > 2 else commit
img_stamp = sys.argv[3] if len(sys.argv) > 3 else commit
others = os.environ.get("B2_OTHERS", "")
if os.path.exists(f"{S}/ada-ref") and len(others.split()) != 2:
    sys.exit("plot: these are an ada-only rerun's records: set B2_OTHERS='COMMIT DATE', the full run the other "
             "tools' cells are from")
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
# how often ada is first, over every ranked measurement (doctables.py's "Not first"), not only the charted ones:
# the README's count and the site's hero
ranks = [k for f in RANKED for d in DOCS if (k := rank(d, f)[0]) is not None]
with open(f"{OUT}/interactive.dat", "w") as f:
    f.write(f"# B2 at ada {commit}: medians, ms (peak RSS in MiB); ada first in {ranks.count(1)} of {len(ranks)} "
            f"ranked measurements; {f'the other tools at ada {others}; ' if others else ''}written by "
            f"bench/b2/plot.py\n")
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

if os.path.exists(f"{S}/b2p"):
    load = loader("b2p")
    tools = {"preview": ["ada", "bat", "rich"], "whole": ["ada", "bat", "rich", "mcat"]}
    blocks = []
    for key, unit in (("wall", "wall"), ("rss", "peak RSS")):
        for mode, label in (("preview", "a preview"), ("whole", "the whole file")):
            res = {f: load(f, mode) for f in PREV_FILES}
            rows = [[f] + ["?" if n not in res[f] else "cap" if res[f][n]["killed"] else cell(res[f][n][key])
                           for n in tools[mode]] for f in PREV_FILES]
            blocks.append(block(f"previewer, {label}, {unit}", rows, tools[mode]))
    with open(f"{OUT}/previewer.dat", "w") as f:
        f.write(f"# B2 previewer files at ada {prev_stamp}: hyperfine medians, ms (peak RSS in MiB); cap at "
                f"{os.environ.get('B2P_CAP', 60)} s; written by bench/b2/plot.py\n")
        f.write("\n\n\n".join(blocks) + "\n")
    print(f"wrote {OUT}/previewer.dat")

if os.path.exists(f"{S}/b2i"):
    load = loader("b2i")
    tools = {"kitty": ["ada", "icat", "icat-shm", "chafa", "mcat"], "text": ["ada", "viu", "chafa", "chafa-vhalf"]}
    blocks = []
    for key, unit in (("wall", "wall"), ("cpu", "CPU"), ("rss", "peak RSS")):
        for mode, label in (("kitty", "kitty graphics"), ("text", "text")):
            res = {f: load(f, mode) for f in IMAGES}
            # a flagged cell (IMAGE_FLAGS) drew nothing: no bar, the site says why
            rows = [[f] + ["?" if n not in res[f] or res[f][n]["killed"] or (f, n) in IMAGE_FLAGS
                           else cell(res[f][n][key]) for n in tools[mode]] for f in IMAGES]
            blocks.append(block(f"image, {label}, {unit}", rows, tools[mode]))
    with open(f"{OUT}/images.dat", "w") as f:
        f.write(f"# B2 image previews at ada {img_stamp}: hyperfine medians, ms (CPU: user + system; peak RSS in MiB); "
                f"written by bench/b2/plot.py\n")
        f.write("\n\n\n".join(blocks) + "\n")
    print(f"wrote {OUT}/images.dat")

if os.path.exists(f"{S}/mermaid.dat"):
    lines = open(f"{S}/mermaid.dat").read().splitlines(True)
    lines[0] = f"# B2 mermaid at ada {commit}:" + lines[0].split(":", 1)[1]   # perf.py skips a "# B2" header
    with open(f"{OUT}/mermaid.dat", "w") as f:
        f.write("".join(lines))
    print(f"wrote {OUT}/mermaid.dat")
