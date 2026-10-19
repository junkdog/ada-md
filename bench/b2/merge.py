"""Merge hyperfine record dirs (b2a.zsh, b2p.zsh, b2i.zsh) into one, a later dir replacing an earlier one's tools.

    python3 -I bench/b2/merge.py OUT IN... [--drop NAME]...

Each `<input>.json` keeps every command's result, by name, from the last dir that ran it; each `*.rss.NAME.json`
and each `bytes.txt` line (keyed by its fields without `=`) comes from the last dir that has it. OUT then reads as
one full run: doctables.py on it needs no B2P_DIRS. --drop leaves a command out (rerun.zsh's reference ada,
"adaref")."""
import glob, json, os, shutil, sys

args, drop = [], set()
it = iter(sys.argv[1:])
for a in it:
    if a == "--drop":
        drop.add(next(it))
    else:
        args.append(a)
out, ins = args[0], args[1:]
os.makedirs(out, exist_ok=True)

merged, rss, nbytes = {}, {}, {}
for d in ins:
    for path in sorted(glob.glob(f"{d}/*.json")):
        f = os.path.basename(path)
        if ".rss." in f:
            if f.removesuffix(".json").rsplit(".", 1)[1] not in drop:
                rss[f] = path
            continue
        results = [r for r in json.load(open(path))["results"] if r["name"] not in drop]
        if f not in merged:
            merged[f] = json.load(open(path)) | {"results": results}
            continue
        old = {r["name"]: i for i, r in enumerate(merged[f]["results"])}
        for r in results:
            if r["name"] in old:
                merged[f]["results"][old[r["name"]]] = r
            else:
                merged[f]["results"].append(r)
    if os.path.exists(b := f"{d}/bytes.txt"):
        for line in open(b):
            key = [x for x in line.split() if "=" not in x]
            if key[-1] not in drop:
                nbytes[" ".join(key)] = line
for f, doc in merged.items():
    json.dump(doc, open(f"{out}/{f}", "w"))
for f, path in rss.items():
    shutil.copyfile(path, f"{out}/{f}")
if nbytes:
    open(f"{out}/bytes.txt", "w").writelines(nbytes.values())
