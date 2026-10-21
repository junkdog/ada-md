#!/usr/bin/env zsh
# rerun.zsh REF OUT: ada's B2 cells rerun (the four lanes, print mode, the previewer files and images) beside the
# ada that REF measured, then merged over REF's records into OUT, to check ada against the last full run without
# rerunning the other viewers.
#   REF:     a full record set: b2/all.jsonl, lane0..3.json, b2a/, b2p/ and b2i/ (merge.py); OUT becomes one too,
#            so it can be the next rerun's REF
#   ADA:     the build under test; ADA_REF: REF's commit, built now (both from `git archive`, each with its own
#            CARGO_TARGET_DIR: "Comparing two ada builds" in ada-optimization). Both are copied into OUT together,
#            since a binary file's page-cache state moves peak RSS (O-23)
# B2_PROFILE (on by default) holds the bench cores ready for the run (b2-bench-profile: EPP performance, no idle
# state deeper than C1; ada-optimization experiment 28), which makes the lanes' load and CCD not matter: then the two
# run at once, one run per job per round, ada on the lanes it took in REF (lanes 2 and 3 folded onto 0 and 1) and
# adaref on the same lanes of the other CCD (lane i + 2), swapping sides each round: 6 rounds (4 for XL), so each build has each side equally often, since
# even held ready a lane moves single cells by 20–40% (experiment 28). abcmp.py takes each cell's A/B from the
# rounds' paired ratios, balanced over the sides. With B2_PROFILE=off they take turns on ada's lanes instead (ABBA).
# Records made with and without the profile don't mix: OUT/profile.txt says which, and a REF made the other way is
# refused (a REF without the file is from before the profile: off). Elsewhere both run in one hyperfine call. The reference ada ("adaref") stays out of the merge: its new/old
# against REF is the machine's drift, and ada's against it the A/B. Writes OUT/compare.md (abcmp.py),
# OUT/doctables.md (the vault notes' tables from the merged records) and OUT/run.log (times, Tctl and clocks).
# B2_MERGE_ONLY=1 redoes only the merge, compare.md and doctables.md of an existing OUT. B2_AB_ONLY=1 runs the A/B
# and writes compare.md, with no merge and no doctables.md, so it takes a REF made with or without the profile;
# old/REF then compares across the two, which isn't drift.
S=${B2_ROOT:-${0:A:h}/root}; REF=${${1:?usage: rerun.zsh REF OUT}:A}; OUT=${${2:?usage: rerun.zsh REF OUT}:A}   # absolute, as in full.zsh
H=${0:A:h}; B2=$H/target/release/b2
L=(0,2,4,6 8,10,12,14 16,18,20,22 24,26,28,30)   # as lanes.py
mark() {
  local c clocks=()
  for c in 0 2 8 16 24; do clocks+=$(( $(</sys/devices/system/cpu/cpu$c/cpufreq/scaling_cur_freq) / 1000 )); done
  print "$1 $(date +%T) Tctl: $(sensors 2>/dev/null | awk '/^Tctl:/ { print $2 }')  clocks(0,2,8,16,24): $clocks" >> $OUT/run.log
}
if [[ -z $B2_MERGE_ONLY ]]; then
[[ -e $OUT ]] && { echo "rerun: $OUT exists" >&2; exit 1 }
PROFILE=${B2_PROFILE:-on}; REF_PROFILE=$(cat $REF/profile.txt 2>/dev/null || print off)
[[ $PROFILE == $REF_PROFILE || -n ${B2_AB_ONLY:-} ]] || { echo "rerun: REF was measured with the profile $REF_PROFILE, this run $PROFILE" >&2; exit 1 }
mkdir -p $OUT/{run,b2}; print $PROFILE > $OUT/profile.txt
if [[ $PROFILE == on ]]; then   # restored when this script exits
  sudo -n /usr/local/sbin/b2-bench-profile on ${(j:,:)L} $$ || exit 1
fi
cp ${ADA:?set ADA} $OUT/ada; cp ${ADA_REF:?set ADA_REF} $OUT/ada-ref
export ADA=$OUT/ada ADA_REF=$OUT/ada-ref
cp $REF/lane?.json $OUT/
print "ada $(sha256sum < $ADA | cut -c1-16), adaref $(sha256sum < $ADA_REF | cut -c1-16); ref $REF" > $OUT/run.log

