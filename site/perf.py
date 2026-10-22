"""The site's performance section, from the B2 data behind the README's charts: bench/b2/plots/{interactive,print}.dat,
the previewer files and image previews: bench/b2/plots/{previewer,images}.dat (plot.py writes all four), and
mermaid diagrams: bench/b2/plots/mermaid.dat (bench/diagrams/compare, stamped by plot.py).

    python3 -I site/perf.py          # rewrites site/index.html between <!-- perf:begin --> and <!-- perf:end -->,
                                     # and the hero's line between <!-- proof:begin --> and <!-- proof:end -->
    python3 -I site/perf.py --check  # exits 1 if index.html is out of date

Writes headline tiles for one document, then CSS-only tabs (radio inputs, no script) with a small bar chart per
metric for each document, then the same for each previewer file and each image, then a chart per kind of mermaid
diagram. Bars are on a log scale and every bar carries its value; a viewer the run couldn't
measure gets the reason instead of a bar."""
import math, os, re, subprocess, sys
from html import escape

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLOTS = f"{ROOT}/bench/b2/plots"
PAGE = f"{ROOT}/site/index.html"

DOCS = {"S": "13 KB, the median note", "M": "86 KB, table-heavy", "L": "238 KB, the largest note",
        "XL": "2 MB, notes concatenated"}
TILE_DOC = "L"
NAMES = {"mdtui": "md-tui", "treemd-head": "treemd", "icat": "kitten icat", "icat-shm": "kitten icat"}
UNITS = {"peak RSS": "MiB", "print to a pipe, peak RSS": "MiB", "previewer, a preview, peak RSS": "MiB",
         "previewer, the whole file, peak RSS": "MiB", "image, kitty graphics, peak RSS": "MiB",
         "image, text, peak RSS": "MiB", "mermaid, sequence diagrams": "µs", "mermaid, flowcharts": "µs",
         "mermaid, state diagrams": "µs", "mermaid, gantt charts": "µs"}
# previewer.dat's and images.dat's rows: (tab label, description)
FILES = {"code-s.rs": ("6 KB", "Rust"), "code-m.rs": ("47 KB", "Rust"), "code-l.rs": ("305 KB", "Rust"),
         "log.log": ("1 MB", "journal log"), "huge.json": ("3.75 MB", "JSON, one line"),
         "huge.log": ("80 MB", "journal log")}
FILE_TAB = "huge.log"
IMAGES = {"fit.png": ("686×811", "PNG that fits as it is"), "diagram.png": ("2473×1808", "PNG diagram"),
          "photo-12mp.png": ("12 MP", "PNG photo"), "photo.jpg": ("1500×2000", "JPEG photo"),
          "photo-12mp.jpg": ("12 MP", "JPEG photo"), "photo-24mp.jpg": ("24 MP", "JPEG photo")}
IMAGE_TAB = "photo.jpg"  # an ordinary photo, not fit.png, ada's best case
CAPTIONS = {"first paint": "first paint", "CPU per line scroll": "CPU per line scrolled", "search": "search",
            "resize, cold": "re-layout after a resize", "held page-down CPU": "CPU over a held page-down",
            "peak RSS": "peak memory", "print to a pipe, wall": "print to a pipe",
            "print to a pipe, peak RSS": "peak memory, printing",
            "previewer, a preview, wall": "a preview", "previewer, the whole file, wall": "the whole file",
            "previewer, a preview, peak RSS": "peak memory, a preview",
            "previewer, the whole file, peak RSS": "peak memory, the whole file",
            "image, kitty graphics, wall": "kitty graphics", "image, text, wall": "half-blocks and symbols",
            "image, kitty graphics, CPU": "CPU, kitty graphics", "image, text, CPU": "CPU, half-blocks and symbols",
            "image, kitty graphics, peak RSS": "peak memory, kitty graphics",
            "image, text, peak RSS": "peak memory, half-blocks and symbols",
            "mermaid, sequence diagrams": "sequence diagrams", "mermaid, flowcharts": "flowcharts",
            "mermaid, state diagrams": "state diagrams", "mermaid, gantt charts": "gantt charts",
            "mermaid, a 120-node flowchart": "a 120-node flowchart"}
