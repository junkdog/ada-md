#!/usr/bin/env python3
"""profile.py MODE DATA [ARGS]: readings of a profile.zsh record (BEN-009's phase 3).

A frame-pointer stack holds no inlined frames, and fat LTO inlines most of a stage into a few symbols, so a
symbol's self time can hide where it goes; `lines` breaks it down by the source line perf maps each sample to.

  self DATA                      self time by symbol
  lines DATA SYMBOL...           each SYMBOL's self time by source file, then its top lines
  callers DATA LEAF [CALLER [N]] samples whose leaf matches LEAF, by the first N frames above it that match CALLER
                                 (a regex; ada:: and 1 by default)
  incl DATA REGEX...             each REGEX's inclusive share: the samples with a matching frame anywhere
  parse DATA...                  a parse profile's buckets: `line_of`'s binary search (inlined into `markdown`),
                                 `line_starts`, allocation, and pulldown-cmark's own symbols

Shares are of all samples, the harness's setup (generating the input, its check) included: a few percent.
"""
import collections
import re
import subprocess
import sys


def report(data, *args):
    out = subprocess.run(["perf", "report", "-i", data, "--no-children", "--stdio", "-g", "none", *args],
                         capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        m = re.match(r"\s*([\d.]+)%\s+(.*?)\s*(-\s+-\s*)?$", line)
        if m and not line.startswith("#"):
            yield float(m.group(1)), m.group(2).strip()


def stacks(data):
    out = subprocess.run(["perf", "script", "-i", data, "-F", "ip,sym", "--no-inline"],
                         capture_output=True, text=True, check=True).stdout
    for sample in out.split("\n\n"):
        frames = [l.strip().split(None, 1)[-1] for l in sample.strip().splitlines()]
        if frames:
            yield frames


def short(path):
    path = re.sub(r"^/rustc/[0-9a-f]+/library/", "std:", path)
    path = re.sub(r"^.*/\.cargo/registry/src/[^/]+/", "", path)
    return re.sub(r"^.*/ada-md/", "", path)


def lines(data, sym):
    return list(report(data, "--sort", "srcline", "--full-source-path", "--symbols", sym,
                       "--percent-limit", "0.01"))


def main(mode, data, *args):
    if mode == "self":
        for pct, sym in report(data, "--sort", "symbol", "--percent-limit", "0.5"):
            print(f"{pct:6.2f}%  {sym[:160]}")
    elif mode == "lines":
        for sym in args:
            rows = lines(data, sym)
            files = collections.Counter()
            for pct, line in rows:
                files[short(line.rsplit(":", 1)[0])] += pct
            print(f"== {sym}, by file")
            for f, pct in files.most_common(12):
                print(f"{pct:6.2f}%  {f}")
            print("-- top lines")
            for pct, line in rows[:20]:
                print(f"{pct:6.2f}%  {short(line)}")
    elif mode == "callers":
        leaf = re.compile(args[0])
        caller = re.compile(args[1] if len(args) > 1 else r"ada::")
        depth = int(args[2]) if len(args) > 2 else 1
        total, hits = 0, collections.Counter()
        for frames in stacks(data):
            total += 1
            if leaf.search(frames[0]):
                chain = [f[:90] for f in frames[1:] if caller.search(f)][:depth]
                hits[" <- ".join(chain) or "?"] += 1
        for k, v in hits.most_common(12):
            print(f"{100 * v / total:6.2f}%  {k}")
    elif mode == "incl":
        pats = [re.compile(p) for p in args]
        total, hits = 0, [0] * len(pats)
        for frames in stacks(data):
            total += 1
            for i, p in enumerate(pats):
                hits[i] += any(p.search(f) for f in frames)
        for p, h in zip(args, hits):
            print(f"{100 * h / total:6.2f}%  {p}")
    elif mode == "parse":
        alloc = ("malloc", "cfree", "realloc", "finish_grow", "clear_highpages", "_int_free", "__mod_memcg")
        for d in (data, *args):
            line_of = 0.0  # `Builder::line_of` is src/doc/parse.rs:188-191 at d733660
            for pct, src in lines(d, "ada::doc::parse::markdown"):
                f, _, n = src.rpartition(":")
                if (f.endswith(("slice/mod.rs", "core/src/cmp.rs", "core/src/hint.rs", "slice/index.rs"))
                        or (f.endswith("src/doc/parse.rs") and n in ("188", "189", "190", "191"))):
                    line_of += pct
            syms = list(report(d, "--sort", "symbol", "--percent-limit", "0.01"))
            total = lambda pred: sum(p for p, s in syms if pred(s))
            print(f"{d.rsplit('/', 1)[-1]:<22} line_of {line_of:5.1f}%"
                  f"  line_starts {total(lambda s: 'parse::line_starts' in s):5.1f}%"
                  f"  alloc {total(lambda s: any(a in s for a in alloc)):5.1f}%"
                  f"  pulldown-cmark's symbols {total(lambda s: 'pulldown_cmark' in s):5.1f}%")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    main(*sys.argv[1:])
