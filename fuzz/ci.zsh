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
#   ci.zsh flags TARGET                the target's libFuzzer flags a line each, `--`, then its
#                                      corpus dirs beyond its own (cover.zsh runs with them)
#   ci.zsh build [TARGET]              the target, else every target, as CI and the local rounds
#                                      build them (no sanitizer, debug assertions on); NOTC's
#                                      without trace-compares, into target/notc
#   ci.zsh binary TARGET               the path of the target's binary as `build` leaves it
#   ci.zsh fuzzers TARGET…             the targets' binaries as fuzzers.tar.zst, for CI's workers
#
# The corpus's store between rounds is the `corpus-TARGET` artifact (kept 90 days), so nothing
# is written to the repo or its releases. Inputs are named by their hash, so a union is a copy.
# `tokens`' store is its languages' corpora, corpus/tokens-LANG (F-017): every input runs
# through every one of its lexers otherwise (LANGS's 60), and one merge of them all takes hours.
setopt err_exit pipe_fail no_unset extended_glob
F=${0:A:h} CALLER=$PWD
cd $F

die() { print -u2 "ci.zsh: $*"; exit 1 }
summary() { print -r -- "$*" >> ${GITHUB_STEP_SUMMARY:-/dev/null} }

# `tokens`' languages (F-017): the edges a language's corpus reaches, its weight (its share of a
# worker's time, so a lexer of more code is fuzzed longer), and the seconds of one core its
# corpus's seed pass takes. The edges are its set-cover cmin's "new coverage edges" on the desk
# machine (`lang.zsh cmin tokens all`, out/lang/cmin-tokens-LANG.log; last 2026-11-14, before
# round 14, and the dated rows below); the seconds, every row's, a -runs=0 replay of its corpus
# by the binary CI runs, without trace-compares (2026-11-21, F-019; 3,187 s of one core with
# them, 814 without), rounded up. A new language goes here first: the build checks the list
# against the binary's (`langs`).
LANGS=(
  nix 16545 58               php 13767 65               elixir 12137 42            ruby 11224 39
  objcpp 10805 24            scala 10142 19             objc 9482 16               java 9431 25
  bash 7518 25               typescriptreact 6897 24    cpp 6804 21                cmakecppheader 6671 19
  typescript 6229 24         haskell 6121 11            rust 6022 16               scss 5920 15
  csharp 5127 17             cmake 5086 7               c 5079 15                  dockerfile 4828 21
  kotlin 4690 12             groovy 4489 16             cmakecheader 4453 14       css 4162 10
  python 3838 15             literatehaskell 3782 17    html 3740 20               crontab 3516 16
  sshdconfig 3426 14         sshconfig 3341 13          makefile 3257 21           go 2867 12
  lua 2557 5                 racket 2556 6              scheme 2492 6              javascript 2384 14
  emacslisp 2310 5           lisp 2272 4                powershell 2256 6          nginx 1940 5
  swift 1783 8               sql 1448 4                 terraform 1409 4           gitattributes 1329 2
  xml 1319 4                 clojure 1246 3             yaml 1067 2                toml 913 2
  log 877 2                  pkgconfig 824 1            json 726 1                 knownhosts 688 1
  gitignore 683 1            fstab 639 1                dotenv 612 1               gitlink 607 1
  cmakecache 568 1           gitmailmap 447 1           ini 435 1                  properties 342 1
  diff 339 1
  # X-178: from its first corpus's cmin (2026-11-19, the git pager's smokes)
  gitconfig 2945 5
  # the assembly languages (X-171, X-181, X-183): from their cmin after X-186's 300 s smokes
  # (2026-11-21; 68k's first, X-183's, had counted a third of its edges)
  x86asm 3645 5              armasm 3141 3              riscvasm 2681 3            mipsasm 2476 2
  z80asm 3357 3              mos6502asm 3720 4          m68kasm 3999 4
  # X-169: Dart, from its cmin after the 300 s smoke and 900 s follow-up (2026-11-21)
  dart 2255 7
  # X-188: from its first corpus's cmin (2026-11-21, after its 300 s smoke)
  manpage 5124 1
)
typeset -A weight pass
for l w p in $LANGS; do weight[$l]=$w pass[$l]=$p; done

# The targets built without trace-compares (F-019, F-021): libFuzzer's compare hooks are 51–72% of
# `tokens`' time (F-018) and ~60% of `session`'s; without them a CI lane runs 2.6–3.5× the inputs and
# adds 2.4× the edges in 30 minutes (C#, TOML), and six plain workers a side, a pair on each sixth of a
# markdown target's corpus, add 2.1× the edges (`session`), 1.35× (`print`) and 2.2× (`render`) in 30
# minutes, every worker that ran them ahead of its pair. What only the compare-guided runs reached was a
# fraction of what only the others did. They build into target/notc, so neither build's flags rebuild
# the other's dependencies.
NOTC=(tokens session print render)

binary() {
  if (( ${NOTC[(Ie)$1]} )); then
    print -r -- target/notc/x86_64-unknown-linux-gnu/release/$1
  else
    print -r -- target/x86_64-unknown-linux-gnu/release/$1
  fi
}

build() {
  local -a fz=(cargo +nightly fuzz build -s none -a --target x86_64-unknown-linux-gnu)
  if [[ -n ${1:-} ]]; then
    if (( ${NOTC[(Ie)$1]} )); then
      CARGO_TARGET_DIR=$F/target/notc $fz --no-trace-compares $1
    else
      $fz $1
    fi
    return
  fi
  # every target (NOTC's too, unused: `cargo fuzz build` builds them all), and beside it NOTC's, one
  # after another into their one target dir
  local -a pids
  local t p rc=0
  $fz & pids+=($!)
  { for t in $NOTC; do CARGO_TARGET_DIR=$F/target/notc $fz --no-trace-compares $t || exit 1; done } &
  pids+=($!)
  for p in $pids; do wait $p || rc=$?; done
  return $rc
}

fuzzers() {
  local t b
  local -a parts
  for t in $@; do
    b=$(binary $t)
    [[ -x $b ]] || die "no binary $b"
    parts+=(-C $F/${b:h} ${b:t})
  done
  tar -cf - $parts | zstd -3 -T0 -q > fuzzers.tar.zst
}

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
  bin=$(binary $t)
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
  local bin=${1:-$F/$(binary tokens)}
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
  flags) flags "${2:?target}"; print -rl -- $args -- $dirs ;;
  build) build "${2:-}" ;;
  binary) binary "${2:?target}" ;;
  fuzzers) shift; (( $# )) || die "fuzzers: no targets"; fuzzers $@ ;;
  *) die "usage: ci.zsh plan|fetch|run|union|pack|langs|flags|build|binary|fuzzers …" ;;
esac
