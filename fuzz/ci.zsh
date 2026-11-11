#!/usr/bin/env zsh
# ci.zsh COMMAND …: fuzz rounds on GitHub's runners (F-014, .github/workflows/fuzz.yml), each
# target's flags as the local rounds run them, and the corpus a round starts from and leaves.
#
#   ci.zsh plan 'session=5 print=3 …'  the workers, one per shard, and the targets, as the
#                                      workflow's JSON outputs; `tokens=N` deals its languages
#                                      to the N workers by weight (a worker's `langs`)
#   ci.zsh fetch TARGET                corpus/TARGET from the newest `corpus-TARGET` artifact, else
#                                      the `fuzz-corpus` release's TARGET.tar.zst, else empty
#                                      (`tokens`: corpus/tokens-LANG, a dir a language)
#   ci.zsh run TARGET SECONDS [LANGS]  fuzz on every CPU; the inputs it adds to out/new, crashes,
#                                      timeouts, OOMs and slow units to out/findings, the log to
#                                      out/TARGET.log. Fails on a finding other than a slow unit.
#                                      `tokens` takes the worker's languages (a comma list), dealt
#                                      to a lane a CPU; a lane fuzzes its languages one after
#                                      another, each alone on its own corpus
#   ci.zsh union TARGET [DIR]          corpus/TARGET (fetched) with the inputs anywhere under DIR,
#                                      as out/TARGET.tar.zst (`tokens`: DIR's tokens-LANG dirs to
#                                      their languages')
#   ci.zsh pack                        the local corpora as target/ci-seed/TARGET.tar.zst, for the
#                                      `fuzz-corpus` release a first round starts from
#   ci.zsh langs [BIN]                 fails unless `tokens`' languages (BIN's list) are LANGS's
#
# The corpus's store between rounds is the `corpus-TARGET` artifact (kept 90 days), so nothing
# is written to the repo or its releases. Inputs are named by their hash, so a union is a copy.
# `tokens`' store is its languages' corpora, corpus/tokens-LANG (F-017): every input runs
# through each of its 40 lexers otherwise, and one merge of them all takes hours.
setopt err_exit pipe_fail no_unset extended_glob
F=${0:A:h} CALLER=$PWD
cd $F

die() { print -u2 "ci.zsh: $*"; exit 1 }
summary() { print -r -- "$*" >> ${GITHUB_STEP_SUMMARY:-/dev/null} }

# `tokens`' languages (F-017): the edges a language's corpus reaches, its weight (its share of a
# worker's time, so a lexer of more code is fuzzed longer), and the seconds of one core its
# corpus's seed pass takes, both from its set-cover cmin on the desk machine (F-017's
# experiment 83, 2026-11-07). A new language goes here first: the build checks the list
# against the binary's (`langs`).
LANGS=(
  typescriptreact 17631 149  typescript 17082 143  csharp 13680 125     java 9442 43
  scss 8800 80               rust 7600 27          html 7371 6          python 7217 59
  cpp 5807 26                makefile 5501 28      javascript 5106 26   kotlin 4704 29
  bash 4671 29               go 4666 28            scheme 3996 10       racket 3987 10
  emacslisp 3352 8           lisp 3211 7           css 3058 8           c 2870 15
  powershell 2668 10         lua 2646 24           clojure 2102 5       xml 1813 9
  gitattributes 1661 3       toml 1536 1           sql 1280 2           log 1127 3
  yaml 1118 2                dotenv 1000 1         gitignore 822 1      dockerfile 724 1
  ini 689 1                  knownhosts 677 1      json 673 1           fstab 592 1
  gitlink 589 1              gitmailmap 490 1      properties 429 1     diff 281 1
  sshconfig 677 1            sshdconfig 677 1     crontab 677 1        nginx 677 1
  php 677 1                  swift 2646 24        cmake 2646 24        ruby 2646 24
  cmakecache 592 1           terraform 2646 24    objc 2870 15         objcpp 2870 15
  scala 2646 24              haskell 2646 24      literatehaskell 677 1
  cmakecheader 677 1         cmakecppheader 677 1 groovy 2646 24       elixir 2646 24
  nix 2646 24
)
typeset -A weight pass
for l w p in $LANGS; do weight[$l]=$w pass[$l]=$p; done

