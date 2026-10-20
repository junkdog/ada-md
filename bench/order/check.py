# python3 -I check.py MAP [ORDER]: how many of order.txt's names a build still has, from its lld map
# (`ADA_LINK_MAP=PATH cargo build --release`, build.rs). Names that went missing are functions renamed by a
# toolchain or dependency bump, or gone from the code: when many have, rerun trace.zsh.
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I leaves the script's directory out
from symbols import text_symbols
here = os.path.dirname(os.path.abspath(__file__))
have = set(n for _, _, n in text_symbols(sys.argv[1]))
names = [l.strip() for l in open(sys.argv[2] if len(sys.argv) > 2 else f"{here}/order.txt") if l.strip()]
found = sum(n in have for n in names)
print(f"{found} of {len(names)} names in {sys.argv[1]} ({100 * found / max(len(names), 1):.1f}%)")
sys.exit(0 if found >= 0.9 * len(names) else 1)
