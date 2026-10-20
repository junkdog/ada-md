# python3 -I order.py MAP TRACE... > order.txt
# An lld --symbol-ordering-file from lackey traces (lackey.zsh): the functions each trace fetched from, mangled,
# by the first trace that fetched them, then in address order (keeps the original neighbors together). So the
# first trace's code is one block at the start of `.text`, and the next trace's what it adds after it: give the
# smallest run first (print), and a run touches only a prefix (sorted by how many traces fetched a function
# instead, print S spread over all 1.6 MiB: experiment 34). MAP is the traced build's lld map (symbols.py).
# Where valgrind loaded it isn't in the trace, and differs between versions (0x108000, 0x4000000 in
# 3.25 for a PIE): the candidate that puts the most fetches inside the binary's `.text` is taken.
import os, sys, bisect, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's directory out
from symbols import text_symbols
syms, traces = text_symbols(sys.argv[1]), sys.argv[2:]
starts = [s[0] for s in syms]
lines = [[int(l.split()[1], 16) << 8 for l in open(path) if l.startswith("I ")] for path in traces]
lo, hi = syms[0][0], syms[-1][0] + syms[-1][1]
inside = {b: sum(lo <= a - b < hi for a in lines[0]) for b in (0, 0x108000, 0x400000, 0x4000000)}
base = max(inside, key=inside.get)
sys.stderr.write(f"base {base:#x}: {inside[base]} of {len(lines[0])} fetched lines in the binary's functions\n")
hits = collections.defaultdict(set)
for t, fetched in enumerate(lines):
    for a in fetched:
        v = a - base
        # a 256-byte line can hold several small functions: take every one it overlaps
        i = bisect.bisect_right(starts, v + 255) - 1
        while i >= 0 and syms[i][0] + max(syms[i][1], 1) > v:
            hits[i].add(t); i -= 1
            if syms[i + 1][0] <= v: break
names = sorted(hits, key=lambda i: (min(hits[i]), syms[i][0]))
seen = set()
for i in names:
    if syms[i][2] not in seen:
        seen.add(syms[i][2]); print(syms[i][2])
sys.stderr.write(f"{len(seen)} functions, {sum(syms[i][1] for i in hits)/1024:.0f} KiB\n")
