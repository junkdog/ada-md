#!/usr/bin/env zsh
# stress.zsh OUT OLD_DIR NEW_DIR: two ada builds A/B'd under steady load, for regression checks. B2's lanes time
# one key at a time, each after the screen held still, so the cores idle at their floor clock between keys and a
# cell moves with how busy the rest of the machine is (O-23; experiment 27: 1.26x between two runs of one build).
# Here every measurement runs back to back:
#   flood: `b2 flood` at STRESS_RATE keys a second (20,000) for 2 s through the pty (line j/k, page PgDn/PgUp,
#          end G/g) on each document: CPU per key and per frame, key-to-frame latency. At 1,000 a second ada's
#          cores were ~8% busy and stayed at 1.8–3.5 GHz, and the cells swung ±40%; 20,000 keeps them busy
#   bench: `examples/bench.rs` in-process (medians of 15): parse, first paint, relayout (a resize), repaint (a
#          page scroll, drawn) and one search keystroke
# Each DIR holds `ada` and `bench` (the example) built from one tree (`git archive`, own CARGO_TARGET_DIR). The two
# run at once for STRESS_ROUNDS rounds (6), on matched cores they swap each round (below). Writes OUT/flood.jsonl,
# OUT/bench/, OUT/run.log and OUT/compare.md.
S=${B2_ROOT:?set B2_ROOT}; OUT=${1:?usage: stress.zsh OUT OLD_DIR NEW_DIR}
OLD=${2:?usage: stress.zsh OUT OLD_DIR NEW_DIR}; NEW=${3:?usage: stress.zsh OUT OLD_DIR NEW_DIR}
H=${0:A:h}; B2=$H/target/release/b2; R=${STRESS_ROUNDS:-6}; RATE=${STRESS_RATE:-20000}
[[ -e $OUT ]] && { echo "stress: $OUT exists" >&2; exit 1 }
mkdir -p $OUT/bench
for n in old new; do   # fresh copies, together (O-23)
  d=${${(P)${(U)n}}}; cp $d/ada $OUT/ada-$n; cp $d/bench $OUT/bench-$n
done
# drop caches once if free memory is low (as b2a.zsh), not before each flood: that would land inside the other
# build's measurement
DROP=${B2_DROP_CACHES:-/usr/local/sbin/oxasm-drop-caches}
if [[ $DROP != off ]] && (( $(awk '/^MemFree:/ { print $2 }' /proc/meminfo) < ${B2_DROP_BELOW_GB:-8} * 1024 * 1024 )); then
  sudo -n $DROP >&2 || { echo "stress: sudo -n $DROP failed; or set B2_DROP_CACHES=off" >&2; exit 1 }
fi
export B2_DROP_CACHES=off
# B2_PROFILE (on by default): hold the cores used here ready for the run (b2-bench-profile, ada-optimization
# experiment 28); restored when this script exits
[[ ${B2_PROFILE:-on} == on ]] && { sudo -n /usr/local/sbin/b2-bench-profile on 4,6,8,10,16,18,20,22 $$ || exit 1 }
print ${B2_PROFILE:-on} > $OUT/profile.txt
mark() {
  local c clocks=()
  for c in 8 10 16; do clocks+=$(( $(</sys/devices/system/cpu/cpu$c/cpufreq/scaling_cur_freq) / 1000 )); done
  print "$1 $(date +%T) Tctl: $(sensors 2>/dev/null | awk '/^Tctl:/ { print $2 }')  clocks(8,10,16): $clocks" >> $OUT/run.log
}
KEYS=("line:j:k:300" "page:\\e[6~:\\e[5~:20" "end:G:g:1")
# The two builds run at once, each in a slot of CCD0 cores matched by amd_pstate_prefcore_ranking (4+8: 226+231,
# 6+10: 236+221), so both see the same clocks and load at the same moment; they swap slots each round, so neither
# keeps the better core. The flood drivers sit on CCD1.
SLOT_BENCH=(8 6) SLOT_VIEW=(4,8 6,10) SLOT_DRIVE=(16,18 20,22)
one() {   # one build's round: build, slot, round
  local n=$1 s=$2 r=$3 d k parts rec
  taskset -c $SLOT_BENCH[s] $OUT/bench-$n s.md m.md l.md xl.md > $OUT/bench/$n-$r.txt 2>&1
  for d in s m l xl; do
    for k in $KEYS; do
      parts=("${(@s.:.)k}")
      rec=$(taskset -c $SLOT_DRIVE[s] $B2 flood . $RATE 2 $parts[2] $parts[3] $parts[4] -- taskset -c $SLOT_VIEW[s] $OUT/ada-$n $d.md)
      print -r -- "{\"build\": \"$n\", \"slot\": $s, \"round\": $r, \"doc\": \"$d\", \"keys\": \"$parts[1]\", \"flood\": $rec}" > $OUT/.flood-$n
      cat $OUT/.flood-$n >> $OUT/flood-$n.jsonl
    done
  done
}
cd $S/corpus
for r in {1..$R}; do
  (( r % 2 )) && slots=(1 2) || slots=(2 1)
  mark "round $r (old in slot $slots[1])"
  one old $slots[1] $r & p1=$!
  one new $slots[2] $r & p2=$!
  wait $p1; wait $p2