# The target's dictionary, input size and corpus dirs beyond its own, as round 12 ran them
# (and the differential targets as their docs do).
typeset -a args dirs
flags() {
  dirs=()
  case $1 in
    session|render) args=(-dict=markdown.dict -max_len=16384) ;;
    print) args=(-dict=markdown.dict -max_len=16384); dirs=(../tests) ;;
    diagram) args=(-dict=diagram.dict) ;;
    stream|tokens) args=(-dict=tokens.dict -max_len=65536) ;;
    images) args=(-dict=images.dict -max_len=65536) ;;
    probe) args=(-max_len=4096) ;;
    sixel) args=(-max_len=8192) ;;
    roots) args=(-max_len=1024) ;;
    *) die "no target $1" ;;
  esac
}

# deal N LANG…: the languages dealt to N groups, the heaviest first, each to the lightest group
# so far; reply=(the groups, comma lists). N is at most the languages' count.
deal() {
  local n=$1 l i min
  shift
  local -a load groups
  for (( i = 1; i <= n; i++ )); do load[i]=0 groups[i]=''; done
  for l in ${${(f)"$(for l in $@; do print -r -- "$weight[$l] $l"; done | sort -k1,1nr -k2,2)"}#* }; do
    min=1
    for (( i = 2; i <= n; i++ )); do (( load[i] < load[min] )) && min=$i; done
    (( load[min] += weight[$l] ))
    groups[min]+=${groups[min]:+,}$l
  done
  reply=($groups)
}

plan() {
  local -a workers targets seen
  local w t n i
  for w in ${=1}; do
    t=${w%%=*} n=${w#*=}
    flags $t
    [[ $n == <1-> ]] || die "workers for $t: $n"
    (( ${seen[(Ie)$t]} )) && die "$t named twice"
    seen+=($t) targets+=("\"$t\"")
    if [[ $t == tokens ]]; then
      (( n <= ${#weight} )) || die "tokens=$n: more workers than its ${#weight} languages"
      deal $n ${(k)weight}
      for i in {1..$n}; do workers+=("{\"target\":\"$t\",\"shard\":$i,\"langs\":\"$reply[i]\"}"); done
    else
      for i in {1..$n}; do workers+=("{\"target\":\"$t\",\"shard\":$i,\"langs\":\"\"}"); done
    fi
  done
  (( ${#workers} <= 256 )) || die "${#workers} workers, a matrix takes 256"
  print "workers=[${(j:,:)workers}]"
  print "targets=[${(j:,:)targets}]"
}

fetch() {
  local t=$1 repo=${GITHUB_REPOSITORY:-junkdog/ada-md} id
  mkdir -p corpus/$t out
  id=$(gh api "repos/$repo/actions/artifacts?name=corpus-$t&per_page=100" \
    --jq '[.artifacts[] | select(.expired | not)] | sort_by(.created_at) | last | .id // empty')
  if [[ -n $id ]]; then
    gh api repos/$repo/actions/artifacts/$id/zip > out/corpus-$t.zip
    unzip -o -q out/corpus-$t.zip -d out
    print "corpus: artifact $id"
  elif gh release download fuzz-corpus -R $repo -p $t.tar.zst -D out --clobber 2> /dev/null; then
    print "corpus: the fuzz-corpus release"
  else
    print "corpus: none, starting empty"
    return
  fi
  tar -C corpus -I zstd -xf out/$t.tar.zst
  rm -f out/$t.tar.zst out/corpus-$t.zip
  if [[ $t == tokens ]]; then
    local -a langs=(corpus/tokens-*(N/))
    (( ${#langs} )) || die "the store's tokens holds no tokens-LANG dirs, the layout before F-017:" \
      "refresh it from the local corpora (runbook §2)"
    print "corpus: ${#langs} languages"
  fi
}

run() {
  local t=$1 secs=$2 bin rc=0
  [[ $secs == <1-> ]] || die "seconds: $secs"
  flags $t
  bin=target/x86_64-unknown-linux-gnu/release/$t
  if [[ -f target/ci/fuzzers.tar.zst ]]; then
    tar -C target/ci -I zstd -xf target/ci/fuzzers.tar.zst $t
    bin=target/ci/$t
  fi
  [[ -x $bin ]] || die "no binary $bin"
  # the runner's size, still unconfirmed by a run (CHK-03)
  print "$(nproc) CPUs ($(lscpu | sed -n 's/^Thread(s) per core: *//p') a core)," \
    "$(free -g | awk '/^Mem/ {print $2}') GB, $(lscpu | sed -n 's/^Model name: *//p')"
  if [[ $t == tokens ]]; then
    [[ -n ${3:-} ]] || die "tokens: no languages for the worker"
    run_langs $secs $bin $3
    return
  fi
  mkdir -p corpus/$t out/new out/findings
  local -a before=(corpus/$t/*(N:t)) after
  $bin $args -fork=$(nproc) -timeout=30 -rss_limit_mb=3072 -ignore_crashes=1 -ignore_timeouts=1 \
    -ignore_ooms=1 -max_total_time=$secs -artifact_prefix=out/findings/ corpus/$t $dirs \
    > out/$t.log 2>&1 || rc=$?
  after=(corpus/$t/*(N:t))
  local -a added=(${after:|before})
  print -rl -- ${added/#/corpus/$t/} | xargs -r cp -t out/new --
  local -a found=(out/findings/^slow-unit-*(N)) slow=(out/findings/slow-unit-*(N))
  local stats=$(grep -a '^#[0-9]*: cov:' out/$t.log | tail -n 1)
  print "rc=$rc, ${#before} inputs at the start, ${#added} added, ${#found} findings, ${#slow} slow units"
  print -r -- "last: $stats"
  summary "**$t** ($(nproc) CPUs, $secs s): rc $rc, ${#before} inputs at the start, ${#added} added," \
    "${#found} findings, ${#slow} slow units"
  summary '```'
  summary "$stats"
  summary '```'
  (( rc == 0 && ${#found} == 0 )) || { tail -n 40 out/$t.log; exit 1 }
}

# lane SECONDS BIN LANG…: one CPU's languages, one after another, each alone on its own corpus
# for its seed pass and its weight's share of what the lane's passes leave (when the passes
# alone fill the lane, it is split by their seconds). -max_total_time counts the seed pass
# (FuzzerFork.cpp starts its clock before the seed merge).
lane() {
  local secs=$1 bin=$2 l s w=0 p=0 rc
  shift 2
  for l in $@; do (( w += weight[$l], p += pass[$l] )); done
  for l in $@; do
    if (( secs > p )); then
      s=$(( pass[$l] + (secs - p) * weight[$l] / w ))
    else
      s=$(( secs * pass[$l] / p ))
    fi
    (( s >= 1 )) || s=1
    rc=0
    ADA_FUZZ_LANGS=$l $bin $args -fork=1 -timeout=30 -rss_limit_mb=3072 -ignore_crashes=1 \
      -ignore_timeouts=1 -ignore_ooms=1 -max_total_time=$s -artifact_prefix=out/findings/$l- \
      corpus/tokens-$l > out/tokens-$l.log 2>&1 || rc=$?
    print "$s $rc" > out/tokens-$l.rc
  done
}

# run_langs SECONDS BIN LANGS: `tokens` on the worker's languages, dealt by weight to a lane a CPU.
run_langs() {
  local secs=$1 bin=$2 l rc=0
  local -a langs=(${(s:,:)3}) lanes pids
  for l in $langs; do (( ${+weight[$l]} )) || die "no $l in LANGS"; done
  local c=$(nproc)
  (( c <= ${#langs} )) || c=${#langs}
  deal $c $langs
  lanes=($reply)
  mkdir -p out/new out/findings
  typeset -A before
  for l in $langs; do
    mkdir -p corpus/tokens-$l
    before[$l]="${(j: :)$(print -rl -- corpus/tokens-$l/*(N:t))}"
  done
  print "tokens on ${#langs} languages, $c lanes: ${(j: | :)lanes}"
  for l in $lanes; do lane $secs $bin ${(s:,:)l} & pids+=($!); done
  wait $pids
  summary "**tokens** ($(nproc) CPUs, $secs s, ${#langs} languages in $c lanes)"
  summary ''
  summary '| language | s | rc | inputs | added | findings | slow | cov | ft | exec/s |'
  summary '|---|---|---|---|---|---|---|---|---|---|'
  local -a was now added found slow line
  local stats cov ft es total=0 bad=0
  for l in $langs; do
    was=(${=before[$l]}) now=(corpus/tokens-$l/*(N:t))
    added=(${now:|was})
    if (( ${#added} )); then
      mkdir -p out/new/tokens-$l
      print -rl -- ${added/#/corpus/tokens-$l/} | xargs -r cp -t out/new/tokens-$l --
    fi
    found=(out/findings/$l-*~*/$l-slow-unit-*(N)) slow=(out/findings/$l-slow-unit-*(N))
    line=(- 1)
    [[ -f out/tokens-$l.rc ]] && line=(${=$(< out/tokens-$l.rc)})
    stats=$(grep -a '^#[0-9]*: cov:' out/tokens-$l.log 2> /dev/null | tail -n 1 || true)
    # `#N: cov: C ft: F corp: … exec/s: E …`
    cov=- ft=- es=-
    if [[ -n $stats ]]; then
      cov=${${stats#*cov: }%% *} ft=${${stats#*ft: }%% *} es=${${stats#*exec/s: }%% *}
    fi
    summary "| $l | $line[1] | $line[2] | ${#was} | ${#added} | ${#found} | ${#slow} | $cov | $ft | $es |"
    print "$l: ${line[1]} s, rc ${line[2]}, ${#was} inputs at the start, ${#added} added, ${#found} findings," \
      "${#slow} slow units; last: $stats"
    (( total += ${#added} ))
    if (( line[2] != 0 || ${#found} )); then
      (( bad += 1 ))
      tail -n 40 out/tokens-$l.log
    fi
  done
  # one log for the artifact, as the other targets leave
  for l in $langs; do print "=== $l"; cat out/tokens-$l.log; done > out/tokens.log
  print "tokens: $total inputs added, $bad languages with a finding or a failure"
  (( bad == 0 )) || exit 1
}

union() {
  local t=$1 shards=${2:-}
  [[ -z $shards || $shards == /* ]] || shards=$CALLER/$shards
  fetch $t
  local -a all
  local n m d
  if [[ $t == tokens ]]; then
    all=(corpus/tokens-*/*(N))
    n=${#all}
    if [[ -n $shards && -d $shards ]]; then
      for d in $shards/*/tokens-*(N/); do
        mkdir -p corpus/${d:t}
        find $d -type f -exec cp --update=none -t corpus/${d:t} -- {} +
      done
    fi
    all=(corpus/tokens-*/*(N))
    m=${#all}
    local -a ld=(corpus/tokens-*(N/:t))
    tar -C corpus -cf - $ld | zstd -19 -T0 -q > out/$t.tar.zst
  else
    all=(corpus/$t/*(N))
    n=${#all}
    if [[ -n $shards && -d $shards ]]; then
      find $shards -type f -exec cp --update=none -t corpus/$t -- {} +
    fi
    all=(corpus/$t/*(N))
    m=${#all}
    tar -C corpus -cf - $t | zstd -19 -T0 -q > out/$t.tar.zst
  fi
  print "$t: $n inputs, $m after the union"
  summary "**$t**: $n inputs, $m after the union ($(( $(stat -c %s out/$t.tar.zst) / 1024 )) KiB packed)"
}

pack() {
  local d
  mkdir -p target/ci-seed
  # a target's corpus, not lang.zsh's corpus/TARGET-LANG (its inputs again); for `tokens` the
  # other way around, its languages' corpora (corpus/tokens, their union, stays home)
  for d in corpus/^*-*(/:t); do
    if [[ $d == tokens ]]; then
      tar -C corpus -cf - corpus/tokens-*(/:t) | zstd -19 -T0 -q > target/ci-seed/$d.tar.zst
    else
      tar -C corpus -cf - $d | zstd -19 -T0 -q > target/ci-seed/$d.tar.zst
    fi
  done
  /usr/bin/ls -la target/ci-seed
}

langs() {
  local bin=${1:-target/x86_64-unknown-linux-gnu/release/tokens}
  [[ $bin == /* ]] || bin=$CALLER/$bin
  [[ -x $bin ]] || die "no binary $bin"
  print -n '\0x' > out-lang-probe
  local -a known=($(ADA_FUZZ_LANGS=list $bin out-lang-probe 2> /dev/null))
  rm -f out-lang-probe
  local -a have=(${(k)weight})
  local -a missing=(${known:|have}) extra=(${have:|known})
  (( ${#missing} == 0 && ${#extra} == 0 )) ||
    die "tokens' languages and LANGS differ: not in LANGS: ${missing:-none}; not in tokens: ${extra:-none}"
  print "tokens: ${#known} languages, as LANGS"
}

case ${1:-} in
  plan) plan "${2:?workers}" ;;
  fetch) fetch "${2:?target}" ;;
  run) run "${2:?target}" "${3:?seconds}" "${4:-}" ;;
  union) union "${2:?target}" "${3:-}" ;;
  pack) pack ;;
  langs) langs "${2:-}" ;;
  *) die "usage: ci.zsh plan|fetch|run|union|pack|langs …" ;;
esac
