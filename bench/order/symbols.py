# The functions in `.text` of an lld map (`ADA_LINK_MAP=PATH cargo build --release`, build.rs): (address, size,
# mangled name), sorted. The map holds the binary's own names whether or not it was stripped.
def text_symbols(path):
    lines = open(path).read().splitlines()
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
        elif in_text and col >= sym_col - 2:
            syms.append((int(p[0], 16), int(p[2], 16), p[4]))
    return sorted(syms)
