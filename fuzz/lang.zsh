#!/usr/bin/env zsh
# lang.zsh COMMAND …: `tokens` or `parse` fuzzed on some of ada-parse's languages alone (ADA_FUZZ_LANGS,
# fuzz_targets/langs.rs), each language's inputs in a corpus of its own, corpus/TARGET-LANG. A smoke for a
# new or changed lexer then neither replays the whole corpus through every lexer first nor shares its runs
# with them. LANGS is a comma list of `Lang`s as the target names them, in lowercase (`python`, `cpp,c`),
# or `all`. Run it under bench/exclusive.zsh, with absolute paths.
#
#   lang.zsh corpus TARGET LANGS [SEEDS…]   each language's corpus/TARGET-LANG: what it has, and the inputs
#                                           of corpus/TARGET and the SEEDS dirs that reach code of that
#                                           language it doesn't (libFuzzer's -merge=1, filtered to the
#                                           language); 12 languages at a time, logs in out/lang/
#   lang.zsh smoke TARGET LANGS SECONDS [SEEDS…]
#                                           12 forks for SECONDS after the seed pass, on the languages'
#                                           corpora and the SEEDS dirs (read only); the inputs it adds go
#                                           to the first language's corpus and are copied to corpus/TARGET
#                                           (the whole one, the CI rounds' seed); findings and the log in
#                                           out/lang/TARGET-LANGS/; its RC is 72, libFuzzer's on SIGTERM
#   lang.zsh cmin TARGET LANGS              each language's corpus/TARGET-LANG minimized (libFuzzer's
#                                           -merge=1 into an empty dir, filtered to the language), the
#                                           old one replaced; 12 languages at a time, logs in out/lang/.
#                                           Its inputs are all in corpus/TARGET too, so nothing is lost
#   lang.zsh names TARGET                   the languages TARGET takes
#
# Each builds the target first, as `cargo fuzz run -s none -a` does. A language new to the target starts
# empty, so `corpus` with its seeds (census files, a dir each) comes first.
# e.g. bench/exclusive.zsh "py smoke" /home/junkdog/opt/dev/rust/ada-md/fuzz/lang.zsh smoke tokens python 300
setopt err_exit pipe_fail no_unset extended_glob
F=${0:A:h} CALLER=$PWD
cd $F
export TMPDIR=$F/target/tmp
mkdir -p $TMPDIR out/lang
# twelve physical cores, the even CPUs, as the runbook's local rounds
CORES=0,2,4,6,8,10,12,14,16,18,20,22

die() { print -u2 "lang.zsh: $*"; exit 1 }

typeset -a args
typeset bin
setup() {
  case $1 in
    tokens) args=(-dict=tokens.dict -max_len=65536) ;;
    parse) args=(-dict=parse.dict -max_len=4096) ;;
    *) die "no target $1 (tokens or parse)" ;;
  esac
  bin=target/x86_64-unknown-linux-gnu/release/$1
}

build() {
  (cd $F/.. && cargo +nightly fuzz build -s none -a $1 --target x86_64-unknown-linux-gnu) || die "build"
}

# The target's own list (`ADA_FUZZ_LANGS=list`), from one input that reaches the filter in either.
names() {
  print -n '\0x' > $TMPDIR/lang-probe
  ADA_FUZZ_LANGS=list $bin $TMPDIR/lang-probe 2> /dev/null
}