done
mark "done"
cat $OUT/flood-old.jsonl $OUT/flood-new.jsonl > $OUT/flood.jsonl

python3 -I - $OUT $RATE > $OUT/compare.md <<'EOF'
import glob, json, os, statistics as st, sys
from collections import defaultdict
out, rate = sys.argv[1], int(sys.argv[2])
fmt = lambda x: "" if x is None else f"{x:.3f}" if x < 10 else f"{x:.1f}"
def row(label, vals):
    """vals: {(build, round): value}. Medians per build, and each round's new/old (the two ran at once)."""
    old = [v for (b, _), v in vals.items() if b == "old"]; new = [v for (b, _), v in vals.items() if b == "new"]
    pairs = [vals[("new", r)] / vals[("old", r)] for (b, r) in vals if b == "old" and ("new", r) in vals and vals[("old", r)]]
    if not pairs:
        return ""
    return (f"| {label} | {fmt(st.median(old))} | {fmt(st.median(new))} | {st.median(pairs):.3f} | "
            f"{min(pairs):.2f}–{max(pairs):.2f} |")
HEAD = "| doc, measure | old | new | new/old (paired) | per round |\n|---|---|---|---|---|"

print(f"## Flood: {rate:,} keys a second for 2 s, medians over rounds\n")
print(HEAD)
fl = defaultdict(dict)
for line in open(f"{out}/flood.jsonl"):
    r = json.loads(line); f = r["flood"]; key = (r["build"], r["round"])
    if f.get("stalled"):
        print(f"stalled: {r['build']} round {r['round']} {r['doc']} {r['keys']}", file=sys.stderr)
        continue
    base = (r["doc"], r["keys"])
    fl[base + ("CPU per key ms",)][key] = f["cpu_ms"] / f["keys"]
    fl[base + ("CPU per frame ms",)][key] = f["cpu_per_frame_ms"]
    fl[base + ("keys per frame",)][key] = f["keys_per_frame"]
    fl[base + ("key to frame p50 ms",)][key] = f["key_to_frame_ms"]["p50"]
for d in ("s", "m", "l", "xl"):
    for k in ("line", "page", "end"):
        for m in ("CPU per key ms", "CPU per frame ms", "keys per frame", "key to frame p50 ms"):
            print(row(f"{d.upper()} {k}, {m}", fl[(d, k, m)]))

print("\n## In-process (examples/bench.rs), ms, medians over rounds of medians of 15\n")
print(HEAD)
COLS = ["parse", "first paint", "relayout", "repaint", "search map", "search key"]
bn = defaultdict(dict)
for path in glob.glob(f"{out}/bench/*.txt"):
    build, rnd = os.path.basename(path).removesuffix(".txt").split("-")
    for line in open(path):
        x = line.split()
        if len(x) >= 9 and x[0].isdigit():
            for c, v in zip(COLS, x[2:8]):
                bn[(x[8].removesuffix(".md"), c)][(build, int(rnd))] = float(v)
for d in ("s", "m", "l", "xl"):
    for c in COLS:
        print(row(f"{d.upper()} {c}", bn[(d, c)]))
EOF
