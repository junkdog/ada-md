"""The site's performance section, from the B2 data behind the README's charts: bench/b2/plots/{interactive,print}.dat.

    python3 -I site/perf.py          # rewrites site/index.html between <!-- perf:begin --> and <!-- perf:end -->
    python3 -I site/perf.py --check  # exits 1 if index.html is out of date

Writes headline tiles for one document, then CSS-only tabs (radio inputs, no script) with a small bar chart per
metric for each document. Bars are on a log scale and every bar carries its value; a viewer the run couldn't
measure gets the reason instead of a bar."""
import math, os, re, subprocess, sys
from html import escape

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLOTS = f"{ROOT}/bench/b2/plots"
PAGE = f"{ROOT}/site/index.html"

DOCS = {"S": "13 KB, the median note", "M": "86 KB, table-heavy", "L": "238 KB, the largest note",
        "XL": "2 MB, notes concatenated"}
TILE_DOC = "L"
NAMES = {"mdtui": "md-tui", "treemd-head": "treemd"}
UNITS = {"peak RSS": "MiB", "print to a pipe, peak RSS": "MiB"}
CAPTIONS = {"first paint": "first paint", "CPU per line scroll": "CPU per line scrolled", "search": "search",
            "resize, cold": "re-layout after a resize", "held page-down CPU": "CPU over a held page-down",
            "peak RSS": "peak memory", "print to a pipe, wall": "print to a pipe",
            "print to a pipe, peak RSS": "peak memory, printing"}
# why a cell is missing, as the README explains it: (metric or None for any, viewer, doc or None for any)
MISSING = [("search", "glow", None, "has no search"),
           ("resize, cold", "glow", None, "blank after a resize"),
           (None, "treemd-head", "XL", "shows one section at a time"),
           (None, "mdtui", "XL", "never finished parsing"),
           ("resize, cold", "mdtui", None, "re-parses for seconds"),
           ("search", "mdfried", "XL", "match off-screen")]
NOTES = {"lowdown": "no highlighting", "bat": "source, not rendered"}
# what each chart times, under its caption, and its kind of time: the legend above the charts says what
# wall and CPU mean; memory has neither
DESCS = {"first paint": "from launch until the document's first line is on screen",
         "CPU per line scroll": "for one line down, from the key until the screen holds still; the median",
         "search": "from the term and Enter until the match is on screen",
         "resize, cold": "from the first resize until the screen stops changing",
         "held page-down CPU": "for 60 page-downs at 30 a second, until the screen catches up",
         "peak RSS": "the most resident at once, over the whole session",
         "print to a pipe, wall": "to print the whole document to a pipe",
         "print to a pipe, peak RSS": "the most resident at once while printing"}
KINDS = {"first paint": "wall", "CPU per line scroll": "CPU", "search": "wall", "resize, cold": "wall",
         "held page-down CPU": "CPU", "print to a pipe, wall": "wall"}
TILES = [("first paint", "first paint"), ("CPU per line scroll", "CPU per line scrolled"),
         ("resize, cold", "re-layout after a resize"), ("peak RSS", "peak memory")]


def parse(path):
    """{title: {doc: {viewer: value or None}}} and the commit from the header."""
    text = open(path).read()
    commit = re.search(r"at ada (\w+)", text).group(1)
    panels = {}
    for block in text.strip().split("\n\n\n"):
        lines = block.strip().splitlines()
        if lines[0].startswith("# B2"):
            lines = lines[1:]
        title, names = lines[0][2:], lines[1].split()[2:]
        panels[title] = {r.split()[0]: {n: None if v == "?" else float(v) for n, v in zip(names, r.split()[1:])}
                         for r in lines[2:]}
    return panels, commit


def num(v):
    return f"{v:.2f}" if v < 1 else f"{v:.1f}" if v < 100 else f"{v:,.0f}"


def why(metric, viewer, doc):
    return next((r for m, v, d, r in MISSING if m in (None, metric) and v == viewer and d in (None, doc)), "not measured")


