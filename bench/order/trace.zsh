#!/usr/bin/env zsh
# trace.zsh [WORK]: regenerate order.txt, the functions build.py links first in ada's `.text` (MEM-02, MEM-08,
# ada-optimization experiments 30 and 34). The functions a typical run fetches go first, so a run maps ~1 MiB less
# of the binary: the kernel maps the pages around each fault, and with the hot code together there is less around
# it. The names are demangled without their crate hashes, so they hold across toolchains, targets and ada's
# version; build.py maps them onto each build's own. The traced binary is the release build itself, ordered, its
# symbols from lld's map. Rerun after large changes to what a run calls, or when build.py says it matched under
# 90% of the names.
# What runs, each under valgrind's lackey (every fetch traced; ~3 min): print S to a pipe, and B2's full TUI
# sessions on S and L (bench/b2: paint, scrolls, search, reload, resize). WORK (default: a temp dir) keeps the
# build and the traces. Needs valgrind and B2's root (B2_ROOT, as bench/b2: corpus/ and target/release/b2).
H=${0:A:h}; R=${H:h:h}; B2D=$R/bench/b2; S=${B2_ROOT:-$B2D/root}
W=${1:-$(mktemp -d)}; mkdir -p $W/traces
print "work: $W"
# the release build of the working tree, as shipped (stripped, ordered), with lld's map for its symbols
CARGO_TARGET_DIR=$W/target python3 -I $H/build.py --locked -q; rc=$?
(( rc == 0 || rc == 2 )) || exit 1
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
python3 -I $H/order.py $W/target/order/host/ada.map $W/traces/print-s.txt $W/traces/tui-s.txt $W/traces/tui-l.txt > $W/order.txt || exit 1
mv $W/order.txt $H/order.txt
