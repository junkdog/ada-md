#!/usr/bin/env zsh
# trace.zsh [WORK]: regenerate order.txt, the functions `cargo ordered` links first in ada's `.text` (MEM-02, MEM-08,
# ada-optimization experiments 30 and 34). The functions a typical run fetches go first, so a run maps ~1 MiB less
# of the binary: the kernel maps the pages around each fault, and with the hot code together there is less around
# it. The names are demangled without their crate hashes, so they hold across toolchains, targets and ada's
# version; `cargo ordered` maps them onto each build's own. The traced binary is the release build itself, ordered, its
# symbols from lld's map. Rerun when `--check` (below) fails, or when `cargo ordered` says it matched under 90% of the
# names.
# What runs, each under valgrind's lackey (every fetch traced; ~8 min): print S to a pipe, B2's full TUI sessions on
# S, M and L (bench/b2: paint, scrolls, search, reload, resize), then the other prints B2 measures (MEM-014): print L
# and XL, a Rust file whole, the previews of a log, JSON and a Makefile, and a GIF in half-blocks and in kitty's
# graphics, at once. The order lists each trace's functions after the earlier traces', so print S's and the TUI's
# code stays where it was and the rest follows it in one block instead of from cold `.text`. WORK (default: a temp
# dir) keeps the build and the traces. Needs valgrind and B2's root (B2_ROOT, as bench/b2: corpus/ and target/release/b2).
# trace.zsh --check [WORK]: whether order.txt has gone stale (BEN-005; run it before a release): print S and the TUI
# session on S traced on the build as shipped, and stale.py's report of the hot functions left out of the order;
# exits 1 when they reach more of cold `.text` than ADA_ORDER_COLD windows (then re-trace). ~2 min; order.txt stays.
[[ $1 == --check ]] && { CHECK=1; shift }
H=${0:A:h}; R=${H:h:h}; B2D=$R/bench/b2; S=${B2_ROOT:-$B2D/root}
W=${${1:-$(mktemp -d)}:A}; mkdir -p $W/traces   # absolute: b2 runs the traced session from the corpus
print "work: $W"
# the release build of the working tree, as shipped (stripped, ordered), with lld's map for its symbols
CARGO_TARGET_DIR=$W/target cargo run -q --locked --manifest-path $R/Cargo.toml -p xtask -- ordered --locked -q; rc=$?
(( rc == 0 || rc == 2 )) || exit 1
EXE=$W/target/release/ada
# lackey.zsh's filter, built before the first trace: built inside one, it would stall the traced run behind the fifo
cargo build -q --release --locked --manifest-path $R/Cargo.toml -p xtask || exit 1
print "print S"
$H/lackey.zsh $W/traces/print-s.txt $EXE -p -w 100 $S/corpus/s.md < /dev/null > /dev/null || exit 1
DOCS=${CHECK:+s}; DOCS=${DOCS:-s,m,l}
print "TUI ${(U)DOCS}"
ADA=$EXE B2_ROOT=$S python3 -I $B2D/mkspec.py 1 $DOCS ada | python3 -I -c '
import json, sys
w, lackey = sys.argv[1:]
jobs = json.load(sys.stdin)
for j in jobs:
    j["cmd"] = [lackey, f"{w}/traces/tui-{j['"'"'file'"'"'].split(".")[0]}.txt"] + j["cmd"]
json.dump(jobs, open(f"{w}/tui.json", "w"), indent=1)' $W $H/lackey.zsh || exit 1
$B2D/target/release/b2 run $W/tui.json $W/tui.jsonl > $W/tui.log 2>&1 || { print "b2 failed: $W/tui.log"; exit 1 }
[[ -n $CHECK ]] && exec python3 -I $H/stale.py $W/target/order/host/ada.{map,order} $W/traces/{print-s,tui-s}.txt
# the other prints, most common first (the order keeps this one): each its own lackey, all at once
C=$S/corpus
typeset -A more=(
  print-l "-p -w 100 $C/l.md"
  print-xl "-p -w 100 $C/xl.md"
  code-rs "-p -w 100 $C/code-m.rs"
  preview-log "-p -w 100 --height 50 $C/log.log"
  preview-json "-p -w 100 --height 50 $C/huge.json"
  preview-mk "-p -w 100 --height 50 $C/kernel-4m.mk"
  gif-half "-p -w 100 --height 50 --image-protocol halfblocks $C/img/photo.gif"
  gif-kitty "-p -w 100 --height 50 --image-protocol kitty $C/img/photo.gif"
)
names=(print-l print-xl code-rs preview-log preview-json preview-mk gif-half gif-kitty)
print "print: $names"
pids=()
for n in $names; do
  $H/lackey.zsh $W/traces/$n.txt $EXE ${=more[$n]} < /dev/null > /dev/null 2> $W/$n.err & pids+=($!)
done
for p in $pids; do wait $p || { print "a trace failed: $W/*.err"; exit 1 }; done
# written beside the traces and moved in only when whole: a failed run keeps the last order.txt
python3 -I $H/order.py $W/target/order/host/ada.map $W/traces/print-s.txt $W/traces/tui-{s,m,l}.txt \
  $W/traces/${^names}.txt > $W/order.txt || exit 1
mv $W/order.txt $H/order.txt