def kind(metric):
    k = KINDS.get(metric)
    return f' <em class="kind {k.lower()}">{k}</em>' if k else ""


def chart(metric, cells, doc):
    unit = UNITS.get(metric, "ms")
    vals = [v for v in cells.values() if v is not None]
    lo, hi = min(vals) / 2.5, max(vals)
    ada = cells["ada"]
    rows = sorted(cells.items(), key=lambda kv: (kv[1] is None, kv[1] or 0))
    out = [f'<figure class="chart"><figcaption>{escape(CAPTIONS[metric])} <span>{unit}</span>{kind(metric)}'
           f'<small>{escape(DESCS[metric])}</small></figcaption><ol>']
    for viewer, v in rows:
        name = escape(NAMES.get(viewer, viewer))
        if viewer in NOTES:
            name += f' <small>{escape(NOTES[viewer])}</small>'
        cls = ' class="ada"' if viewer == "ada" else ""
        if v is None:
            out.append(f'<li class="missing"><span class="name">{name}</span>'
                       f'<span class="why">{escape(why(metric, viewer, doc))}</span></li>')
            continue
        w = 100 * math.log(v / lo) / math.log(hi / lo)
        ratio = "" if viewer == "ada" else f' <small>{v / ada:.1f}×</small>' if v / ada < 10 else f' <small>{v / ada:,.0f}×</small>'
        out.append(f'<li{cls} style="--w:{w:.1f}%"><span class="name">{name}</span><span class="bar"></span>'
                   f'<span class="val">{num(v)}{ratio}</span></li>')
    out.append("</ol></figure>")
    return "\n".join(out)


def tiles(panels):
    out = ['<div class="tiles">']
    for metric, label in TILES:
        cells = panels[metric][TILE_DOC]
        unit = UNITS.get(metric, "ms")
        best = min((v, k) for k, v in cells.items() if k != "ada" and v is not None)
        out.append(f'<div class="tile"><b>{num(cells["ada"])} <span>{unit}</span></b><p>{escape(label)}{kind(metric)}</p>'
                   f'<small>next: {escape(NAMES.get(best[1], best[1]))}, {num(best[0])} {unit}'
                   f' ({best[0] / cells["ada"]:.1f}×)</small></div>')
    out.append("</div>")
    return "\n".join(out)


def section():
    inter, commit = parse(f"{PLOTS}/interactive.dat")
    prints, _ = parse(f"{PLOTS}/print.dat")
    date = subprocess.run(["git", "-C", ROOT, "log", "-1", "--format=%cs", "--", "bench/b2/plots/interactive.dat"],
                          capture_output=True, text=True).stdout.strip() or "?"
    out = [f'<p class="lede">Measured on the {DOCS[TILE_DOC].split(",")[0]} note, the largest in the vault:</p>',
           tiles(inter), '<div class="doc-tabs" role="radiogroup" aria-label="document">']
    for doc, desc in DOCS.items():
        checked = " checked" if doc == TILE_DOC else ""
        out.append(f'<input type="radio" name="doc" id="doc-{doc.lower()}"{checked}>'
                   f'<label for="doc-{doc.lower()}"><b>{doc}</b> {escape(desc)}</label>')
    out.append("</div>")
    for doc in DOCS:
        out.append(f'<div class="perf-panel" data-doc="{doc.lower()}"><h3>In a 120×40 terminal</h3><div class="charts">')
        out += [chart(m, inter[m][doc], doc) for m in inter]
        out.append('</div><h3>Printing, <code>--print</code> and the other printers</h3><div class="charts">')
        out += [chart(m, prints[m][doc], doc) for m in prints]
        out.append("</div></div>")
    out.append(f'<p class="stamp">B2 at ada <code>{commit}</code>, {date}</p>')
    return "\n".join(out)


page = open(PAGE).read()
new = re.sub(r"(<!-- perf:begin -->\n).*?(<!-- perf:end -->)", lambda m: m[1] + section() + "\n" + m[2], page,
             flags=re.S)
if "--check" in sys.argv:
    sys.exit(0 if new == page else 1)
open(PAGE, "w").write(new)
