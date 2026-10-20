import json, os, sys, statistics as st
S = os.environ.get("B2_ROOT") or os.path.join(os.path.dirname(os.path.abspath(__file__)), "root")  # corpus/, tools/bin/, home/<viewer>/, b2/, b2a/
nbytes = {}
for l in open(f"{S}/b2a/bytes.txt"):
    d, n, b = l.split(); nbytes[(d, n)] = int(b.split("=")[1])
res = {}
for d in ["s", "m", "l", "xl"]:
    for r in json.load(open(f"{S}/b2a/{d}.json"))["results"]:
        m = r["measurements"]
        g = lambda k: st.median(x[k]["value"] for x in m)
        res[(d, r["name"])] = dict(n=len(m), wall=g("time_wall_clock") * 1e3, cpu=g("time_cpu") * 1e3,
                                   user=g("time_user") * 1e3, sys=g("time_system") * 1e3,
                                   rss=g("memory_peak_resident") / 2**20, ins=g("instructions") / 1e6)
names = ["ada", "mdcat", "glow", "leaf", "lowdown"]
def f(x): return f"{x:,.0f}" if x >= 100 else f"{x:.1f}"
# one number per cell, so that a table sorts by any column
D = ["s", "m", "l", "xl"]
print("| Viewer | " + " | ".join(f"{k} {d.upper()}" for d in D for k in ("Wall", "CPU")) + " |")
print("|" + "---|" * (1 + 2 * len(D)))
for n in names:
    print(f"| {n} | " + " | ".join(f"{f(res[(d,n)]['wall'])} | {f(res[(d,n)]['cpu'])}" for d in D) + " |")
print("\nwall and CPU in ms; peak RSS in MiB, output in KB, instructions in millions:\n")
print("| Viewer | " + " | ".join(f"{k} {d.upper()}" for k in ("RSS", "Output", "Instructions") for d in D) + " |")
print("|" + "---|" * (1 + 3 * len(D)))
for n in names:
    r = [res[(d, n)] for d in D]
    print(f"| {n} | " + " | ".join([f"{x['rss']:.1f}" for x in r] + [f"{nbytes[(d,n)]/1000:,.0f}" if (d, n) in nbytes else "" for d in D]
                                   + [f"{x['ins']:,.0f}" for x in r]) + " |")
print("\nruns:", {k: v["n"] for k, v in res.items() if k[0] in ("s", "xl")})
