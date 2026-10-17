"""Compare two SGR-styled outputs by what each character shows. Exit 1 on a difference."""
import re, sys

SEQ = re.compile(r'\x1b\[([0-9;]*)m')
VIS = {'ul', 'rev', 'strike'}

def lines(path):
    text = open(path, encoding='utf-8', errors='replace').read()
    out = []
    for n, line in enumerate(text.split('\n')):
        st = dict(mods=frozenset(), fg=None, bg=None, ulc=None)
        cells, pos = [], 0
        for m in list(SEQ.finditer(line)) + [None]:
            end = m.start() if m else len(line)
            for ch in line[pos:end]:
                blank = ch == ' ' and not (st['mods'] & VIS)
                cells.append((ch,
                              None if blank else st['fg'],
                              st['bg'],
                              frozenset() if blank else st['mods'],
                              st['ulc'] if 'ul' in st['mods'] else None))
            if not m:
                break
            pos = m.end()
            st = apply(st, m.group(1))
        if st != dict(mods=frozenset(), fg=None, bg=None, ulc=None):
            sys.exit(f'{path}:{n + 1}: line ends styled: {st}')
        while cells and cells[-1][0] == ' ' and cells[-1][1:] == (None, None, frozenset(), None):
            cells.pop()
        out.append(cells)
    return out

def apply(st, params):
    st = dict(st)
    p = [int(x) if x else 0 for x in params.split(';')] if params else [0]
    i, mods = 0, set(st['mods'])
    names = {1: 'bold', 2: 'dim', 3: 'italic', 4: 'ul', 7: 'rev', 9: 'strike'}
    while i < len(p):
        n = p[i]
        if n == 0:
            mods = set(); st.update(fg=None, bg=None, ulc=None)
        elif n in names: mods.add(names[n])
        elif n == 22: mods -= {'bold', 'dim'}
        elif n == 23: mods.discard('italic')
        elif n == 24: mods.discard('ul')
        elif n == 27: mods.discard('rev')
        elif n == 29: mods.discard('strike')
        elif n in (38, 48, 58):
            if p[i + 1] == 2: c = ('rgb', *p[i + 2:i + 5]); i += 4
            else: c = ('idx', p[i + 2]); i += 2
            st[{38: 'fg', 48: 'bg', 58: 'ulc'}[n]] = c
        elif n == 39: st['fg'] = None
        elif n == 49: st['bg'] = None
        elif n == 59: st['ulc'] = None
        elif 30 <= n <= 37 or 90 <= n <= 97: st['fg'] = ('named', n)
        elif 40 <= n <= 47 or 100 <= n <= 107: st['bg'] = ('named', n - 10)
        else: sys.exit(f'unknown SGR {n}')
        i += 1
    st['mods'] = frozenset(mods)
    return st

a, b = lines(sys.argv[1]), lines(sys.argv[2])
if a != b:
    for n, (x, y) in enumerate(zip(a, b)):
        if x != y:
            k = next(i for i in range(max(len(x), len(y))) if i >= len(x) or i >= len(y) or x[i] != y[i])
            print(f'line {n + 1} col {k}: {x[k:k+3]} vs {y[k:k+3]}')
            break
    else:
        print(f'line count {len(a)} vs {len(b)}')
    sys.exit(1)