# LANGS checked against the target's list; `all` is all of it.
typeset -a langs
pick() {
  local -a known=($(names))
  (( ${#known} )) || die "no languages from $bin"
  if [[ $1 == all ]]; then
    langs=($known)
  else
    langs=(${(s:,:)1})
    local l
    for l in $langs; do (( ${known[(Ie)$l]} )) || die "no $l in $t; one of ${(j:, :)known}"; done
  fi
}

abs() { local p; for p in $@; do [[ $p == /* ]] && print -r -- $p || print -r -- $CALLER/$p; done }

# merge1 TARGET LANG SEEDS…: one language's merge, as `corpus` runs them side by side.
merge1() {
  local t=$1 l=$2
  shift 2
  setup $t
  local d=corpus/$t-$l log=out/lang/merge-$t-$l.log
  mkdir -p $d corpus/$t
  local -a was=($d/*(N))
  local rc=0
  ADA_FUZZ_LANGS=$l $bin $args -merge=1 -timeout=30 -rss_limit_mb=3072 $d corpus/$t $@ > $log 2>&1 || rc=$?
  local -a now=($d/*(N))
  print -r -- "$l: ${#was} → ${#now} inputs, $(du -sh $d | cut -f1), rc $rc, $(date +%T)"
}

# cmin1 TARGET LANG: one language's corpus minimized, as `cmin` runs them side by side.
cmin1() {
  local t=$1 l=$2
  setup $t
  local d=corpus/$t-$l log=out/lang/cmin-$t-$l.log
  [[ -d $d ]] || return 0
  local m=$TMPDIR/cmin-$t-$l
  rm -rf $m
  mkdir -p $m
  local -a was=($d/*(N))
  local rc=0
  ADA_FUZZ_LANGS=$l $bin $args -merge=1 -timeout=30 -rss_limit_mb=3072 -artifact_prefix=out/lang/cmin-$t-$l- \
    $m $d > $log 2>&1 || rc=$?
  local -a now=($m/*(N))
  if (( rc == 0 && ${#now} > 0 )); then
    rm -rf $d
    mv $m $d
  else
    rm -rf $m
  fi
  print -r -- "$l: ${#was} → ${#now} inputs, $(du -sh $d | cut -f1), rc $rc, $(date +%T)"
}

cmin() {
  t=$1
  setup $t
  build $t
  pick $2
  print "$t: minimizing ${#langs} languages' corpora, $(date +%T)"
  print -rl -- $langs | taskset -c $CORES xargs -P 12 -I{} zsh $F/lang.zsh cmin1 $t {}
}

corpus() {
  t=$1
  setup $t
  build $t
  pick $2
  shift 2
  local -a seeds=($(abs $@)) all=(corpus/$t/*(N))
  print "$t: merging ${#langs} languages from corpus/$t (${#all} inputs)${seeds:+ and ${(j:, :)seeds}}," \
    "$(date +%T)"
  print -rl -- $langs | taskset -c $CORES xargs -P 12 -I{} zsh $F/lang.zsh merge1 $t {} $seeds
}

smoke() {
  t=$1
  local secs=$3
  [[ $secs == <1-> ]] || die "seconds: $secs"
  setup $t
  build $t
  pick $2
  shift 3
  local -a seeds=($(abs $@)) dirs=(corpus/$t-${^langs})
  mkdir -p $dirs corpus/$t
  local o=out/lang/$t-${(j:,:)langs}
  rm -rf $o
  mkdir -p $o/findings
  local -a before=($dirs[1]/*(N:t)) after added found slow
  print "$t on ${(j:,:)langs}: ${#before} inputs in $dirs[1], started at $(date +%T)"
  ADA_FUZZ_LANGS=${(j:,:)langs} setsid taskset -c $CORES $bin $args -fork=12 -timeout=30 -rss_limit_mb=3072 \
    -artifact_prefix=$o/findings/ $dirs $seeds > $o/log.txt 2>&1 &
  local pid=$! rc=0
  trap "kill -TERM -- -$pid 2> /dev/null" INT TERM EXIT
  # the seed pass, bounded by the fuzzer itself
  while kill -0 $pid 2> /dev/null && ! grep -q 'starting to fuzz' $o/log.txt; do sleep 1; done
  print "seed pass done at $(date +%T)"
  kill -0 $pid 2> /dev/null && sleep $secs
  kill -TERM -- -$pid 2> /dev/null || true
  wait $pid || rc=$?
  trap - INT TERM EXIT
  after=($dirs[1]/*(N:t))
  added=(${after:|before})
  print -rl -- ${added/#/$dirs[1]/} | xargs -r cp --update=none -t corpus/$t --
  found=($o/findings/^slow-unit-*(N)) slow=($o/findings/slow-unit-*(N))
  print "=== RC=$rc at $(date +%T): ${#added} inputs added (copied to corpus/$t), ${#found} findings," \
    "${#slow} slow units"
  grep -a '^#[0-9]*: cov:' $o/log.txt | sed -n '1p;$p' || true
  print "log: $F/$o/log.txt"
}

typeset t
case ${1:-} in
  corpus) (( $# >= 3 )) || die "corpus TARGET LANGS [SEEDS…]"; shift; corpus $@ ;;
  smoke) (( $# >= 4 )) || die "smoke TARGET LANGS SECONDS [SEEDS…]"; shift; smoke $@ ;;
  cmin) (( $# == 3 )) || die "cmin TARGET LANGS"; shift; cmin $@ ;;
  names) (( $# == 2 )) || die "names TARGET"; setup $2; build $2; names ;;
  merge1) shift; merge1 $@ ;;
  cmin1) shift; cmin1 $@ ;;
  *) die "corpus, cmin, smoke or names" ;;
esac
