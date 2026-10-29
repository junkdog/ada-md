#!/usr/bin/env zsh
# ci.zsh COMMAND …: fuzz rounds on GitHub's runners (F-014, .github/workflows/fuzz.yml), each
# target's flags as the local rounds run them, and the corpus a round starts from and leaves.
#
#   ci.zsh plan 'session=5 print=3 …'  the workers, one per shard, and the targets, as the
#                                      workflow's JSON outputs
#   ci.zsh fetch TARGET                corpus/TARGET from the newest `corpus-TARGET` artifact, else
#                                      the `fuzz-corpus` release's TARGET.tar.zst, else empty
#   ci.zsh run TARGET SECONDS          fuzz on every CPU; the inputs it adds to out/new, crashes,
#                                      timeouts, OOMs and slow units to out/findings, the log to
#                                      out/TARGET.log. Fails on a finding other than a slow unit
#   ci.zsh union TARGET [DIR]          corpus/TARGET (fetched) with the inputs anywhere under DIR,
#                                      as out/TARGET.tar.zst
#   ci.zsh pack                        the local corpora as target/ci-seed/TARGET.tar.zst, for the
#                                      `fuzz-corpus` release a first round starts from
#
# The corpus's store between rounds is the `corpus-TARGET` artifact (kept 90 days), so nothing
# is written to the repo or its releases. Inputs are named by their hash, so a union is a copy.
setopt err_exit pipe_fail no_unset extended_glob
F=${0:A:h} CALLER=$PWD
cd $F

die() { print -u2 "ci.zsh: $*"; exit 1 }
summary() { print -r -- "$*" >> ${GITHUB_STEP_SUMMARY:-/dev/null} }

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
    make) args=(-dict=make.dict -max_len=4096) ;;
    bash) args=(-dict=bash.dict -max_len=4096) ;;
    probe) args=(-max_len=4096) ;;
    sixel) args=(-max_len=8192) ;;
    roots) args=(-max_len=1024) ;;
    *) die "no target $1" ;;
  esac
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
    for i in {1..$n}; do workers+=("{\"target\":\"$t\",\"shard\":$i}"); done
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
  mkdir -p corpus/$t out/new out/findings
  local -a before=(corpus/$t/*(N:t)) after
  # the runner's size, still unconfirmed by a run (CHK-03)
  print "$(nproc) CPUs, $(free -g | awk '/^Mem/ {print $2}') GB, $(lscpu | sed -n 's/^Model name: *//p')"
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

union() {
  local t=$1 shards=${2:-}
  [[ -z $shards || $shards == /* ]] || shards=$CALLER/$shards
  fetch $t
  local -a all=(corpus/$t/*(N))
  local n=${#all}
  if [[ -n $shards && -d $shards ]]; then
    find $shards -type f -exec cp --update=none -t corpus/$t -- {} +
  fi
  all=(corpus/$t/*(N))
  local m=${#all}
  tar -C corpus -cf - $t | zstd -19 -T0 -q > out/$t.tar.zst
  print "$t: $n inputs, $m after the union"
  summary "**$t**: $n inputs, $m after the union ($(( $(stat -c %s out/$t.tar.zst) / 1024 )) KiB packed)"
}

pack() {
  local d
  mkdir -p target/ci-seed
  for d in corpus/*(/:t); do
    tar -C corpus -cf - $d | zstd -19 -T0 -q > target/ci-seed/$d.tar.zst
  done
  /usr/bin/ls -la target/ci-seed
}

case ${1:-} in
  plan) plan "${2:?workers}" ;;
  fetch) fetch "${2:?target}" ;;
  run) run "${2:?target}" "${3:?seconds}" ;;
  union) union "${2:?target}" "${3:-}" ;;
  pack) pack ;;
  *) die "usage: ci.zsh plan|fetch|run|union|pack …" ;;
esac
