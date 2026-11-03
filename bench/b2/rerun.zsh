#!/usr/bin/env zsh
# rerun.zsh REF OUT: ada's B2 cells rerun (the lanes, print mode, the previewer files and images) beside the
# ada that REF measured, then merged over REF's records into OUT, to check ada against the last full run without
# rerunning the other viewers.
#   REF:     a full record set: b2/all.jsonl, its lane specs, b2a/, b2p/ and b2i/ (merge.py); OUT becomes one too,
#            so it can be the next rerun's REF
#   ADA:     the build under test; ADA_REF: REF's commit, built now (both from `git archive`, each with its own
#            CARGO_TARGET_DIR: "Comparing two ada builds" in ada-optimization). Both are copied into OUT together,
#            since a binary file's page-cache state moves peak RSS (O-23), and the page cache is dropped after,
#            so both map as read back from disk, as full.zsh has every viewer's (BEN-001)
# B2_PROFILE (on by default) holds the bench cores ready for the run (b2-bench-profile: EPP performance, no idle
# state deeper than C1, the clock floor at the top; ada-optimization experiment 28, BEN-04), which makes the lanes' load
# not matter. The lanes are CCD0's two, as full.zsh's (BEN-002, 2026-11-03: a job's CCD moved its per-key cells up to
# 1.4×): ada's S, M and L jobs and adaref's run at once, one run per job per round, one build on each lane, swapping
# lanes each round; then the XL jobs on the first lane alone, the two builds in turn, the order flipping each round
# (two XL jobs at once on CCD0 read search 1.8× slow). 6 rounds (4 for XL), so each build has each side equally often.
# abcmp.py takes each cell's A/B from the rounds' paired ratios, balanced over the sides. With B2_PROFILE=off they take
# turns on the lanes instead (ABBA). A REF of another layout (OUT/layout.txt: "ccd0", or none for the four lanes over
# both CCDs before) is refused, as one of another profile, unless B2_AB_ONLY.
# Records made under different profiles don't mix: OUT/profile.txt holds the profile's id ("on" before BEN-04's clock
# floor, "on+clock" since; off without it), and a REF made another way is refused (a REF without the file is from
# before the profile: off). Elsewhere both run in one hyperfine call. The reference ada ("adaref") stays out of the merge: its new/old
# against REF is the machine's drift, and ada's against it the A/B. Writes OUT/compare.md (abcmp.py),
# OUT/doctables.md (the vault notes' tables from the merged records) and OUT/run.log (times, Tctl and clocks).
# B2_MERGE_ONLY=1 redoes only the merge, compare.md and doctables.md of an existing OUT. B2_AB_ONLY=1 runs the A/B
# and writes compare.md, with no merge and no doctables.md, so it takes a REF made with or without the profile;
# old/REF then compares across the two, which isn't drift.
# B2_AB_PAR=1, with B2_AB_ONLY and the profile, runs the lanes' rounds two at a time (2026-10-31): the lanes mostly
# wait out their quiet windows (load ~0.1), so each even round runs beside the odd one before it, on the same lanes'
# SMT siblings (1,3,5,7 and 9,11,13,15, held ready too; the XL jobs still one at a time). Each build then has a job on every core at once, so contention lands on
# both sides alike and the paired ratios hold; the even round's lanes run their jobs in reverse order, so that the
# two builds' bursts on a core don't line up document for document, and copy their documents under OUT/run/par/
# (b2 keeps a job's copy by its name, and the reload test rewrites it). A/B only: cells measured this way don't
# compare with a round-at-a-time run's, so they're never merged.
S=${B2_ROOT:-${0:A:h}/root}; REF=${${1:?usage: rerun.zsh REF OUT}:A}; OUT=${${2:?usage: rerun.zsh REF OUT}:A}   # absolute, as in full.zsh
H=${0:A:h}; B2=$H/target/release/b2
L=(0,2,4,6 8,10,12,14)   # as lanes.py: CCD0's two lanes
L2=(1,3,5,7 9,11,13,15)   # their SMT siblings, for B2_AB_PAR
mark() {
  local c clocks=()
  for c in 0 2 8 16 24; do clocks+=$(( $(</sys/devices/system/cpu/cpu$c/cpufreq/scaling_cur_freq) / 1000 )); done
  print "$1 $(date +%T) Tctl: $(sensors 2>/dev/null | awk '/^Tctl:/ { print $2 }')  clocks(0,2,8,16,24): $clocks" >> $OUT/run.log
}
if [[ -z $B2_MERGE_ONLY ]]; then
[[ -e $OUT ]] && { echo "rerun: $OUT exists" >&2; exit 1 }
PROFILE=${B2_PROFILE:-on}; REF_PROFILE=$(cat $REF/profile.txt 2>/dev/null || print off)
PAR=${B2_AB_PAR:-}
[[ -z $PAR || ( -n ${B2_AB_ONLY:-} && $PROFILE == on ) ]] || { echo "rerun: B2_AB_PAR needs B2_AB_ONLY and the profile" >&2; exit 1 }
mkdir -p $OUT/{run,b2}; PROFILE_ID=off
if [[ $PROFILE == on ]]; then   # restored when this script exits
  sudo -n /usr/local/sbin/b2-bench-profile on ${(j:,:)L}${PAR:+,${(j:,:)L2}} $$ || exit 1
  PROFILE_ID=$(cat /run/b2-bench-profile/id 2>/dev/null)
  [[ -n $PROFILE_ID ]] || { echo "rerun: the installed b2-bench-profile is older than bench/b2's (no id): reinstall it" >&2; exit 1 }
