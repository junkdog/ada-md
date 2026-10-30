#!/usr/bin/env zsh
# run.zsh [COMMIT]: ada's throughput at COMMIT (HEAD by default), BEN-009; the vault's ada-throughput has the
# method. Builds COMMIT from `git archive` with this tree's harness laid over it (examples/throughput*, and
# examples/common), so every commit is measured by the same harness; then, under the bench profile:
#   cells:      examples/throughput pinned to one CPU ($THROUGHPUT_CPU, 2): every input and stage, with the
#               ceiling kernels and the core's counters, as JSON lines (cells.jsonl)
#   end to end: `ada -p -w 100` under hyperfine on B2's four physical cores (e2e.*.json)
# into $B2_ROOT/results/<date>-<commit>-throughput[-n]/. bench/throughput/report.py prints the tables.
# Aborts unless kernel.perf_event_paranoid is 1 and every input matches inputs.sha256.
#   THROUGHPUT_ONLY=xl,prose   only these inputs (examples/throughput's --only); no end-to-end cells unless listed
#   THROUGHPUT_SAMPLES=N       samples a cell (15)
#   B2_PROFILE=off             without the bench profile (records under different profiles don't mix)
setopt err_exit no_unset pipe_fail
H=${0:A:h}; REPO=${H:h:h}; S=${B2_ROOT:-$REPO/bench/b2/root}
CORPUS=$S/corpus; CPU=${THROUGHPUT_CPU:-2}; E2E_CPUS=2,4,6,8
COMMIT=$(git -C $REPO rev-parse --short ${1:-HEAD})

[[ $(</proc/sys/kernel/perf_event_paranoid) == 1 ]] || {
  echo "run: kernel.perf_event_paranoid is $(</proc/sys/kernel/perf_event_paranoid), needs 1: sudo sysctl kernel.perf_event_paranoid=1" >&2
  exit 1
}
(cd $CORPUS && sha256sum --quiet -c $H/inputs.sha256) || { echo "run: an input in $CORPUS changed (inputs.sha256)" >&2; exit 1 }

OUT=$S/results/$(date +%F)-$COMMIT-throughput; n=1
while [[ -e $OUT ]]; do OUT=$S/results/$(date +%F)-$COMMIT-throughput-$((++n)); done
mkdir -p $OUT

# the build: COMMIT's tree, this tree's harness; cargo rebuilds only what changed since the last run
BUILD=$S/build/throughput-$COMMIT
if [[ ! -d $BUILD ]]; then
  mkdir -p $BUILD
  git -C $REPO archive $COMMIT | tar -x -C $BUILD
fi
rm -rf $BUILD/examples/throughput $BUILD/examples/common
cp -r $REPO/examples/throughput.rs $REPO/examples/throughput $REPO/examples/common $BUILD/examples/
harness=(examples/throughput.rs examples/throughput examples/common)
HARNESS=$(cd $REPO && find $harness -type f | sort | xargs sha256sum | sha256sum | cut -c1-16)
DIRTY=$(git -C $REPO status --porcelain -- $harness | wc -l)
(cd $BUILD && CARGO_TARGET_DIR=$BUILD/target cargo build --release --locked --example throughput --bin ada) \
  > $OUT/build.log 2>&1 || { echo "run: the build failed, $OUT/build.log" >&2; exit 1 }
# fresh copies, as B2 runs them (BEN-001)
cp $BUILD/target/release/examples/throughput $BUILD/target/release/ada $OUT/

# drop the page cache and compact memory once if free memory is low, as b2a.zsh does (B2_DROP_CACHES=off skips)
DROP=${B2_DROP_CACHES:-/usr/local/sbin/oxasm-drop-caches}
free_kib=$(awk '/^MemFree:/ { print $2 }' /proc/meminfo)
if [[ $DROP != off ]] && (( free_kib < ${B2_DROP_BELOW_GB:-8} * 1024 * 1024 )); then
  sudo -n $DROP >&2 || { echo "run: sudo -n $DROP failed; or set B2_DROP_CACHES=off" >&2; exit 1 }
  print "dropped caches: MemFree was $((free_kib / 1024)) MiB" >> $OUT/run.log
fi

PROFILE_ID=off
if [[ ${B2_PROFILE:-on} == on ]]; then   # restored when this script exits
  sudo -n /usr/local/sbin/b2-bench-profile on $E2E_CPUS $$ || exit 1
  PROFILE_ID=$(</run/b2-bench-profile/id)
fi
print $PROFILE_ID > $OUT/profile.txt

mark() {
  local c clocks=()
  for c in ${(s:,:)E2E_CPUS}; do clocks+=$(( $(</sys/devices/system/cpu/cpu$c/cpufreq/scaling_cur_freq) / 1000 )); done
  print "$1 $(date +%T) Tctl: $(sensors 2>/dev/null | awk '/^Tctl:/ { print $2 }')  clocks($E2E_CPUS): $clocks" >> $OUT/run.log
}
{
  print "commit $COMMIT"
  print "harness $HARNESS ($DIRTY files uncommitted)"
  print "profile $PROFILE_ID"
  print "cpu $CPU, end to end $E2E_CPUS"
  print "date $(date -Is)"
  print "kernel $(uname -r)"
  print "rustc $(cd $BUILD && rustc -V)"
  print "model $(lscpu | awk -F: '/Model name/ { gsub(/^ +/, "", $2); print $2 }')"
} > $OUT/meta.txt
mark start

args=($CORPUS)
[[ -n ${THROUGHPUT_ONLY:-} ]] && args+=(--only $THROUGHPUT_ONLY)
[[ -n ${THROUGHPUT_SAMPLES:-} ]] && args+=(--samples $THROUGHPUT_SAMPLES)
taskset -c $CPU $OUT/throughput $args > $OUT/cells.jsonl 2> $OUT/cells.log \
  || { echo "run: throughput failed, $OUT/cells.log" >&2; exit 1 }
mark cells

# end to end, as b2a.zsh runs print mode: B2's config, stdin /dev/null, stdout a pipe
export XDG_CONFIG_HOME=$S/home/x/config COLUMNS=100
e2e=("xl:-p -w 100 $CORPUS/xl.md" "log:-p -w 100 -l log $CORPUS/log.log")
for c in $e2e; do
  id=${c%%:*}
  [[ -z ${THROUGHPUT_ONLY:-} || ,$THROUGHPUT_ONLY, == *,$id,* ]] || continue
  taskset -c $E2E_CPUS $S/tools/bin/hyperfine -N --input=null --output=pipe --warmup 3 --min-runs 20 \
    --export-json $OUT/e2e.$id.json -n $id "$OUT/ada ${c#*:}" > $OUT/e2e.$id.txt 2>&1
done
mark end
print "=== $OUT"
