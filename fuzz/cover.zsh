#!/usr/bin/env zsh
# cover.zsh COMMAND …: a corpus replayed on 12 cores, minimized by set cover, and fuzzed by 12 workers on a
# twelfth of it each (F-020). libFuzzer replays a corpus in one process, for a cmin (`-set_cover_merge=1`) and
# for `-fork`'s seed pass, and its set cover counts every remaining input again for each pick: on 2026-11-19
# `session`'s cmin took 1 h 52 (55 min the replay at 28 inputs a second, the set cover as long again), and
# three 10-minute smokes' seed passes 45 minutes, 11 of the 12 cores idle.
#
# The replay: each of 12 shards of the inputs runs libFuzzer's own inner merge step (`-merge_inner=2`: every
# feature of each input to the shard's control file; it resumes after an input that crashes, times out or
# runs out of memory, which lands in the findings dir), one shard a physical core. `cargo xtask cover` then
# picks from the control files what `-set_cover_merge=1` would, lazily, in seconds (xtask/src/cover.rs).
#
# The smoke's workers are plain libFuzzer processes, not `-fork`'s jobs. A fork parent knows the features of
# what it replayed itself, and it replays each job's new inputs before it starts the next job: from a sample
# of the corpus (2,000 of the cover's best, 81% of `session`'s features) the jobs bring back inputs the rest
# of the corpus already has, and the workers waited on the parent's replays three quarters of the time
# (2026-11-21: 56 K sessions in 10 min against 209 K from the whole corpus). A plain worker keeps what is new
# to its own twelfth, in a dir of its own, with no one to wait on; at the end those inputs are replayed and
# kept only where they add to the whole corpus's features (`cover --covered`, against the control files of
# the replay it began with). Only those whose features file (`-features_dir`: what the input added to its
# worker) holds a feature the corpus lacks are replayed (`cover --new`), as `-fork`'s parent filters its
# jobs' inputs. The workers don't share inputs while they run.
#
#   cover.zsh cmin SPEC…                  each corpus minimized in place: the inputs that reach all its
#                                         features, the rest dropped (a CI round's inputs brought home are
#                                         in no other corpus, so this is their only copy); one after another
#   cover.zsh replay SPEC                 the replay and the cover alone: target/cover/NAME/corpus.picks,
#                                         best first, the corpus left as it is
#   cover.zsh smoke SPEC SECONDS [SEEDS…] the corpus replayed and covered, the cover and the SEEDS dirs'
#                                         files (and the target's own seed dirs, ci.zsh flags) dealt to 12
#                                         workers, which fuzz for SECONDS after their seed pass (one that
#                                         finds a crash, timeout or OOM stops there, the others go on);
#                                         then their new inputs, and the seeds, that add to the corpus's
#                                         features are copied into it. Findings and logs in out/cover/NAME/
#
# SPEC is a target (`session`: corpus/session) or `tokens:LANG` (corpus/tokens-LANG, run as
# ADA_FUZZ_LANGS=LANG, as lang.zsh's). A run's lists, logs, picks and replay findings are in target/cover/NAME/
# until the next (its control files are removed at the end: print's are 5 GB); an input that failed in a
# replay is a finding (`failed:` in its cover log). Each command builds the target first, as CI does
# (`ci.zsh build`: `tokens` without trace-compares, F-019; COVER_BUILD=0 skips it). Timers: an input times
# out after COVER_INPUT_TIMEOUT seconds (30, as the rounds' -timeout), and runs over 3072 MB of RSS
# (COVER_RSS_MB) as an OOM; bench/work.zsh caps the whole job (WORK_FUZZ_MAX).
# e.g. bench/work.zsh fuzz "session cmin" /home/junkdog/opt/dev/rust/ada-md/fuzz/cover.zsh cmin session
#      bench/work.zsh fuzz "print smoke" /home/junkdog/opt/dev/rust/ada-md/fuzz/cover.zsh smoke print 600 SEEDS
setopt err_exit pipe_fail no_unset extended_glob
zmodload zsh/datetime
zmodload -F zsh/files b:zf_ln
F=${0:A:h} CALLER=$PWD
cd $F
export TMPDIR=$F/target/tmp
mkdir -p $TMPDIR
# twelve physical cores, the even CPUs, as the runbook's local rounds
CORES=(0 2 4 6 8 10 12 14 16 18 20 22)
INPUT_TIMEOUT=${COVER_INPUT_TIMEOUT:-30}
RSS_MB=${COVER_RSS_MB:-3072}