fi
[[ $PROFILE_ID == $REF_PROFILE || -n ${B2_AB_ONLY:-} ]] || { echo "rerun: REF was measured with the profile $REF_PROFILE, this run $PROFILE_ID" >&2; exit 1 }
print $PROFILE_ID > $OUT/profile.txt
REF_LAYOUT=$(cat $REF/layout.txt 2>/dev/null || print 4lanes)
[[ $REF_LAYOUT == ccd0 || -n ${B2_AB_ONLY:-} ]] || { echo "rerun: REF ran on another lane layout ($REF_LAYOUT; full.zsh runs CCD0's two lanes since BEN-002): run a full run, or B2_AB_ONLY=1" >&2; exit 1 }
print ccd0 > $OUT/layout.txt
cp ${ADA:?set ADA} $OUT/ada; cp ${ADA_REF:?set ADA_REF} $OUT/ada-ref
export ADA=$OUT/ada ADA_REF=$OUT/ada-ref
# both copies read back from disk, as full.zsh has every viewer's (BEN-001); B2_DROP_CACHES=off skips it
DROP=${B2_DROP_CACHES:-/usr/local/sbin/oxasm-drop-caches}
if [[ $DROP != off ]]; then
  sudo -n $DROP > $OUT/drop-caches.txt 2>&1 || { echo "rerun: sudo -n $DROP failed; or set B2_DROP_CACHES=off" >&2; exit 1 }
fi
cp $REF/lane*.json(N) $OUT/
print "ada $(sha256sum < $ADA | cut -c1-16), adaref $(sha256sum < $ADA_REF | cut -c1-16); ref $REF${PAR:+; rounds two at a time (B2_AB_PAR)}" > $OUT/run.log

# the lanes: each of ada's jobs, as mkspec.py writes it now, split into one-run jobs, one spec per round and lane
# (OUT/run/r<round>/lane0.json and lane1.json at once, then lane-xl.json alone on lane 0's cores)
python3 -I - $H $REF $OUT/run $PROFILE "$PAR" <<'EOF' || exit 1
import json, os, shutil, subprocess, sys
h, ref, out, profile, par = sys.argv[1:]
def mk(binary, name, runs, docs):
    env = os.environ | {"ADA": binary}
    jobs = json.loads(subprocess.check_output([sys.executable, "-I", f"{h}/mkspec.py", str(runs), docs, "ada"], env=env))
    return [(j | {"name": j["name"].replace("ada-", name + "-", 1), "runs": 1}, runs) for j in jobs]
