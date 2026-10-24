# python3 -I rss.py RUNS BIN... -- FILE...: each binary's `--print -w 100 FILE`, RUNS times after 2 warmups, the
# binaries interleaved run by run; prints a markdown table of the medians, peak RSS in MiB and wall ms. The order
# probe (order-probe.yml, MEM-08) puts a plain build beside build.py's, on runners valgrind and hyperfine don't
# reach. Peak RSS from wait4's rusage: ru_maxrss is KiB on Linux, bytes on macOS.
import os, statistics, subprocess, sys, time
runs, rest = int(sys.argv[1]), sys.argv[2:]
bins, files = rest[:rest.index("--")], rest[rest.index("--") + 1:]
unit = 1 if sys.platform == "darwin" else 1024

def run(bin, file):
    t = time.perf_counter()
    p = subprocess.Popen([bin, "--print", "-w", "100", file], stdout=subprocess.DEVNULL)
    _, status, ru = os.wait4(p.pid, 0)
    if status:
        sys.exit(f"{bin} {file}: exit status {status:#x}")
    return ru.ru_maxrss * unit / 2**20, (time.perf_counter() - t) * 1e3

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
