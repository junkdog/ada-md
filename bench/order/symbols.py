# The functions in `.text` of a linker map (`ADA_LINK_MAP=PATH cargo build --release`, build.rs), lld's or ld64's:
# (address, size, mangled name), sorted. The map holds the binary's own names whether or not it was stripped.
# order.txt holds them demangled (MEM-08): the mangled names' crate hashes change with the toolchain, the target and
# ada's version, so `place` maps the order onto a build's own names, for its second link (build.py).
import bisect, collections, os, re, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))

# What lackey traces (lackey.zsh) of the build MAP maps fetched: (syms, lines, hits). syms is text_symbols(MAP),
# lines each trace's fetched 256-byte lines as addresses in the binary, hits the traces that fetched each function,
# by its index in syms. Where valgrind loaded the binary isn't in the trace, and differs between versions (0x108000,
# 0x4000000 in 3.25 for a PIE): the candidate that puts the most fetches inside the binary's `.text` is taken.
def fetched(map_path, traces):
    syms = text_symbols(map_path)
    starts = [s[0] for s in syms]
    raw = [[int(l.split()[1], 16) << 8 for l in open(path) if l.startswith("I ")] for path in traces]
    lo, hi = syms[0][0], syms[-1][0] + syms[-1][1]
    inside = {b: sum(lo <= a - b < hi for a in raw[0]) for b in (0, 0x108000, 0x400000, 0x4000000)}
    base = max(inside, key=inside.get)
    sys.stderr.write(f"base {base:#x}: {inside[base]} of {len(raw[0])} fetched lines in the binary's functions\n")
    lines = [[a - base for a in t] for t in raw]
    hits = collections.defaultdict(set)
    for t, fetched in enumerate(lines):
        for v in fetched:
            # a 256-byte line can hold several small functions: take every one it overlaps
            i = bisect.bisect_right(starts, v + 255) - 1
            while i >= 0 and syms[i][0] + max(syms[i][1], 1) > v:
                hits[i].add(t); i -= 1
                if syms[i + 1][0] <= v: break
    return syms, lines, hits

# a map's bytes aren't all UTF-8: ld64's lists C string literals by their contents
def text_symbols(path):
    lines = open(path, errors="replace").read().splitlines()
    if is_ld64(lines[0]):
        return ld64_text_symbols(lines)
    head = lines[0]
    sym_col = head.index("Symbol")
    out_col = head.index("Out")
    syms, in_text = [], False
    for l in lines[1:]:
        p = l.split()
        if len(p) < 5:
            continue
        col = len(l) - len(l.lstrip()) + l.lstrip().index(p[4], len(" ".join(p[:4])))
        if col <= out_col + 2:   # an output section
            in_text = p[4] == ".text"
        elif in_text and col >= sym_col - 2 and not p[4].startswith("$"):   # AArch64's `$x`, `$d`: mapping symbols
            syms.append((int(p[0], 16), int(p[2], 16), p[4]))
    return sorted(syms)

def is_ld64(head):
    return head.startswith("# Path: ")

# ld64's map: `# Sections:` rows (address, size, segment, section), then `# Symbols:` rows (address, size, `[ n]`
# for the object file, name), then `# Dead Stripped Symbols:`. A name can hold spaces (a literal's).
def ld64_text_symbols(lines):
    sections = lines.index("# Sections:")
    symbols = lines.index("# Symbols:")
    lo = hi = None
    for l in lines[sections + 2:symbols]:
        p = l.split()
        if p[2:4] == ["__TEXT", "__text"]:
            lo = int(p[0], 16); hi = lo + int(p[1], 16)
    if lo is None:
        raise ValueError("ld64 map: no __TEXT,__text section")
    syms = []
    for l in lines[symbols + 2:]:
        if l.startswith("#"):
            break
        m = re.match(r"(0x[0-9A-Fa-f]+)\s+(0x[0-9A-Fa-f]+)\s+\[\s*\d+\]\s(.*)", l)
        if m and lo <= int(m[1], 16) < hi:
            syms.append((int(m[1], 16), int(m[2], 16), m[3]))
    return sorted(syms)

# names demangled without crate hashes or LTO's `.NNN` (demangle/, built on first use); C names as they are
def demangle(names):
    tool = f"{HERE}/demangle"
    subprocess.run(["cargo", "build", "--release", "--locked", "-q", "--manifest-path", f"{tool}/Cargo.toml",
                    "--target-dir", f"{tool}/target"], check=True)
    out = subprocess.run([f"{tool}/target/release/demangle"], input="".join(n + "\n" for n in names),
                         capture_output=True, text=True, check=True).stdout.splitlines()
    assert len(out) == len(names)
    return out

# order.txt's names as the build mapped in MAP has them: every function a name stands for (a generic instantiated
# in several crates, LTO's copies of a local function), in the map's address order. Returns them and how many of
# order.txt's names the build has. Mach-O names carry a leading `_` (`__RNv…`, `_onig_search`): matched without it,
# placed with it, as ld64's order file takes them.
def place(map_path, order_path=f"{HERE}/order.txt"):
    have = [n for _, _, n in text_symbols(map_path)]
    with open(map_path, errors="replace") as f:
        bare = [n[1:] if n.startswith("_") else n for n in have] if is_ld64(f.readline()) else have
    funcs = collections.defaultdict(list)
    for mangled, name in zip(have, demangle(bare)):
        funcs[name].append(mangled)
    names = [l.strip() for l in open(order_path) if l.strip()]
    placed, seen = [], set()
    for name in names:
        for mangled in funcs.get(name, ()):
            if mangled not in seen:
                seen.add(mangled); placed.append(mangled)
    return placed, sum(n in funcs for n in names), len(names)
