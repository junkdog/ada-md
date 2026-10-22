#!/usr/bin/env zsh
# trace.zsh [WORK]: regenerate order.txt, the lld symbol ordering file that build.rs links ada with (MEM-02,
# ada-optimization experiments 30 and 34). The functions a typical run fetches go first in `.text`, so a run maps
# ~1 MiB less of the binary: the kernel maps the pages around each fault, and with the hot code together there
# is less around it.
# The names are rustc's v0 symbols, which hold while the toolchain and the dependencies' versions do; a bump
# renames the moved crates' symbols and the file goes stale without a word (lld skips names it doesn't know).
# So are the profile's settings: a build with `strip` changed hashes every name differently, which is why the
# traced binary is the release build itself and its symbols come from lld's map (ADA_LINK_MAP, build.rs). Rerun
# after a toolchain or dependency bump, or after large changes to what a run calls; `check.py MAP` says how many
# names a build still has.
# What runs, each under valgrind's lackey (every fetch traced; ~3 min): print S to a pipe, and B2's full TUI
# sessions on S and L (bench/b2: paint, scrolls, search, reload, resize). WORK (default: a temp dir) keeps the
# build and the traces. Needs valgrind and B2's root (B2_ROOT, as bench/b2: corpus/ and target/release/b2).
H=${0:A:h}; R=${H:h:h}; B2D=$R/bench/b2; S=${B2_ROOT:-$B2D/root}
W=${1:-$(mktemp -d)}; mkdir -p $W/traces
print "work: $W"
# the release build of the working tree, as shipped (stripped), with lld's map for its symbols
ADA_LINK_MAP=$W/ada.map CARGO_TARGET_DIR=$W/target cargo build --release --locked -q \
  --manifest-path $R/Cargo.toml || exit 1
EXE=$W/target/release/ada
print "print S"
$H/lackey.zsh $W/traces/print-s.txt $EXE -p -w 100 $S/corpus/s.md < /dev/null > /dev/null || exit 1
print "TUI S and L"
ADA=$EXE B2_ROOT=$S python3 -I $B2D/mkspec.py 1 s,l ada | python3 -I -c '
import json, sys
w, lackey = sys.argv[1:]
jobs = json.load(sys.stdin)
for j in jobs:
    j["cmd"] = [lackey, f"{w}/traces/tui-{j['"'"'file'"'"'].split(".")[0]}.txt"] + j["cmd"]
json.dump(jobs, open(f"{w}/tui.json", "w"), indent=1)' $W $H/lackey.zsh || exit 1
$B2D/target/release/b2 run $W/tui.json $W/tui.jsonl > $W/tui.log 2>&1 || { print "b2 failed: $W/tui.log"; exit 1 }
# written beside the traces and moved in only when whole: a failed run keeps the last order.txt
python3 -I $H/order.py $W/ada.map $W/traces/print-s.txt $W/traces/tui-s.txt $W/traces/tui-l.txt > $W/order.txt || exit 1
mv $W/order.txt $H/order.txt
python3 -I $H/check.py $W/ada.map $H/order.txt
