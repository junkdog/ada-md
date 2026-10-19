"""ada against the reference ada in one rerun (rerun.zsh): every ranked cell, the new build over the old one run
beside it, and the old one over REF's ada, the drift since the run being merged into.

    python3 -I bench/b2/abcmp.py OUT REF [THRESHOLD]

The lanes come from OUT/run/new.jsonl (jobs "ada-<doc>" and "adaref-<doc>"), print mode, the previewer files and
images from OUT/run/{b2a,b2p,b2i}/ (commands "ada" and "adaref" in one hyperfine call). A cell whose new/old is
outside 1 ± THRESHOLD (0.05) is listed; below each table, the medians. new/old is the A/B: drift moves both
sides alike. A drift far from 1 means REF's other viewers no longer compare with ada's new cells as they stand:
OUT/keep.txt lists each part and measure whose median drift is beyond 1 ± 0.03 ("lanes", "b2a rss", ...), whose
ada cells rerun.zsh then takes from REF, as the other viewers'."""
import glob, json, os, statistics as st, sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import doctables as dt

out, ref = sys.argv[1], sys.argv[2]
th = float(sys.argv[3]) if len(sys.argv) > 3 else 0.05
DRIFT = 0.03
keep, rss_drift = [], []


def load(path):
    runs = defaultdict(list)
    for line in open(path):
        r = json.loads(line)
        v, d = r["name"].rsplit("-", 1)
        runs[(v, d)].append(r)
    return runs


def summary(rows):
    for label, k in (("new/old", 0), ("old/REF", 1)):
        xs = [r[k] for r in rows if r[k] is not None]
        if xs:
            print(f"- {label}: median {st.median(xs):.3f} over {len(xs)} cells, range {min(xs):.2f}–{max(xs):.2f}")


def ratio(a, b):
    return a / b if a is not None and b else None


def show(head, rows):
    """rows: (new/old, old/REF, cells...): the table of cells outside the threshold, then the medians."""
    print("| " + " | ".join(head) + " | new/old | old/REF |\n|" + "---|" * (len(head) + 2))
    for r in sorted(rows, key=lambda r: -(r[0] or 1)):
        if r[0] is not None and abs(r[0] - 1) > th:
            print("| " + " | ".join(r[2:]) + f" | {r[0]:.2f} | " + (f"{r[1]:.2f}" if r[1] is not None else "–") + " |")
    print()
    summary(rows)


print(f"## Lanes: cells where ada's new/old is outside 1 ± {th}\n")
now, before = load(f"{out}/run/new.jsonl"), load(f"{ref}/b2/all.jsonl")
rows = []
for d in dt.DOCS:
    for f in dt.RANKED:
        dt.runs = now
        new, old = dt.val("ada", d, f), dt.val("adaref", d, f)
        dt.runs = before
        was = dt.val("ada", d, f)
        if new is None and old is None:
            continue
        rows.append((ratio(new, old), ratio(old, was), f, d.upper(), dt.fmt(was), dt.fmt(old), dt.fmt(new)))
show(["facet", "doc", "REF ada", "old now", "new now"], rows)
if abs(st.median(r[1] for r in rows if r[1] is not None) - 1) > DRIFT:
    keep.append("lanes")


def medians(path, name, rss_path):
    for r in json.load(open(path))["results"]:
        if r["name"] != name:
            continue
        m = r["measurements"]
        if any(x.get("exit_code", 0) != 0 for x in m):
            return None
        g = lambda k, m=m: st.median(x[k]["value"] for x in m)
        rss = g("memory_peak_resident")
        if os.path.exists(rss_path):
            m2 = json.load(open(rss_path))["results"][0]["measurements"]
            rss = st.median(x["memory_peak_resident"]["value"] for x in m2)
        return dict(wall=g("time_wall_clock") * 1e3, cpu=g("time_cpu") * 1e3, rss=rss / 2**20)
    return None


for part in ("b2a", "b2p", "b2i"):
    paths = sorted(p for p in glob.glob(f"{out}/run/{part}/*.json") if ".rss." not in p)
    if not paths:
        continue
    print(f"\n## {part}: cells where ada's new/old is outside 1 ± {th}\n")
    rows = []
    for p in paths:
        f = os.path.basename(p).removesuffix(".json")
        new = medians(p, "ada", f"{out}/run/{part}/{f}.rss.ada.json")
        old = medians(p, "adaref", f"{out}/run/{part}/{f}.rss.adaref.json")
        was = (medians(f"{ref}/{part}/{f}.json", "ada", f"{ref}/{part}/{f}.rss.ada.json")
               if os.path.exists(f"{ref}/{part}/{f}.json") else None)
        for k in ("wall", "cpu", "rss"):
            n, o, w = (x[k] if x else None for x in (new, old, was))
            rows.append((ratio(n, o), ratio(o, w), f, k, dt.fmt(w), dt.fmt(o), dt.fmt(n)))
    show(["input", "measure", "REF ada", "old now", "new now"], rows)
    # the previewer files' REF ada is partly a later commit's rerun (logs, JSON), so some cells' drift is a code
    # change; the median holds. Peak RSS is judged over all three parts at once: its drift is the binary file's
    # page-cache state (O-23), one cause, ~0.5 MiB, which a median over large images would hide
    for k in ("wall", "cpu"):
        xs = [r[1] for r in rows if r[3] == k and r[1] is not None]
        if xs and abs(st.median(xs) - 1) > DRIFT:
            keep.append(f"{part} {k}")
    rss_drift += [(part, r[1]) for r in rows if r[3] == "rss" and r[1] is not None]
if rss_drift and abs(st.median(x for _, x in rss_drift) - 1) > DRIFT:
    keep += sorted({f"{p} rss" for p, _ in rss_drift})
print(f"\n## Drift beyond 1 ± {DRIFT}: these keep REF's ada cells\n")
print("\n".join(f"- {k}" for k in keep) or "- none")
if rss_drift:
    print(f"\npeak RSS old/REF over the three parts: median {st.median(x for _, x in rss_drift):.3f}")
open(f"{out}/keep.txt", "w").write("".join(k + "\n" for k in keep))