ref_jobs = {j["name"] for f in os.listdir(ref) if f.startswith("lane") and f.endswith(".json")
            for j in json.load(open(f"{ref}/{f}"))}
# even round counts, so that side by side each build takes each lane equally often
rounds = (6, 4) if profile == "on" else (5, 3)
new = mk(os.environ["ADA"], "ada", rounds[0], "s,m,l"); new_xl = mk(os.environ["ADA"], "ada", rounds[1], "xl")
old = mk(os.environ["ADA_REF"], "adaref", rounds[0], "s,m,l"); old_xl = mk(os.environ["ADA_REF"], "adaref", rounds[1], "xl")
if any(j["name"] not in ref_jobs for j, _ in new + new_xl):
    sys.exit("rerun: a job of ada's isn't on REF's lanes")
NAMES = ["lane0", "lane1", "lane-xl"]
for r in range(rounds[0]):
    lanes = [[], [], []]
    for k, ((a, n), (b, _)) in enumerate(zip(new, old)):
        if r >= n:
            continue
        if profile == "on":   # at once, one build a lane, the lanes swapping each round
            lanes[0].append(a if r % 2 == 0 else b); lanes[1].append(b if r % 2 == 0 else a)
        else:                 # in turn, the order flipping each round, the jobs dealt over the two lanes
            lanes[k % 2] += [a, b] if r % 2 == 0 else [b, a]
    for (a, n), (b, _) in zip(new_xl, old_xl):   # XL: alone on CCD0, in turn, the order flipping each round
        if r < n:
            lanes[2] += [a, b] if r % 2 == 0 else [b, a]
    if par and r % 2 == 1:   # beside the round before: the jobs reversed, their documents copied from par/corpus;
        # not the XL lane, which runs a round at a time
        os.makedirs(f"{out}/par/corpus", exist_ok=True)
        for j, _ in new:
            shutil.copy(f"{j['cwd']}/{j['file']}", f"{out}/par/corpus/")
        lanes[:2] = [[j | {"cwd": f"{out}/par/corpus"} for j in reversed(l)] for l in lanes[:2]]
    os.makedirs(f"{out}/r{r + 1}", exist_ok=True)
    for name, l in zip(NAMES, lanes):
        if l:
            json.dump(l, open(f"{out}/r{r + 1}/{name}.json", "w"), indent=1)
    print(f"round {r + 1}:", "; ".join(f"{name}: " + " ".join(j["name"] for j in l) for name, l in zip(NAMES, lanes) if l))
EOF
mark "b2 start"
rounds=($OUT/run/r<->(n)); step=${PAR:+2}; step=${step:-1}
for (( k = 1; k <= $#rounds; k += step )); do
  pids=(); names=()
  for (( j = 0; j < step && k + j <= $#rounds; j++ )); do
    rd=$rounds[k+j]; names+=${rd:t}
    for i in 0 1; do
      [[ -s $rd/lane$i.json ]] || continue
      cpus=$L[i+1]; (( j )) && cpus=$L2[i+1]
      taskset -c $cpus $B2 run $rd/lane$i.json $rd/lane$i.jsonl > $rd/lane$i.log 2>&1 &
      pids+=$!
    done
  done
  rc=0; for p in $pids; do wait $p || rc=1; done
  # the XL jobs after the lanes, alone on lane 0's cores, a round at a time even with B2_AB_PAR
  for (( j = 0; j < step && k + j <= $#rounds; j++ )); do
    rd=$rounds[k+j]
    [[ -s $rd/lane-xl.json ]] || continue
    taskset -c $L[1] $B2 run $rd/lane-xl.json $rd/lane-xl.jsonl > $rd/lane-xl.log 2>&1 || rc=1
  done
  mark "b2 ${(j:+:)names} rc=$rc"
done
# each record tagged with its round, for abcmp.py's paired ratios
python3 -I - $OUT/run/r<->/lane*.jsonl(N) > $OUT/run/new.jsonl <<'EOF'
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
  [[ $PROFILE_ID == $REF_PROFILE ]] || print "\nold/REF: REF was measured with the profile $REF_PROFILE, this run $PROFILE_ID: not drift" >> $OUT/compare.md
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
