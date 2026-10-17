import re, sys, collections
for d in sys.argv[1:]:
    lines = open(d, encoding="utf-8").read().split("\n")
    n = len(lines)
    words = collections.defaultdict(list)
    for i, l in enumerate(lines):
        for w in re.findall(r"(?<![\w`*\[/.-])([A-Za-z]{7,14})(?![\w`*\]/.-])", l):
            words[w.lower()].append(i)
    cands = []
    for w, occ in words.items():
        if len(occ) == 1 and 0.55 * n < occ[0] < 0.75 * n and not lines[occ[0]].startswith(("```", "|", "    ")):
            cands.append((occ[0], w))
    cands.sort()
    out = []
    for i, w in cands:
        l = lines[i]
        nb = [x for x in re.findall(r"(?<![\w`*\[/.-])([A-Za-z]{7,14})(?![\w`*\]/.-])", l) if x.lower() != w and len(words[x.lower()]) <= 2 and min(words[x.lower()]) > 0.3 * n]
        if nb:
            out.append((i, w, nb[0], l[:110]))
    print(d, n)
    for o in out[:4]:
        print("  ", o)