# why a cell is missing, as the README explains it: (metric or None for any, viewer, doc or None for any)
MISSING = [("search", "glow", None, "has no search"),
           ("resize, cold", "glow", None, "blank after a resize"),
           (None, "treemd-head", "XL", "shows one section at a time"),
           (None, "mdtui", "XL", "never finished parsing"),
           ("resize, cold", "mdtui", None, "re-parses for seconds"),
           ("search", "mdfried", "XL", "match off-screen"),
           ("mermaid, gantt charts", "mmdflux", None, "draws no gantt charts")]
NOTES = {"lowdown": "no highlighting", "bat": "source, not rendered"}
# notes on the previewer files and images, by viewer and, where it's only one file, (viewer, file)
PREV_NOTES = {("rich", "log.log"): "no color", ("rich", "huge.log"): "no color",
              # bat's printer skips highlighting a line over 16 KiB; ada colors it (O-48)
              ("bat", "huge.json"): "no color on lines over 16 KB",
              ("mcat", "code-l.rs"): "most of it as markdown"}
IMAGE_NOTES = {"icat": "streamed, as ada", "icat-shm": "shared memory", "chafa-vhalf": "half-blocks only",
               "mcat": None, "viu": None}
# what each chart times, under its caption, and its kind of time: the legend above the charts says what
# wall and CPU mean; memory has neither
DESCS = {"first paint": "from launch until the document's first line is on screen",
         "CPU per line scroll": "for one line down, from the key until the screen holds still; the median",
         "search": "from the term and Enter until the match is on screen",
         "resize, cold": "from the first resize until the screen stops changing",
         "held page-down CPU": "for 60 page-downs at 30 a second, until the screen catches up",
         "peak RSS": "the most resident at once, over the whole session",
         "print to a pipe, wall": "to print the whole document to a pipe",
         "print to a pipe, peak RSS": "the most resident at once while printing",
         "previewer, a preview, wall": "the first 2,000 rows (bat and rich: 2,000 source lines), to a pipe",
         "previewer, the whole file, wall": "all of it, to a pipe at 100 columns",
         "previewer, a preview, peak RSS": "the most resident at once, for a preview",
         "previewer, the whole file, peak RSS": "the most resident at once, for the whole file",
         "image, kitty graphics, wall": "a 100×50-cell preview in kitty's graphics protocol, to a pipe",
         "image, text, wall": "the same preview drawn in characters, for any terminal",
         "image, kitty graphics, CPU": "user and system time for the kitty preview, over all threads",
         "image, text, CPU": "user and system time for the preview in characters, over all threads",
         "image, kitty graphics, peak RSS": "the most resident at once, kitty graphics",
         "image, text, peak RSS": "the most resident at once, in characters",
         "mermaid, sequence diagrams": "two at 100 columns, added up; mermaid-text leaves the one with long messages 357 wide",
         "mermaid, flowcharts": "seven, from a vault and Mermaid's docs, at 100 columns; added up",
         "mermaid, state diagrams": "two, at 100 columns; added up",
         "mermaid, gantt charts": "Mermaid's example and another, at 100 columns; added up",
         "mermaid, a 120-node flowchart": "generated: a chain, each node with an edge further on"}
KINDS = {"first paint": "wall", "CPU per line scroll": "CPU", "search": "wall", "resize, cold": "wall",
         "held page-down CPU": "CPU", "print to a pipe, wall": "wall", "previewer, a preview, wall": "wall",
         "previewer, the whole file, wall": "wall", "image, kitty graphics, wall": "wall", "image, text, wall": "wall",
         "image, kitty graphics, CPU": "CPU", "image, text, CPU": "CPU", "mermaid, sequence diagrams": "wall",
         "mermaid, flowcharts": "wall", "mermaid, state diagrams": "wall", "mermaid, gantt charts": "wall",
         "mermaid, a 120-node flowchart": "wall"}
