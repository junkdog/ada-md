"""Split the B2 jobs into 4 lanes (longest first, greedy) and write one spec per lane."""
import json, os, subprocess, sys
H = os.path.dirname(os.path.abspath(__file__))
LANES = ["0,2,4,6", "8,10,12,14", "16,18,20,22", "24,26,28,30"]  # one thread per physical core, a CCD each
viewers = "ada,treemd,treemd-head,leaf,mdfried,mdtui,glow"
jobs = json.loads(subprocess.check_output([sys.executable, "-I", H + "/mkspec.py", "5", "s,m,l", viewers]))
jobs += json.loads(subprocess.check_output([sys.executable, "-I", H + "/mkspec.py", "3", "xl", viewers]))
COST = {"treemd-l": 3, "treemd-m": 2, "treemd-xl": 4, "treemd-head-xl": 3, "mdtui-l": 3, "mdtui-m": 2, "mdtui-xl": 10,
        "mdfried-xl": 3, "glow-xl": 3, "leaf-xl": 2, "ada-xl": 1.5}
cost = lambda j: COST.get(j["name"], 1.2) * j["runs"]
lanes = [[] for _ in LANES]
for j in sorted(jobs, key=cost, reverse=True):
    min(lanes, key=lambda l: sum(map(cost, l))).append(j)
for i, l in enumerate(lanes):
    json.dump(l, open(f"{H}/lane{i}.json", "w"), indent=1)
    print(i, LANES[i], round(sum(map(cost, l))), "min:", " ".join(j["name"] for j in l))
