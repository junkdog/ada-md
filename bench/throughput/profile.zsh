#!/usr/bin/env zsh
# profile.zsh INPUT STAGE [SAMPLES]: one cell of examples/throughput under `perf record --call-graph fp`, BEN-009's
# phase 3; the vault's ada-throughput has the method. Builds this tree's example with frame pointers and line tables
# (unstripped) into $B2_ROOT/build/throughput-prof/, then runs only INPUT's STAGE (--only, --stages; SAMPLES samples,
# 60 by default), pinned to $THROUGHPUT_CPU (2), without the bench profile: a profile wants shares, not times.
# Records go to $B2_ROOT/build/throughput-prof/perf/INPUT.STAGE.{data,jsonl,err}; bench/throughput/profile.py reads
# them. Stages: c0 c1-parse c2 html parse c1-layout layout c1-print print (markdown), c0 c1-lex tokens highlight
# syntect (lexers).
setopt err_exit no_unset pipe_fail
H=${0:A:h}; REPO=${H:h:h}; S=${B2_ROOT:-$REPO/bench/b2/root}
CPU=${THROUGHPUT_CPU:-2}; BUILD=$S/build/throughput-prof
(( $# >= 2 )) || { echo "usage: profile.zsh INPUT STAGE [SAMPLES]" >&2; exit 2 }
input=$1 stage=$2 samples=${3:-60}

[[ $(</proc/sys/kernel/perf_event_paranoid) == 1 ]] || {
  echo "profile: kernel.perf_event_paranoid is $(</proc/sys/kernel/perf_event_paranoid), needs 1" >&2
  exit 1
}
mkdir -p $BUILD/perf
(cd $REPO && CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
  RUSTFLAGS="-C force-frame-pointers=yes" cargo build --release --example throughput --target-dir $BUILD) \
  > $BUILD/build.log 2>&1 || { echo "profile: the build failed, $BUILD/build.log" >&2; exit 1 }

out=$BUILD/perf/$input.$stage
perf record -q -F 20000 --call-graph fp -o $out.data -- \
  taskset -c $CPU $BUILD/release/examples/throughput $S/corpus --only $input --stages $stage --samples $samples \
  > $out.jsonl 2> $out.err || { echo "profile: the run failed, $out.err" >&2; exit 1 }
tail -1 $out.err
print "=== $out.data"
