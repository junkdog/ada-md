#!/usr/bin/env zsh
# rerun.zsh REF OUT: ada's B2 cells rerun (the four lanes, print mode, the previewer files and images) beside the
# ada that REF measured, then merged over REF's records into OUT, to check ada against the last full run without
# rerunning the other viewers.
#   REF:     a full record set: b2/all.jsonl, lane0..3.json, b2a/, b2p/ and b2i/ (merge.py); OUT becomes one too,
#            so it can be the next rerun's REF
#   ADA:     the build under test; ADA_REF: REF's commit, built now (both from `git archive`, each with its own
#            CARGO_TARGET_DIR: "Comparing two ada builds" in ada-optimization). Both are copied into OUT together,
#            since a binary file's page-cache state moves peak RSS (O-23)
# The two run interleaved on the lanes ada took in REF, one run per job, the order flipping each round (ABBA), and
# in one hyperfine call per input elsewhere. The reference ada ("adaref") stays out of the merge: its new/old
# against REF is the machine's drift, and ada's against it the A/B. Writes OUT/compare.md (abcmp.py),
# OUT/doctables.md (the vault notes' tables from the merged records) and OUT/run.log (times, Tctl and clocks).
# B2_MERGE_ONLY=1 redoes only the merge, compare.md and doctables.md of an existing OUT.
S=${B2_ROOT:?set B2_ROOT}; REF=${1:?usage: rerun.zsh REF OUT}; OUT=${2:?usage: rerun.zsh REF OUT}
H=${0:A:h}; B2=$H/target/release/b2
L=(0,2,4,6 8,10,12,14 16,18,20,22 24,26,28,30)   # as lanes.py
mark() {
  local c clocks=()
  for c in 0 2 8 16 24; do clocks+=$(( $(</sys/devices/system/cpu/cpu$c/cpufreq/scaling_cur_freq) / 1000 )); done
  print "$1 $(date +%T) Tctl: $(sensors 2>/dev/null | awk '/^Tctl:/ { print $2 }')  clocks(0,2,8,16,24): $clocks" >> $OUT/run.log
}
if [[ -z $B2_MERGE_ONLY ]]; then
[[ -e $OUT ]] && { echo "rerun: $OUT exists" >&2; exit 1 }
mkdir -p $OUT/{run,b2}
cp ${ADA:?set ADA} $OUT/ada; cp ${ADA_REF:?set ADA_REF} $OUT/ada-ref
export ADA=$OUT/ada ADA_REF=$OUT/ada-ref
cp $REF/lane?.json $OUT/
print "ada $(sha256sum < $ADA | cut -c1-16), adaref $(sha256sum < $ADA_REF | cut -c1-16); ref $REF" > $OUT/run.log

# the lanes: each of ada's jobs from REF's lanes, as mkspec.py writes it now, split into one-run jobs, ada and
# adaref in turn
python3 -I - $H $REF $OUT/run <<'EOF' || exit 1
import json, os, subprocess, sys
h, ref, out = sys.argv[1:]
def mk(binary, name, runs, docs):
    env = os.environ | {"ADA": binary}
    jobs = json.loads(subprocess.check_output([sys.executable, "-I", f"{h}/mkspec.py", str(runs), docs, "ada"], env=env))
    return [j | {"name": j["name"].replace("ada-", name + "-", 1), "runs": 1} for j in jobs]
lane = {j["name"]: i for i in range(4) for j in json.load(open(f"{ref}/lane{i}.json"))}
pairs = [(a, b, n) for runs, docs in ((5, "s,m,l"), (3, "xl"))
         for a, b in zip(mk(os.environ["ADA"], "ada", runs, docs), mk(os.environ["ADA_REF"], "adaref", runs, docs))
         for n in [runs]]
for i in range(4):
    mine = [(a, b, n) for a, b, n in pairs if lane.get(a["name"]) == i]
    jobs = [j for r in range(max((n for *_, n in mine), default=0))
            for a, b, n in mine if r < n for j in ((a, b) if r % 2 == 0 else (b, a))]
    if jobs:
        json.dump(jobs, open(f"{out}/lane{i}.json", "w"), indent=1)
        print(f"lane {i}:", " ".join(j["name"] for j in jobs))
EOF
pids=()
mark "b2 start"
for i in 0 1 2 3; do
  [[ -s $OUT/run/lane$i.json ]] || continue
  taskset -c $L[i+1] $B2 run $OUT/run/lane$i.json $OUT/run/lane$i.jsonl > $OUT/run/lane$i.log 2>&1 &
  pids+=$!
done
rc=0; for p in $pids; do wait $p || rc=1; done
mark "b2 end rc=$rc"
cat $OUT/run/lane?.jsonl > $OUT/run/new.jsonl

# print mode, previewer files, images: ada and adaref in one hyperfine call per input
for part in b2a b2p b2i; do
  mark "$part start"
  env ${(U)part}_ONLY=ada ${(U)part}_OUT=$OUT/run/$part zsh $H/$part.zsh > $OUT/run/$part.log 2>&1
  mark "$part end rc=$?"
done
fi

# the merge: ada's new cells over REF's, except where adaref drifted from REF (OUT/keep.txt): those keep REF's
python3 -I $H/abcmp.py $OUT $REF > $OUT/compare.md
keep=("${(@f)$(<$OUT/keep.txt)}")
python3 -I - $REF/b2/all.jsonl $OUT/run/new.jsonl ${(M)keep:#lanes} > $OUT/b2/all.jsonl <<'EOF'
import json, sys
ref, new, kept = sys.argv[1], sys.argv[2], len(sys.argv) > 3
ada = lambda line: json.loads(line)["name"].rsplit("-", 1)[0] == "ada"
sys.stdout.writelines(open(ref) if kept else [x for x in open(ref) if not ada(x)] + [x for x in open(new) if ada(x)])
EOF
for part in b2a b2p b2i; do
  if (( ${keep[(Ie)$part wall]} + ${keep[(Ie)$part cpu]} )); then
    python3 -I $H/merge.py $OUT/$part $REF/$part
  else
    python3 -I $H/merge.py $OUT/$part $REF/$part $OUT/run/$part --drop adaref
  fi
  (( ${keep[(Ie)$part rss]} )) && python3 -I - $REF/$part $OUT/$part <<'EOF'
# REF's peak RSS for ada: its own short call's record, or, where it had none (the huge files whole), its result
# from the input's main call, written as one
import glob, json, os, shutil, sys
ref, out = sys.argv[1:]
for path in glob.glob(f"{ref}/*.json"):
    f = os.path.basename(path).removesuffix(".json")
    if ".rss." in f:
        continue
    if os.path.exists(own := f"{ref}/{f}.rss.ada.json"):
        shutil.copyfile(own, f"{out}/{f}.rss.ada.json")
    elif rs := [r for r in json.load(open(path))["results"] if r["name"] == "ada"]:
        json.dump({"results": rs}, open(f"{out}/{f}.rss.ada.json", "w"))
EOF
done
B2_ROOT=$OUT python3 -I $H/doctables.py > $OUT/doctables.md
mark "done"
