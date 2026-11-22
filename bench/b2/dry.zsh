#!/usr/bin/env zsh
# dry.zsh OUT VIEWERS: a dry run of the lanes (startup, memory, idle and the keys) for viewers new to B2, beside two
# controls whose cells the last full run holds: ADA (the ada binary those cells came from, by default the 2026-11-14
# rerun's, de95d5b) and leaf. Fewer runs than a full run (3 for S, M and L, 2 for XL, DRY_RUNS and DRY_XL_RUNS), the
# same lanes (CCD0's two, then XL alone on the first), the bench profile and a page-cache drop, as full.zsh. The
# controls' cells against the full run's say how far the machine has drifted, so the new viewers' cells can be read
# beside the tables. Nothing is merged: OUT/report.md is report.py over this run alone. First used 2026-11-22 for
# the Obsidian viewers (basalt, ekphos, emeraldian, smd), before they join a full run.
S=${B2_ROOT:-${0:A:h}/root}; OUT=${${1:?usage: dry.zsh OUT VIEWERS}:A}; NEW=${2:?usage: dry.zsh OUT VIEWERS}
H=${0:A:h}; B2=$H/target/release/b2
L=(0,2,4,6 8,10,12,14)   # as lanes.py: CCD0's two lanes
[[ -e $OUT ]] && { echo "dry: $OUT exists" >&2; exit 1 }
mkdir -p $OUT/b2
cp ${ADA:-$S/results/2026-11-14-de95d5b-ada/ada} $OUT/ada; export ADA=$OUT/ada
sudo -n /usr/local/sbin/oxasm-drop-caches > $OUT/drop-caches.txt 2>&1 || { echo "dry: the cache drop failed" >&2; exit 1 }
sudo -n /usr/local/sbin/b2-bench-profile on ${(j:,:)L} $$ || exit 1
cat /run/b2-bench-profile/id > $OUT/profile.txt
mark() {
  local c clocks=()
  for c in 0 2 8 16 24; do clocks+=$(( $(</sys/devices/system/cpu/cpu$c/cpufreq/scaling_cur_freq) / 1000 )); done
  print "$1 $(date +%T) Tctl: $(sensors 2>/dev/null | awk '/^Tctl:/ { print $2 }')  clocks(0,2,8,16,24): $clocks" >> $OUT/run.log
}
print "dry: $NEW beside ada $(sha256sum < $ADA | cut -c1-16) and leaf" > $OUT/run.log
python3 -I - $H $OUT ada,leaf,$NEW ${DRY_RUNS:-3} ${DRY_XL_RUNS:-2} <<'EOF' || exit 1
import json, subprocess, sys
h, out, viewers, runs, xl_runs = sys.argv[1:]
jobs = json.loads(subprocess.check_output([sys.executable, "-I", h + "/mkspec.py", runs, "s,m,l", viewers]))
xl = json.loads(subprocess.check_output([sys.executable, "-I", h + "/mkspec.py", xl_runs, "xl", viewers]))
lanes = [[], []]
for j in jobs:  # every viewer's S, M and L over both lanes, the heavier documents spread first
    min(lanes, key=len).append(j)
for i, l in enumerate(lanes):
    json.dump(l, open(f"{out}/lane{i}.json", "w"), indent=1)
json.dump(xl, open(f"{out}/lane-xl.json", "w"), indent=1)
EOF
mark "b2 start"
pids=()
for i in 0 1; do
  taskset -c $L[i+1] $B2 run $OUT/lane$i.json $OUT/lane$i.jsonl > $OUT/lane$i.log 2>&1 &
  pids+=$!
done
rc=0; for p in $pids; do wait $p || rc=1; done
mark "b2 lanes end rc=$rc"
taskset -c $L[1] $B2 run $OUT/lane-xl.json $OUT/lane-xl.jsonl > $OUT/lane-xl.log 2>&1 || rc=1
mark "b2 end rc=$rc"
cat $OUT/{lane0,lane1,lane-xl}.jsonl > $OUT/b2/all.jsonl
python3 -I $H/report.py $OUT/b2/all.jsonl > $OUT/report.md 2>&1
print "=== RC=$rc"
exit $rc
