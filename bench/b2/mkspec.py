import json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's directory out
import timeouts
S = os.environ["B2_ROOT"]  # corpus/, tools/bin/, home/<viewer>/, b2/, b2a/
B = S + "/tools/bin"
DOCS = {
    "s": dict(first="Hook", end_sentinel="alt-tab", search_term="consulted", search_sentinel="happened"),
    "m": dict(first="sift-llm", end_sentinel="denominator", search_term="rewards whichever", search_sentinel="arm quit first"),
    "l": dict(first="Successor", end_sentinel="item 19 makes", search_term="collision", search_sentinel="composed"),
    "xl": dict(first="harvest", end_sentinel="pricing", search_term="premature consolidation", search_sentinel="would hurt"),
}
PG = "\x1b[6~"
VIEWERS = {
    "ada": dict(cmd=[os.environ.get("ADA", "ada")],
                keys=dict(line="j", page=PG, end="G", home="g", quit="q", search=["/", "TERM", "\r"], search_reset=[])),
    "treemd": dict(cmd=[B + "/treemd"],
                keys=dict(setup=["\t"], line="j", page=PG, end="G", home="g", quit="q", search=["/", "TERM", "\r"], search_reset=["\x1b"])),
    "treemd-head": dict(cmd=[B + "/treemd-head"],
                keys=dict(setup=["\t"], line="j", page=PG, end="G", home="g", quit="q", search=["/", "TERM", "\r"], search_reset=["\x1b"])),
    "leaf": dict(cmd=[B + "/leaf", "--watch"],
                keys=dict(line="j", page=PG, end="G", home="g", quit="q", search=["/", "TERM", "\r"], search_reset=["\x1b"])),
    "mdfried": dict(cmd=[B + "/mdfried", "--watch"],
                keys=dict(line="j", page=PG, end="G", home="g", quit="q", search=["/", "TERM", "\r"], search_reset=["\x1b"])),
    "mdtui": dict(cmd=[B + "/mdt"],
                keys=dict(line="j", page="d", end="G", home="g", quit="q", search=["/", "TERM", "\r"], search_reset=["\x1b"])),
    "glow": dict(cmd=["/usr/bin/glow", "-s", "dark", "-t"],
                keys=dict(line="j", page=PG, end="G", home="g", quit="q", search=None, search_reset=[])),
}
ALL = ["paint", "idle", "line", "page", "end", "search", "burst", "reload", "resize"]
runs = int(sys.argv[1]); docs = sys.argv[2].split(","); viewers = sys.argv[3].split(",")
scen = sys.argv[4].split(",") if len(sys.argv) > 4 else ALL
jobs = []
skip = timeouts.skipped("b2")  # jobs that always hit the paint cap, while the viewer is still that version
for v in viewers:
    for d in docs:
        if f"{v}-{d}" in skip:
            continue
        vv = VIEWERS[v]
        sc = [x for x in scen if not (x == "search" and vv["keys"]["search"] is None)]
        home = f"{S}/home/{v}"
        jobs.append(dict(name=f"{v}-{d}", cmd=vv["cmd"] + [f"{d}.md"], cwd=S + "/corpus", file=f"{d}.md", runs=runs,
                         keys=vv["keys"], scenarios=sc, reload_from=DOCS[d]["first"], paint_cap=90 if d == "xl" else 20,
                         idle_runs=3,
                         env={"XDG_CONFIG_HOME": home + "/config", "XDG_DATA_HOME": home + "/data",
                              "XDG_STATE_HOME": home + "/state", "XDG_CACHE_HOME": home + "/cache"},
                         **DOCS[d]))
json.dump(jobs, sys.stdout, indent=1)
