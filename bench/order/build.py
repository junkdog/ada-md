# python3 -I build.py [CARGO_ARGS...]: `cargo build --release CARGO_ARGS` with the hot code first in `.text` (MEM-02,
# MEM-08): the binary the release ships and the benches measure. A plain build isn't ordered: order.txt's names are
# demangled, and lld needs the build's own. So the build links with a map, `place` maps order.txt onto the map's
# names, and the build links again with them (ADA_SYMBOL_ORDER, build.rs). The order doesn't change a name, so the
# next run links with the last one's and links once more only if it no longer fits: after a code change.
# The files go in CARGO_TARGET_DIR (default: target)/order/TARGET (`--target`'s, or host). Exits 1 when the build fails, 2 when it built but
# matched under 90% of order.txt's names (trace.zsh says how many are left after a large change); on a target
# lld doesn't link (build.rs) the build has no map, and goes unordered.
import os, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's directory out
from symbols import HERE, place
root = os.path.dirname(os.path.dirname(HERE))
args = sys.argv[1:]
triple = next((a.split("=", 1)[1] for a in args if a.startswith("--target=")),
              next((b for a, b in zip(args, args[1:]) if a == "--target"), "host"))
work = os.path.join(os.environ.get("CARGO_TARGET_DIR") or f"{root}/target", "order", triple)
os.makedirs(work, exist_ok=True)
order, link_map = f"{work}/ada.order", f"{work}/ada.map"
env = os.environ | {"ADA_SYMBOL_ORDER": order, "ADA_LINK_MAP": link_map}

# a build cargo finds fresh doesn't link: the last link's map is still the binary's
def build():
    cmd = ["cargo", "build", "--release", "--manifest-path", f"{root}/Cargo.toml", *args]
    if subprocess.run(cmd, env=env).returncode:
        sys.exit(1)
    if not os.path.exists(link_map):
        print("symbol order: not linked by lld on this target, unordered")
        sys.exit(0)
    return place(link_map)

last = open(order).read().splitlines() if os.path.exists(order) else None
placed, found, total = build()
if placed != last:
    with open(order, "w") as f:
        f.writelines(n + "\n" for n in placed)
    if build()[0] != placed:
        sys.exit("symbol order: the second link renamed functions")
print(f"symbol order: {found} of {total} names ({100 * found / max(total, 1):.1f}%), {len(placed)} functions")
sys.exit(0 if found >= 0.9 * total else 2)
