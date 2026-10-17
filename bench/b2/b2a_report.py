import json, os, sys, statistics as st
S = os.environ["B2_ROOT"]  # corpus/, tools/bin/, home/<viewer>/, b2/, b2a/
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
def f(x): return f"{x/1000:.2f} s" if x >= 1000 else (f"{x:.0f}" if x >= 100 else f"{x:.1f}")
print("| Viewer | " + " | ".join(f"{d.upper()}: wall (CPU) ms" for d in ["s", "m", "l", "xl"]) + " |")
print("|---|---|---|---|---|")
for n in names:
    print(f"| {n} | " + " | ".join(f"{f(res[(d,n)]['wall'])} ({f(res[(d,n)]['cpu'])})" for d in ["s", "m", "l", "xl"]) + " |")
print()
print("| Viewer | " + " | ".join(f"{d.upper()}: RSS MiB / output KB / Minstr" for d in ["s", "m", "l", "xl"]) + " |")
print("|---|---|---|---|---|")
for n in names:
    print(f"| {n} | " + " | ".join(f"{res[(d,n)]['rss']:.1f} / {nbytes[(d,n)]/1000:.0f} / {res[(d,n)]['ins']:.0f}" for d in ["s", "m", "l", "xl"]) + " |")
print("\nruns:", {k: v["n"] for k, v in res.items() if k[0] in ("s", "xl")})
