# python3 -I stale.py MAP PLACED TRACE...: whether order.txt has gone stale (BEN-005). `cargo ordered`'s match rate counts
# order.txt's names found in a build, so a hot function added or renamed since the trace, never listed, can't fail
# it: it links into cold `.text`, where a fault maps the 64 KiB around it (fault-around). rc.2's TRM-002, TRM-004
# and TRM-005 cost 0.3–0.5 MiB that way at 99.4% matched (experiment 51). This lists the functions the traces
# fetched that the build didn't place, and counts the 64 KiB windows of `.text` they fetch from that the placed
# block doesn't already cover. MAP and PLACED are `cargo ordered`'s (ada.map, ada.order), the traces lackey.zsh's of that
# build (trace.zsh --check). Exits 1 past ADA_ORDER_COLD cold windows (default 2).
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's directory out
from symbols import demangle, fetched
limit = int(os.environ.get("ADA_ORDER_COLD", 2))
map_path, placed_path, traces = sys.argv[1], sys.argv[2], sys.argv[3:]
syms, lines, hits = fetched(map_path, traces)
placed = {l.strip() for l in open(placed_path) if l.strip()}
# lld links the placed functions first in `.text`, in one block
hot_end = max(a + n for a, n, name in syms if name in placed)
hot = {w for w in range(syms[0][0] >> 16, ((hot_end - 1) >> 16) + 1)}
end = syms[-1][0] + syms[-1][1]
cold, out = set(), set()
for t, path in enumerate(traces):
    mine = [i for i, ts in hits.items() if t in ts and syms[i][2] not in placed]
    windows = {v >> 16 for v in lines[t] if hot_end <= v < end} - hot
    cold |= windows; out.update(mine)
    print(f"{os.path.basename(path).removesuffix('.txt')}: {sum(t in ts for ts in hits.values())} functions fetched, "
          f"{len(mine)} outside the order ({sum(syms[i][1] for i in mine) / 1024:.1f} KiB), {len(windows)} cold 64 KiB windows")
out = sorted(out, key=lambda i: syms[i][0])
for i, name in zip(out, demangle([syms[i][2] for i in out])):
    print(f"  {syms[i][0] >> 16:#x}0000 {syms[i][1]:6d}  {name[:150]}")
print(f"{len(cold)} cold windows in all, up to {len(cold) * 64} KiB mapped beyond the placed block "
      f"({hot_end - syms[0][0] >> 10} KiB); the limit {limit}")
if len(cold) > limit:
    print("stale: re-trace (zsh bench/order/trace.zsh) and commit order.txt")
    sys.exit(1)
