#!/usr/bin/env zsh
# lackey.zsh OUT.txt CMD...: run CMD under valgrind's lackey, writing each 256-byte line it touched once, as
# "I hexline" (a fetch) or "D hexline" (a load or store). The raw trace goes through a fifo and never reaches the
# disk. stdin and stdout are CMD's, so it can stand in for a viewer in a b2 job. The lines are kept by `cargo xtask
# lackey` (xtask/src/lackey.rs), which replaced an awk filter that took most of a trace's CPU (2026-10-31).
out=$1; shift
fifo=$out.fifo; rm -f $fifo; mkfifo $fifo
cargo run -q --release --locked --manifest-path ${0:A:h:h:h}/Cargo.toml -p xtask -- lackey < $fifo > $out &
valgrind --tool=lackey --trace-mem=yes --log-file=$fifo "$@"
rc=$?; wait; rm -f $fifo; exit $rc
