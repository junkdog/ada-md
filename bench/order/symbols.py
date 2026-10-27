# The functions in `.text` of a linker map (`ADA_LINK_MAP=PATH cargo build --release`, build.rs), lld's or ld64's:
# (address, size, mangled name), sorted. The map holds the binary's own names whether or not it was stripped.
# order.txt holds them demangled (MEM-08): the mangled names' crate hashes change with the toolchain, the target and
# ada's version, so `cargo ordered` (xtask/, which reads these maps too) maps the order onto a build's own names.
import bisect, collections, os, re, subprocess, sys
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

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

# names demangled without crate hashes or LTO's `.NNN`, as order.txt keeps them (`cargo xtask demangle`); C names
# as they are
def demangle(names):
    out = subprocess.run(["cargo", "run", "-q", "--locked", "--manifest-path", f"{ROOT}/Cargo.toml", "-p", "xtask", "--",
                          "demangle"], input="".join(n + "\n" for n in names), capture_output=True, text=True,
                         check=True).stdout.splitlines()
    assert len(out) == len(names)
    return out
