# python3 -I order.py MAP TRACE... > order.txt
# The hot-code order from lackey traces (lackey.zsh): the functions each trace fetched from, demangled (MEM-08:
# `cargo ordered` maps them onto a build's own names), by the first trace that fetched them, then in address order (keeps the original neighbors together). So the
# first trace's code is one block at the start of `.text`, and the next trace's what it adds after it: give the
# smallest run first (print), and a run touches only a prefix (sorted by how many traces fetched a function
# instead, print S spread over all 1.6 MiB: experiment 34). MAP is the traced build's lld map (symbols.py).
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's directory out
from symbols import demangle, fetched
syms, _, hits = fetched(sys.argv[1], sys.argv[2:])
names = sorted(hits, key=lambda i: (min(hits[i]), syms[i][0]))
seen = set()
for name in demangle([syms[i][2] for i in names]):
    if name not in seen:
        seen.add(name); print(name)
sys.stderr.write(f"{len(seen)} names, {len(names)} functions, {sum(syms[i][1] for i in hits)/1024:.0f} KiB\n")
