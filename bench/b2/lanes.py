"""Split the B2 jobs over CCD0's two lanes and write one spec per lane: the S, M and L jobs over lanes 0 and 1
(longest first, greedy), which run at once, and the XL jobs into lane-xl.json, which runs after them on lane 0's
cores alone. CCD0 only since BEN-002 (2026-11-03, ada-optimization experiment 77): a job's CCD moved its per-key
cells up to 1.4× (CCD1 slower on line and page scroll), and XL search read 1.8× with both CCD0 lanes busy at once."""
import json, os, subprocess, sys
H = os.path.dirname(os.path.abspath(__file__))
LANES = ["0,2,4,6", "8,10,12,14"]  # one thread per physical core, CCD0's two lanes; XL runs on the first alone
viewers = "ada,treemd,treemd-head,leaf,mdfried,mdtui,glow"
jobs = json.loads(subprocess.check_output([sys.executable, "-I", H + "/mkspec.py", "5", "s,m,l", viewers]))
xl = json.loads(subprocess.check_output([sys.executable, "-I", H + "/mkspec.py", "3", "xl", viewers]))
COST = {"treemd-l": 3, "treemd-m": 2, "treemd-xl": 4, "treemd-head-xl": 3, "mdtui-l": 3, "mdtui-m": 2, "mdtui-xl": 10,
        "mdfried-xl": 3, "glow-xl": 3, "leaf-xl": 2, "ada-xl": 1.5}
cost = lambda j: COST.get(j["name"], 1.2) * j["runs"]
lanes = [[] for _ in LANES]
for j in sorted(jobs, key=cost, reverse=True):
    min(lanes, key=lambda l: sum(map(cost, l))).append(j)
for i, l in enumerate(lanes):
    json.dump(l, open(f"{H}/lane{i}.json", "w"), indent=1)
    print(i, LANES[i], round(sum(map(cost, l))), "min:", " ".join(j["name"] for j in l))
xl.sort(key=cost, reverse=True)
json.dump(xl, open(f"{H}/lane-xl.json", "w"), indent=1)
print("xl", LANES[0], round(sum(map(cost, xl))), "min, after both:", " ".join(j["name"] for j in xl))
