# python3 -I rss.py RUNS BIN... -- FILE...: each binary's `--print -w 100 FILE`, RUNS times after 2 warmups, the
# binaries interleaved run by run; prints a markdown table of the medians, peak RSS in MiB and wall ms. The order
# probe (order-probe.yml, MEM-08) puts a plain build beside build.py's, on runners valgrind and hyperfine don't
# reach. Peak RSS from /usr/bin/time, not from this process's wait4: on Linux, exec carries the forking process's
# peak into the child's ru_maxrss, so a child of python starts at python's ~10 MiB. time is small, and reports its
# child's: GNU's `%M` in KiB, BSD's `-l` in bytes.
import os, re, statistics, subprocess, sys, time
runs, rest = int(sys.argv[1]), sys.argv[2:]
bins, files = rest[:rest.index("--")], rest[rest.index("--") + 1:]
darwin = sys.platform == "darwin"

def run(bin, file):
    t = time.perf_counter()
    p = subprocess.run(["/usr/bin/time", *(["-l"] if darwin else ["-f", "%M"]), bin, "--print", "-w", "100", file],
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    ms = (time.perf_counter() - t) * 1e3
    if p.returncode:
        sys.exit(f"{bin} {file}: exit status {p.returncode}\n{p.stderr}")
    if darwin:
        peak = int(re.search(r"(\d+)\s+maximum resident set size", p.stderr)[1])
    else:
        peak = int(p.stderr.split()[-1]) * 1024
    return peak / 2**20, ms

print("| binary | " + " | ".join(f"{os.path.basename(f)} MiB | ms" for f in files) + " |")
print("|---|" + "---|---|" * len(files))
rows = {b: [] for b in bins}
for f in files:
    got = {b: [] for b in bins}
    for i in range(runs + 2):
        for b in bins:
            r = run(b, f)
            if i >= 2:
                got[b].append(r)
    for b in bins:
        rows[b].append((statistics.median(r for r, _ in got[b]), statistics.median(t for _, t in got[b])))
for b in bins:
    print(f"| {os.path.basename(b)} | " + " | ".join(f"{r:.2f} | {t:.2f}" for r, t in rows[b]) + " |")
