#!/usr/bin/env zsh
# full.zsh OUT: the whole B2 run, every viewer: the four lanes, print mode, the previewer files and images, mermaid
# diagrams against the crates md-tui and leaf draw them with (bench/diagrams/compare, built from the same tree as ADA;
# COMPARE overrides), then the vault notes' tables. OUT is a full record set (b2/all.jsonl, lane0..3.json, b2a/, b2p/, b2i/, doctables.md),
# so it serves as REF for rerun.zsh as it is.
# B2_PROFILE (on by default) holds every lane core ready for the run (b2-bench-profile: EPP performance, no idle
# state deeper than C1, the clock floor at the top; ada-optimization experiment 28, BEN-04) and writes the profile's
# id (or off) to OUT/profile.txt: cells made under different profiles don't mix. ADA is copied into OUT and run from there.
S=${B2_ROOT:-${0:A:h}/root}; OUT=${${1:?usage: full.zsh OUT}:A}   # absolute: the lane specs run ADA from the corpus
H=${0:A:h}; B2=$H/target/release/b2
L=(0,2,4,6 8,10,12,14 16,18,20,22 24,26,28,30)   # as lanes.py
[[ -e $OUT ]] && { echo "full: $OUT exists" >&2; exit 1 }
mkdir -p $OUT/b2
cp ${ADA:?set ADA} $OUT/ada; export ADA=$OUT/ada
COMPARE=${COMPARE:-$H/../diagrams/compare/target/release/compare}
[[ -x $COMPARE ]] || { echo "full: no $COMPARE (cargo build --release --manifest-path bench/diagrams/compare/Cargo.toml)" >&2; exit 1 }
cp $COMPARE $OUT/compare
PROFILE=${B2_PROFILE:-on}; PROFILE_ID=off
if [[ $PROFILE == on ]]; then   # restored when this script exits
  sudo -n /usr/local/sbin/b2-bench-profile on ${(j:,:)L} $$ || exit 1
  PROFILE_ID=$(cat /run/b2-bench-profile/id 2>/dev/null)
  [[ -n $PROFILE_ID ]] || { echo "full: the installed b2-bench-profile is older than bench/b2's (no id): reinstall it" >&2; exit 1 }
fi
print $PROFILE_ID > $OUT/profile.txt
mark() {
  local c clocks=()
  for c in 0 2 8 16 24; do clocks+=$(( $(</sys/devices/system/cpu/cpu$c/cpufreq/scaling_cur_freq) / 1000 )); done
  print "$1 $(date +%T) Tctl: $(sensors 2>/dev/null | awk '/^Tctl:/ { print $2 }')  clocks(0,2,8,16,24): $clocks" >> $OUT/run.log
}
print "ada $(sha256sum < $ADA | cut -c1-16) ($(git -C $H rev-parse --short HEAD) checked out), profile $PROFILE_ID" > $OUT/run.log

python3 -I $H/lanes.py > $OUT/lanes.txt && cp $H/lane?.json $OUT/   # lanes.py writes them beside itself
mark "b2 start"
pids=()
for i in 0 1 2 3; do
  taskset -c $L[i+1] $B2 run $OUT/lane$i.json $OUT/lane$i.jsonl > $OUT/lane$i.log 2>&1 &
  pids+=$!
done
rc=0; for p in $pids; do wait $p || rc=1; done
mark "b2 end rc=$rc"
cat $OUT/lane?.jsonl > $OUT/b2/all.jsonl
for part in b2a b2p b2i; do
  mark "$part start"
  env ${(U)part}_OUT=$OUT/$part zsh $H/$part.zsh > $OUT/$part.log 2>&1
  mark "$part end rc=$?"
done
mark "diagrams start"   # one core, still held ready
taskset -c 2 $OUT/compare --dat $OUT/mermaid.dat > $OUT/diagrams.md 2> $OUT/diagrams.log
mark "diagrams end rc=$?"
B2_ROOT=$OUT python3 -I $H/b2a_report.py > $OUT/b2a.md 2>&1
B2_ROOT=$OUT python3 -I $H/doctables.py > $OUT/doctables.md
B2_ROOT=$OUT python3 -I $H/rank.py > $OUT/rank.txt 2>&1
mark "done"