# the lanes: each of ada's jobs from REF's lanes, as mkspec.py writes it now, split into one-run jobs, one spec per
# round and lane (OUT/run/r<round>/lane<i>.json)
python3 -I - $H $REF $OUT/run $PROFILE <<'EOF' || exit 1
import json, os, subprocess, sys
h, ref, out, profile = sys.argv[1:]
def mk(binary, name, runs, docs):
    env = os.environ | {"ADA": binary}
    jobs = json.loads(subprocess.check_output([sys.executable, "-I", f"{h}/mkspec.py", str(runs), docs, "ada"], env=env))
    return [(j | {"name": j["name"].replace("ada-", name + "-", 1), "runs": 1}, runs) for j in jobs]
lane = {j["name"]: i for i in range(4) for j in json.load(open(f"{ref}/lane{i}.json"))}
# even round counts, so that side by side each build takes each CCD equally often
rounds = (6, 4) if profile == "on" else (5, 3)
new = mk(os.environ["ADA"], "ada", rounds[0], "s,m,l") + mk(os.environ["ADA"], "ada", rounds[1], "xl")
old = mk(os.environ["ADA_REF"], "adaref", rounds[0], "s,m,l") + mk(os.environ["ADA_REF"], "adaref", rounds[1], "xl")
if any(j["name"] not in lane for j, _ in new):
    sys.exit("rerun: a job of ada's isn't on REF's lanes")
if profile == "on":   # side by side, ada's lanes on CCD0 mirrored on CCD1: a job REF ran on lane 2 or 3 takes 0 or 1,
    # the same place on the other CCD, since each build takes each side every other round anyway
    lane = {name: i % 2 for name, i in lane.items()}
for r in range(rounds[0]):
    lanes = [[] for _ in range(4)]
    for (a, n), (b, _) in zip(new, old):
        if r >= n:
            continue
        i = lane[a["name"]]
        if profile == "on":   # at once, the sides swapping each round
            lanes[i].append(a if r % 2 == 0 else b); lanes[i + 2].append(b if r % 2 == 0 else a)
        else:                 # in turn on ada's lanes, the order flipping each round
            lanes[i] += [a, b] if r % 2 == 0 else [b, a]
    os.makedirs(f"{out}/r{r + 1}", exist_ok=True)
    for i, l in enumerate(lanes):
        if l:
            json.dump(l, open(f"{out}/r{r + 1}/lane{i}.json", "w"), indent=1)
    print(f"round {r + 1}:", "; ".join(f"lane {i}: " + " ".join(j["name"] for j in l) for i, l in enumerate(lanes) if l))
EOF
mark "b2 start"
for rd in $OUT/run/r<->(n); do
  pids=()
  for i in 0 1 2 3; do
    [[ -s $rd/lane$i.json ]] || continue
    taskset -c $L[i+1] $B2 run $rd/lane$i.json $rd/lane$i.jsonl > $rd/lane$i.log 2>&1 &
    pids+=$!
  done
  rc=0; for p in $pids; do wait $p || rc=1; done
  mark "b2 ${rd:t} rc=$rc"
done
# each record tagged with its round, for abcmp.py's paired ratios
python3 -I - $OUT/run/r<->/lane?.jsonl > $OUT/run/new.jsonl <<'EOF'
import json, sys
for path in sys.argv[1:]:
    rnd = int(path.rsplit("/", 2)[-2][1:])
    for line in open(path):
        print(json.dumps(json.loads(line) | {"round": rnd}))
EOF

# print mode, previewer files, images: ada and adaref in one hyperfine call per input
for part in b2a b2p b2i; do
  mark "$part start"
  env ${(U)part}_ONLY=ada ${(U)part}_OUT=$OUT/run/$part zsh $H/$part.zsh > $OUT/run/$part.log 2>&1
  mark "$part end rc=$?"
done
fi

if [[ -n ${B2_AB_ONLY:-} ]]; then
  python3 -I $H/abcmp.py $OUT $REF > $OUT/compare.md
  [[ $PROFILE == $REF_PROFILE ]] || print "\nold/REF: REF was measured with the profile $REF_PROFILE, this run $PROFILE: not drift" >> $OUT/compare.md
  mark "done (A/B only)"
  exit 0
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