die() { print -u2 -r -- "cover.zsh: $*"; exit 1 }
abs() { local p; for p in $@; do [[ $p == /* ]] && print -r -- $p || print -r -- $CALLER/$p; done }
clock() { strftime %T $EPOCHSECONDS }
since() { printf '%d:%02d' $(( ($EPOCHSECONDS - $1) / 60 )) $(( ($EPOCHSECONDS - $1) % 60 )) }

# setup SPEC: the target, its binary, flags and extra corpus dirs, the corpus, and the work dir
typeset t lang corpus name bin W
typeset -a args extra
setup() {
  t=${1%%:*} lang=
  [[ $1 == *:* ]] && lang=${1#*:}
  local -a f=(${(f)"$(zsh $F/ci.zsh flags $t)"})
  local cut=${f[(i)--]}
  (( cut <= ${#f} )) || die "no target $t"
  args=(${f[1,cut-1]}) extra=(${f[cut+1,-1]})
  if [[ -n $lang ]]; then
    name=$t-$lang
    export ADA_FUZZ_LANGS=$lang
  else
    name=$t
    unset ADA_FUZZ_LANGS
  fi
  corpus=corpus/$name
  bin=$(zsh $F/ci.zsh binary $t)
  W=target/cover/$name
  mkdir -p $corpus
}

build() {
  [[ ${COVER_BUILD:-1} == 0 ]] && return
  zsh $F/ci.zsh build $t || die "build"
  (cd $F/.. && cargo build -q -p xtask) || die "xtask"
}

# shard CF LOG CPU: a shard's inner merge on its core, again after each input that ends it, while each
# attempt gets further (an attempt writes at least the next input's STARTED line)
shard() {
  local cf=$1 log=$2 cpu=$3
  local -i rc size last=-1
  while :; do
    size=$(stat -c %s $cf)
    (( size > last )) || { print -r -- "$cf: no progress, rc $rc" >> $log; return 1 }
    last=$size rc=0
    taskset -c $cpu $bin $args -merge_inner=2 -merge_control_file=$cf -timeout=$INPUT_TIMEOUT \
      -rss_limit_mb=$RSS_MB -artifact_prefix=$W/findings/ >> $log 2>&1 || rc=$?
    (( rc == 0 )) && return 0
  done
}

# replay LABEL INPUT…: the inputs (by size, smallest first) dealt to 12 shards and replayed side by side,
# into $W/LABEL.cf.N; logs $W/LABEL.replay.N.log
replay() {
  local label=$1 i l t0=$EPOCHREALTIME
  shift
  mkdir -p $W/findings
  print -rl -- $@ | awk -v w=$W/$label -v n=${#CORES} '{ print > (w ".list." ((NR - 1) % n)) }'
  local -a pids
  for l in $W/$label.list.*(N); do
    i=${l##*.}
    { wc -l < $l; print 0; cat $l } > $W/$label.cf.$i
    shard $W/$label.cf.$i $W/$label.replay.$i.log $CORES[i+1] & pids+=($!)
  done
  local p bad=0
  for p in $pids; do wait $p || bad=1; done
  (( bad == 0 )) || die "a shard stopped early: $W/$label.replay.*.log"
  printf '%s: %d inputs replayed in %.0f s on %d cores (%s of control files), %s\n' $label $# \
    $(( EPOCHREALTIME - t0 )) ${#pids} "$(du -ch $W/$label.cf.* | tail -1 | cut -f1)" "$(clock)"
}

# cover LABEL [BASE]: `xtask cover` over LABEL's control files (against BASE's, covered, when given) into
# $W/LABEL.picks, best first; the summary to $W/LABEL.cover.log and stdout
cover() {
  local label=$1 base=${2:-} t0=$EPOCHREALTIME l
  local -a cmd=(cargo run -q -p xtask -- cover)
  if [[ -n $base ]]; then
    for l in $F/$W/$base.cf.*(N); do cmd+=(--covered $l); done
  fi
  (cd $F/.. && $cmd $F/$W/$label.cf.*) > $W/$label.picks 2> $W/$label.cover.log || {
    cat $W/$label.cover.log >&2
    die "xtask cover"
  }
  grep -v '^failed: ' $W/$label.cover.log || true
  printf '%s: covered in %.1f s\n' $label $(( EPOCHREALTIME - t0 ))
  local -a failed=(${(f)"$(grep '^failed: ' $W/$label.cover.log || true)"})
  (( ${#failed} == 0 )) || print -r -- "${#failed} inputs failed in the replay: $W/$label.cover.log, $W/findings/"
}

cmin1() {
  setup $1
  build
  local -i t0=$EPOCHSECONDS
  local was=$(du -sh $corpus | cut -f1)
  local -a inputs=($corpus/*(N.oL))
  (( ${#inputs} )) || die "$corpus is empty"
  rm -rf $W
  print -r -- "$name: replaying $corpus on ${#CORES} cores, $(clock)"
  replay corpus $inputs
  cover corpus
  rm -f $W/corpus.cf.*
  local -a picks=(${(f)"$(<$W/corpus.picks)"})
  (( ${#picks} )) || die "$name: the cover kept nothing; $corpus left as it was"
  rm -rf $corpus.cover $W/old
  mkdir $corpus.cover
  zf_ln -- $picks $corpus.cover
  local -a kept=($corpus.cover/*(N.))
  (( ${#kept} == ${#picks} )) || die "$name: ${#kept} of ${#picks} inputs linked; $corpus left as it was"
  mv $corpus $W/old
  mv $corpus.cover $corpus
  rm -rf $W/old
  print -r -- "=== $name: ${#inputs} → ${#picks} inputs ($was → $(du -sh $corpus | cut -f1)) in $(since $t0)," \
    "$(clock)"
}

cmin() {
  local s
  for s in $@; do cmin1 $s; done
}

smoke() {
  setup $1
  local secs=$2
  [[ $secs == <1-> ]] || die "seconds: $secs"
  shift 2
  local -a seeds=($(abs $@) ${extra:A})
  build
  local -i t0=$EPOCHSECONDS n=${#CORES} k
  local o=out/cover/$name
  rm -rf $W $o
  mkdir -p $W $o/findings
  local -a inputs=($corpus/*(N.oL)) picks=() seedfiles=(${^seeds}/**/*(N.))
  print -r -- "$name: ${#inputs} inputs in $corpus, ${#seedfiles} in the seeds (${(j:, :)seeds:-none}), $(clock)"
  if (( ${#inputs} )); then
    replay corpus $inputs
    cover corpus
    picks=(${(f)"$(<$W/corpus.picks)"})
  fi
  local -i t1=$EPOCHSECONDS

  # the cover's inputs dealt to the shards best first, so each has some of every rank; then the seeds, linked
  # under names of their own (two seed dirs may hold the same name)
  for k in {1..$n}; do mkdir $W/shard.$k $o/new.$k $o/feat.$k; done
  for (( k = 1; k <= ${#picks}; k++ )); do zf_ln -- $picks[k] $W/shard.$(( (k - 1) % n + 1 ))/; done
  for (( k = 1; k <= ${#seedfiles}; k++ )); do
    zf_ln -s -- ${seedfiles[k]:A} $W/shard.$(( (k + ${#picks} - 1) % n + 1 ))/seed-$k
  done

  local -a pids
  for k in {1..$n}; do
    setsid taskset -c $CORES[k] $bin $args -timeout=30 -rss_limit_mb=$RSS_MB -reload=0 -print_final_stats=1 \
      -features_dir=$o/feat.$k -artifact_prefix=$o/findings/ $o/new.$k $W/shard.$k > $o/worker.$k.log 2>&1 &
    pids+=($!)
  done
  trap "kill -TERM $pids 2> /dev/null" INT TERM EXIT
  local p
  # the seed pass: until every worker has loaded its shard, or stopped
  for k in {1..$n}; do
    while kill -0 $pids[k] 2> /dev/null && ! grep -qa 'INITED' $o/worker.$k.log; do sleep 1; done
  done
  local -i t2=$EPOCHSECONDS
  print -r -- "$name: ${#picks} inputs of the cover and ${#seedfiles} seeds loaded by $n workers, $(clock)" \
    "($(( t2 - t1 )) s)"
  # SECONDS of fuzzing, or less where every worker has stopped at a finding
  local -i deadline=$(( EPOCHSECONDS + secs )) alive
  while (( EPOCHSECONDS < deadline )); do
    alive=0
    for p in $pids; do kill -0 $p 2> /dev/null && (( alive += 1 )); done
    (( alive )) || break
    sleep 1
  done
  kill -TERM $pids 2> /dev/null || true
  for p in $pids; do wait $p || true; done
  trap - INT TERM EXIT
  local -i t3=$EPOCHSECONDS

  # what the workers kept that may add to the corpus's features (by their features files), and the seeds,
  # replayed and covered against the corpus
  local -a kept=($o/new.*/*(N.)) cands=() adds=()
  local -i runs=0 r
  for k in {1..$n}; do
    r=$(grep -a 'stat::number_of_executed_units' $o/worker.$k.log | awk '{ print $2 }' | tail -1)
    (( runs += r ))
  done
  if (( ${#picks} && ${#kept} )); then
    local -a cmd=(cargo run -q -p xtask -- cover)
    local l
    for l in $F/$W/corpus.cf.*(N); do cmd+=(--covered $l); done
    for k in {1..$n}; do cmd+=(--new $F/$o/new.$k $F/$o/feat.$k); done
    (cd $F/.. && $cmd) > $W/prefilter.list 2> $W/prefilter.log || { cat $W/prefilter.log >&2; die "prefilter" }
    cat $W/prefilter.log
    cands=(${(f)"$(<$W/prefilter.list)"})
  else
    cands=($kept)
  fi
  cands+=($seedfiles)
  if (( ${#cands} )); then
    replay new $cands
    if (( ${#picks} )); then cover new corpus; else cover new; fi
    adds=(${(f)"$(<$W/new.picks)"})
  fi
  local sum file
  for file in $adds; do
    sum=$(sha1sum < $file)
    cp --update=none -- $file $corpus/${sum%% *}
  done
  rm -f $W/*.cf.*
  local -a found=($o/findings/^slow-unit-*(N)) slow=($o/findings/slow-unit-*(N))
  local -a stopped=(${(f)"$(grep -la 'ERROR: libFuzzer\|==ERROR' $o/worker.*.log || true)"})
  print -r -- "=== $name, $(clock): $runs runs; the workers kept ${#kept} inputs, ${#cands} replayed with the" \
    "${#seedfiles} seeds, ${#adds} added to $corpus; ${#found} findings, ${#slow} slow units in $o/findings" \
    "(${#stopped} workers stopped at one)"
  print -r -- "$(since $t0) from start to end: replay and cover $(( t1 - t0 )) s, seed pass $(( t2 - t1 )) s," \
    "fuzzing $(( t3 - t2 )) s, the new inputs' replay and cover $(( EPOCHSECONDS - t3 )) s"
  print -r -- "logs: $F/$o/worker.*.log"
}

case ${1:-} in
  replay)
    (( $# == 2 )) || die "replay SPEC"
    setup $2; build; rm -rf $W
    inputs=($corpus/*(N.oL))
    (( ${#inputs} )) || die "$corpus is empty"
    replay corpus $inputs; cover corpus; rm -f $W/corpus.cf.* ;;
  cmin) (( $# >= 2 )) || die "cmin SPEC…"; shift; cmin $@ ;;
  smoke) (( $# >= 3 )) || die "smoke SPEC SECONDS [SEEDS…]"; shift; smoke $@ ;;
  *) die "cmin, replay or smoke" ;;
esac