# the viewers that draw mermaid with each crate
MERMAID_NOTES = {"mermaid-text": "md-tui's", "mmdflux": "leaf's"}
TILES = [("first paint", "first paint"), ("CPU per line scroll", "CPU per line scrolled"),
         ("resize, cold", "re-layout after a resize"), ("peak RSS", "peak memory")]


def parse(path):
    """{title: {doc: {viewer: value, None, or "cap"}}} and the stamp from the header: a commit, or a commit and
    the files rerun at another ("137570c, logs e9ec3b7")."""
    text = open(path).read()
    commit = re.search(r"at ada (.+?):", text).group(1)
    panels = {}
    for block in text.strip().split("\n\n\n"):
        lines = block.strip().splitlines()
        if lines[0].startswith("# B2"):
            lines = lines[1:]
        title, names = lines[0][2:], lines[1].split()[2:]
        panels[title] = {r.split()[0]: {n: None if v == "?" else v if v == "cap" else float(v)
                                        for n, v in zip(names, r.split()[1:])}
                         for r in lines[2:] if not r.startswith("#")}
    return panels, commit


def num(v):
    return f"{v:.2f}" if v < 1 else f"{v:.1f}" if v < 100 else f"{v:,.0f}"


def why(metric, viewer, doc):
    return next((r for m, v, d, r in MISSING if m in (None, metric) and v == viewer and d in (None, doc)), "not measured")


def kind(metric):
    k = KINDS.get(metric)
    return f' <em class="kind {k.lower()}">{k}</em>' if k else ""


def chart(metric, cells, doc, note=NOTES.get, cap=None):
    """note(viewer) is the small print under a viewer's name; a `cap` cell was stopped after cap seconds."""
    unit = UNITS.get(metric, "ms")
    cells = {k: None if v == "cap" else v for k, v in cells.items()} | {k: "cap" for k, v in cells.items() if v == "cap"}
    vals = [v for v in cells.values() if v not in (None, "cap")]
    lo, hi = min(vals) / 2.5, max(vals)
    ada = cells["ada"]
    rows = sorted(cells.items(), key=lambda kv: (kv[1] in (None, "cap"), kv[1] == "cap", 0 if kv[1] == "cap" else kv[1] or 0))
    out = [f'<figure class="chart"><figcaption>{escape(CAPTIONS[metric])} <span>{unit}</span>{kind(metric)}'
           f'<small>{escape(DESCS[metric])}</small></figcaption><ol>']
    for viewer, v in rows:
        name = escape(NAMES.get(viewer, viewer))
        if n := note(viewer):
            name += f' <small>{escape(n)}</small>'
        cls = ' class="ada"' if viewer == "ada" else ""
        if v == "cap":
            out.append(f'<li class="missing"><span class="name">{name}</span>'
                       f'<span class="why">stopped after {cap} s</span></li>')
            continue
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
    out += tabbed("prev", "previewer-perf", f"{PLOTS}/previewer.dat", FILES, FILE_TAB,
                  "As a previewer: code, logs and JSON",
                  "What fzf's preview pane shows besides markdown, against the tools ada replaces there: bat, rich "
                  "and mcat. A preview stops at a budget; the whole file is printed, all of it, to a pipe.",
                  lambda f: lambda v: PREV_NOTES.get((v, f)))
    out += tabbed("img", "image-perf", f"{PLOTS}/images.dat", IMAGES, IMAGE_TAB,
                  "Image previews",
                  "A 100×50-cell preview of each image, printed to a pipe, against kitten icat, chafa, mcat and "
                  "viu: in kitty's graphics protocol, and drawn in characters for any other terminal.",
                  lambda f: IMAGE_NOTES.get)
    out += mermaid()
    out.append(f'<p class="stamp">B2 at ada <code>{commit}</code>, {date}{stamps()}</p>')
    return "\n".join(out)


def stamps():
    """The previewer files' and images' stamps, where they differ from the documents' commit."""
    extra = []
    for name, label in (("previewer", "previewer files"), ("images", "images"), ("mermaid", "mermaid")):
        if os.path.exists(path := f"{PLOTS}/{name}.dat"):
            commit = parse(path)[1]
            if commit != parse(f"{PLOTS}/interactive.dat")[1]:
                extra.append(f"{label} at ada <code>{escape(commit)}</code>")
    return "".join(f"; {e}" for e in extra)


def tabbed(key, anchor, path, inputs, checked, title, lede, notes):
    """A heading (with an anchor link, id anchor), a lede, one tab per input and a panel of charts each, from a .dat of plot.py's whose rows are
    the inputs. notes(input) gives the note function for that input's charts."""
    if not os.path.exists(path):
        return []
    panels, _ = parse(path)
    cap = re.search(r"cap at (\d+) s", open(path).read())
    out = [f'<h3 class="perf-sub" id="{anchor}">{escape(title)} <a class="anchor" href="#{anchor}" '
           f'aria-label="link to this section">#</a></h3><p class="lede">{escape(lede)}</p>',
           f'<div class="doc-tabs" role="radiogroup" aria-label="{key}">']
    for i, (f, (size, desc)) in enumerate(inputs.items()):
        on = " checked" if f == checked else ""
        out.append(f'<input type="radio" name="{key}" id="{key}-{i}"{on}>'
                   f'<label for="{key}-{i}"><b>{escape(size)}</b> {escape(desc)}</label>')
    out.append("</div>")
    for i, f in enumerate(inputs):
        out.append(f'<div class="perf-panel" data-{key}="{i}"><div class="charts">')
        out += [chart(m, panels[m][f], f, notes(f), cap and cap[1]) for m in panels]
        out.append("</div></div>")
    return out


def mermaid():
    """Mermaid diagrams, in process, against the crates md-tui and leaf draw them with: a chart per kind."""
    if not os.path.exists(path := f"{PLOTS}/mermaid.dat"):
        return []
    panels, _ = parse(path)
    out = ['<h3 class="perf-sub" id="mermaid-perf">Mermaid diagrams <a class="anchor" href="#mermaid-perf" '
           'aria-label="link to this section">#</a></h3><p class="lede">A <code>```mermaid</code> block parsed and '
           'drawn, against the crates two of the viewers above draw with: mermaid-text in md-tui and mmdflux in '
           'leaf, called as they call them. ada draws to the cells it paints from, the crates to a string; in '
           'process, the best of repeated runs on one core.</p>',
           '<div class="charts">']
    out += [chart(m, panels[m]["all"], None, MERMAID_NOTES.get) for m in panels]
    out.append("</div>")
    return out


def proof():
    """The hero's line under the pitch: how often ada is first over every ranked measurement, as plot.py counts it
    into interactive.dat's header, against how many other viewers."""
    text = open(f"{PLOTS}/interactive.dat").read()
    first, total = re.search(r"ada first in (\d+) of (\d+)", text).groups()
    others = len(re.search(r"^# doc (.*)$", text, re.M)[1].split()) - 1
    words = {4: "four", 5: "five", 6: "six", 7: "seven"}
    return (f'<p class="proof"><a href="#performance">First in {first} of {total} measurements</a> against '
            f'{words.get(others, others)} other terminal viewers, on time, CPU and memory.</p>')


page = open(PAGE).read()
new = re.sub(r"(<!-- perf:begin -->\n).*?(<!-- perf:end -->)", lambda m: m[1] + section() + "\n" + m[2], page,
             flags=re.S)
new = re.sub(r"(<!-- proof:begin -->\n).*?(<!-- proof:end -->)", lambda m: m[1] + proof() + "\n" + m[2], new,
             flags=re.S)
if "--check" in sys.argv:
    sys.exit(0 if new == page else 1)
open(PAGE, "w").write(new)
