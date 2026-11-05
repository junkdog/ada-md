#!/usr/bin/env zsh
# work.zsh: the machine's work lock, which the agents working in the two checkouts at once (ada-md and
# ../ada-md-secondary) share, so that a benchmark never runs beside other work. The protocol around it:
# ada-optimization.md, "How we measure", "Two checkouts at once".
#   work.zsh bench|fuzz|build [--cpus LIST] LABEL CMD [ARGS…]   runs CMD as a job of that kind; exits with its status
#   work.zsh status [--json]                                    the jobs waiting and running
#   work.zsh log [N]                                            the last N events of the work log (20)
# What may run beside what: a bench beside nothing; a fuzz beside builds, but not beside another fuzz; a build
# beside builds and one fuzz. Compile under `build`, then measure under `bench`, so a compile never holds the machine.
#   The state is in $ADA_LOCK_DIR/.work (the main checkout by default, from either checkout; WORK_DIR moves it): a
#   lock per CCD (ccd0, ccd1: a bench takes them exclusively, the others shared), `fuzz` (one fuzz job at a time) and
#   `gate`, which every job passes on its way in and a waiting bench holds, so that builds started after it queue
#   behind it instead of starving it (Linux's flock doesn't favor a waiting exclusive lock). The locks are taken
#   before CMD starts and held until this script and everything CMD started have exited (they inherit them), and the
#   kernel releases them however they end. A job that has to wait prints what holds them and waits in flock, without
#   polling: start it with run_in_background and wait for its notification.
#   --cpus LIST (e.g. 0-15 or 2,4,6,8) pins CMD with taskset and names the CCDs the job locks; without it, all CPUs,
#   unpinned. WORK_SPLIT=1, on both sides, lets a bench whose CPUs lie in one CCD lock only that CCD, and a build that
#   finds a CCD held pin itself to the free ones: off until BEN-017 has measured a CCD0 bench beside a CCD1 build.
#   A bench, once it holds its locks, waits until the CPUs of the CCDs it holds average under WORK_MAX_BUSY percent
#   busy (5) over WORK_BUSY_WINDOW seconds of /proc/stat (5), for at most WORK_WAIT_MIN minutes (30), and exits 75
#   without running CMD if they never do; WORK_MAX_BUSY=off skips the check. It catches what the lock can't: an
#   unwrapped build, a script, a desktop app.
#   The work log: .work/jobs/ID.json names each waiting or running job, held under a lock of its own (ID.lock), so a
#   reader tells a killed job's file from a live one by trying that lock; .work/log.jsonl is the history (queue,
#   start, end, skip, lost, clear). A job whose work.zsh is gone while its lock is still held is "leftover":
#   processes CMD started still run, and `fuser -v .work/jobs/ID.lock` names them.
#   Until the second checkout runs this script, bench and fuzz jobs also take the old bench/exclusive.zsh's
#   .bench-lock (WORK_LEGACY=0 stops it).
# e.g. bench/work.zsh build "lexer js" cargo test -p ada-parse --release --test lexer_speed --no-run
#      bench/work.zsh bench --cpus 2 "lexer js" env ONLY=javascript cargo test -p ada-parse --release \
#        --test lexer_speed -- --ignored --nocapture
#      bench/work.zsh fuzz "py smoke" /home/junkdog/opt/dev/rust/ada-md/fuzz/lang.zsh smoke tokens python 300
emulate zsh
setopt no_unset extended_glob
zmodload zsh/datetime

usage() {
  print -u2 "usage: work.zsh bench|fuzz|build [--cpus LIST] LABEL CMD [ARGS…]
       work.zsh status [--json]
       work.zsh log [N]"
  exit 2
}
die() { print -u2 -r -- "work.zsh: $*"; exit 2 }

root=${ADA_LOCK_DIR:-$HOME/opt/dev/rust/ada-md}
W=${WORK_DIR:-$root/.work}
J=$W/jobs
mkdir -p $J || exit 1

# --- CPUs and CCDs ---------------------------------------------------------------------------------------------
cpus_expand() {   # "0-3,8" -> REPLY=(0 1 2 3 8)
  local part
  reply=()
  for part in ${(s:,:)1}; do
    if [[ $part == (#b)(<->)-(<->) ]]; then reply+=({$match[1]..$match[2]})
    elif [[ $part == <-> ]]; then reply+=($part)
    else return 1; fi
  done
}
cpus_compact() {   # (0 1 2 3 8) -> REPLY="0-3,8"
  local -a out; local c a=-1 b=-1
  for c in ${(no)@}; do
    if (( a >= 0 && c == b + 1 )); then b=$c; continue; fi
    (( a >= 0 )) && { (( a == b )) && out+=($a) || out+=($a-$b) }
    a=$c b=$c
  done
  (( a >= 0 )) && { (( a == b )) && out+=($a) || out+=($a-$b) }
  REPLY=${(j:,:)out}
}
cpus_expand $(</sys/devices/system/cpu/online); online=($reply)
typeset -A ccd_of
for c in $online; do
  f=/sys/devices/system/cpu/cpu$c/cache/index3/id
  [[ -r $f ]] && ccd_of[$c]=$(<$f) || ccd_of[$c]=0
done
all_ccds=(${(nou)ccd_of})
ccd_cpus() {   # REPLY=(the CPUs of the CCDs given)
  local c; local -a ds=($@); reply=()
  for c in $online; do (( ${ds[(Ie)$ccd_of[$c]]} )) && reply+=($c); done
}

# --- JSON --------------------------------------------------------------------------------------------------------
js() {   # REPLY = $1 as a JSON string
  local s=${1//[[:cntrl:]]/ }
  s=${s//\\/\\\\}; s=${s//\"/\\\"}
  REPLY="\"$s\""
}
dur() {   # seconds -> 1h02m, 12m30s, 45s
  local s=${1%.*}
  if (( s >= 3600 )); then printf '%dh%02dm' $(( s / 3600 )) $(( s % 3600 / 60 ))
  elif (( s >= 60 )); then printf '%dm%02ds' $(( s / 60 )) $(( s % 60 ))
  else printf '%ds' $s; fi
}
load() { cut -d' ' -f1-3 /proc/loadavg }

# --- the work log, read ------------------------------------------------------------------------------------------
logev_raw() { print -r -- "$1" >> $W/log.jsonl }
# a job file's field, from its one-line JSON (strings and numbers only; the writer below is the only one)
field() { local f=$1 k=$2; [[ $f =~ "\"$k\":(\"([^\"\\\\]|\\\\.)*\"|[-0-9.]+|true|false|null)" ]] && REPLY=${${match[1]#\"}%\"} || REPLY= }

# Lists the job files, clearing those whose job is gone: a file whose lock nobody holds is a killed job's (logged as
# lost) or a leftover's whose processes have since ended (logged as clear). Sets live=(file…) of the live ones.
scan() {
  local f id st ts
  live=()
  for f in $J/*.json(N); do
    id=${f:t:r}
    if flock -n -s $J/$id.lock true 2>/dev/null; then
      # gone; the mv makes sure one reader logs it
      if mv -f $f $f.gone 2>/dev/null; then
        field "$(<$f.gone)" state; st=$REPLY
        js "$id"; ts=$EPOCHSECONDS
        logev_raw "{\"t\":\"$(date -Is)\",\"ts\":$ts,\"ev\":\"${${st:#leftover}:+lost}${${(M)st:#leftover}:+clear}\",\"id\":$REPLY,\"job\":$(<$f.gone)}"
        rm -f $f.gone $J/$id.lock
      fi
      continue
    fi
    live+=($f)
  done
}

status_human() {   # one line per job, $1 the id to leave out (a waiting job's own)
  local f j state kind label pid since started queued checkout branch cpus now=$EPOCHSECONDS
  scan
  for f in $live; do
    [[ ${f:t:r} == ${1:-} ]] && continue
    j="$(<$f)"
    field "$j" state; state=$REPLY; field "$j" kind; kind=$REPLY; field "$j" label; label=$REPLY
    field "$j" pid; pid=$REPLY; field "$j" queued; queued=$REPLY; field "$j" started; started=$REPLY
    field "$j" checkout; checkout=${REPLY:t}; field "$j" branch; branch=$REPLY; field "$j" cpus; cpus=$REPLY
    [[ $state != leftover ]] && ! kill -0 $pid 2>/dev/null && state=leftover
    [[ $started == (|null) ]] && since=$queued || since=$started
    printf '  %-8s %-5s %-24s %8s  %s (%s), cpus %s, pid %s\n' $state $kind "\"$label\"" "$(dur $(( now - since )))" \
      $checkout ${branch:-detached} $cpus $pid
    [[ $state == leftover ]] && print -r -- "           its processes still hold the lock: fuser -v $J/${f:t:r}.lock"
  done
  (( $#live )) || print "  (no jobs)"
}

status_json() {
  local f j state pid first=1
  scan
  print -rn -- "{\"now\":$EPOCHSECONDS,\"dir\":\"$W\",\"jobs\":["
  for f in $live; do
    j="$(<$f)"
    field "$j" state; state=$REPLY; field "$j" pid; pid=$REPLY
    [[ $state != leftover ]] && ! kill -0 $pid 2>/dev/null && j=${j/\"state\":\"$state\"/\"state\":\"leftover\"}
    (( first )) || print -rn ,
    first=0
    print -rn -- "$j"
  done
  print -rn -- "],\"recent\":["
  if [[ -r $W/log.jsonl ]]; then
    print -rn -- "${(j:,:)${(f)$(grep -E '"ev":"(end|skip|lost|clear)"' $W/log.jsonl | tail -n ${WORK_RECENT:-8})}}"
  fi
  print -r -- "]}"
}

case ${1:-} in
  status)
    if [[ ${2:-} == --json ]]; then status_json; else
      print -r -- "work.zsh: jobs in $W, $(date -Is), load $(load)"
      status_human
    fi
    exit 0 ;;
  log)
    n=${2:-20}
    [[ -r $W/log.jsonl ]] || { print "(no work log yet)"; exit 0 }
    if (( $+commands[jq] )); then
      tail -n $n $W/log.jsonl | jq -r '[.t[0:19], .ev, (.kind // .job.kind // ""), ((.label // .job.label // "") | "\"\(.)\""),
        (if .ev == "end" then "rc=\(.rc) ran \(.run_s)s" + (if .leftover then ", leftover" else "" end)
         elif .ev == "start" then "waited \(.wait_s)s, cpus \(.cpus)" + (if .busy then ", busy \(.busy)" else "" end)
         elif .ev == "queue" then "\(.checkout | split("/") | last) (\(.branch)), cpus \(.cpus)"
         elif .ev == "skip" then "not run: busy \(.busy)" else "" end)] | @tsv' | column -t -s $'\t'
    else
      tail -n $n $W/log.jsonl
    fi
    exit 0 ;;
  bench|fuzz|build) kind=$1; shift ;;
  *) usage ;;
esac

# --- a job -------------------------------------------------------------------------------------------------------
cpus_arg=
case ${1:-} in
  --cpus) (( $# >= 2 )) || usage; cpus_arg=$2; shift 2 ;;
  --cpus=*) cpus_arg=${1#--cpus=}; shift ;;
esac
(( $# >= 2 )) || usage
label=$1
shift
if [[ -n $cpus_arg ]]; then
  cpus_expand $cpus_arg || die "--cpus $cpus_arg: not a CPU list"
  want=(${(nou)reply})
  for c in $want; do (( ${online[(Ie)$c]} )) || die "--cpus $cpus_arg: CPU $c isn't online ($(</sys/devices/system/cpu/online))"; done
else
  want=($online)
fi
typeset -U mine=()
for c in $want; do mine+=($ccd_of[$c]); done
split=${WORK_SPLIT:-0}
case $kind in
  bench) (( split )) && locks=($mine) || locks=($all_ccds); mode=-x ;;
  *)     locks=($mine); mode=-s ;;
esac
pinned=0; [[ -n $cpus_arg ]] && pinned=1
max_busy=${WORK_MAX_BUSY:-5}; [[ $kind == bench ]] || max_busy=off
window=${WORK_BUSY_WINDOW:-5}
wait_min=${WORK_WAIT_MIN:-30}

id=$(strftime %Y%m%d-%H%M%S $EPOCHSECONDS)-$$
queued=$EPOCHSECONDS
checkout=$PWD
branch=$(git branch --show-current 2>/dev/null)
js "$label"; jlabel=$REPLY
js "${(j: :)${(q-)@}}"; jcmd=$REPLY
js "$checkout"; jcheckout=$REPLY
js "$branch"; jbranch=$REPLY

# the job's own lock first, then its file: a file without a held lock is always a gone job's
exec {jfd}>$J/$id.lock || exit 1
flock -x $jfd || exit 1
state=waiting started=null
write_job() {
  local cpus; cpus_compact $want; cpus=$REPLY
  print -r -- "{\"id\":\"$id\",\"pid\":$$,\"kind\":\"$kind\",\"label\":$jlabel,\"state\":\"$state\",\"checkout\":$jcheckout,\"branch\":$jbranch,\"cpus\":\"$cpus\",\"pinned\":$(( pinned )),\"locks\":\"${(j:,:)${(@)locks/#/ccd}}\",\"queued\":$queued,\"started\":$started,\"command\":$jcmd}" >| $J/$id.json.tmp &&
    mv -f $J/$id.json.tmp $J/$id.json
}
logev() { logev_raw "{\"t\":\"$(date -Is)\",\"ts\":$EPOCHSECONDS,\"ev\":\"$1\",\"id\":\"$id\",\"kind\":\"$kind\",\"label\":$jlabel${2:-}}" }
write_job
cpus_compact $want
logev queue ",\"checkout\":$jcheckout,\"branch\":$jbranch,\"cpus\":\"$REPLY\",\"locks\":\"${(j:,:)${(@)locks/#/ccd}}\",\"load\":\"$(load)\",\"command\":$jcmd"

done_logged=0 rc=1
cleanup() {
  local still=0 run=0
  [[ $started != null ]] && run=$(( EPOCHSECONDS - started ))
  rm -f -- $J/$id.json.tmp
  [[ -n ${legacy_marker:-} ]] && rm -f -- $legacy_marker
  # closing our descriptor releases the job's lock unless processes CMD started still hold it
  exec {jfd}>&-
  if ! flock -n -x $J/$id.lock true 2>/dev/null; then
    still=1 state=leftover
    write_job
  else
    rm -f -- $J/$id.json $J/$id.lock
  fi
  if (( ! done_logged )); then
    logev end ",\"rc\":$rc,\"run_s\":$run,\"load\":\"$(load)\"${${still:#0}:+,\"leftover\":true}"
  fi
}
trap 'cleanup' EXIT
trap 'rc=129; exit 129' HUP
trap 'rc=130; exit 130' INT
trap 'rc=143; exit 143' TERM

announced=0
announce() {
  (( announced )) && return
  announced=1
  print -r -- "work.zsh: $kind '$label' waits, $(date -Is); the work in $W:"
  status_human $id
}
take() {   # take MODE FD WHAT: flock, announcing the wait if it has to
  flock -n $1 $2 && return 0
  announce
  flock $1 $2
}

# one fuzz job at a time, waited for before the gate, so it doesn't hold the gate while it waits
if [[ $kind == fuzz ]]; then
  exec {ffd}>>$W/fuzz || exit 1
  take -x $ffd || exit 1
fi
exec {gfd}>>$W/gate || exit 1
take -x $gfd || exit 1
typeset -A lfd
for d in $locks; do
  exec {fd}>>$W/ccd$d || exit 1
  lfd[$d]=$fd
done
if [[ $kind == build ]] && (( split )); then
  # a build takes the CCDs it finds free, and pins itself to them, if a bench holds the others
  got=()
  for d in $locks; do flock -n -s $lfd[$d] && got+=($d); done
  if (( $#got && $#got < $#locks )); then
    for d in ${locks:|got}; do fd=$lfd[$d]; exec {fd}>&-; unset "lfd[$d]"; done
    ccd_cpus $got
    want=(${want:*reply}) locks=($got) pinned=1
    print -r -- "work.zsh: build '$label' pinned to CPUs $(cpus_compact $want; print -rn $REPLY): a bench holds the other CCD"
  fi
fi
for d in $locks; do take $mode $lfd[$d] || exit 1; done
# leave the gate, and don't hand it to CMD
flock -u $gfd
exec {gfd}>&-
if [[ $kind != build && ${WORK_LEGACY:-1} != 0 ]]; then
  exec {ofd}>>$root/.bench-lock || exit 1
  if ! flock -n -x $ofd; then
    print -r -- "work.zsh: $kind '$label' waits for the old .bench-lock, held by:"
    sed 's/^/  /' $root/.bench-running 2>/dev/null || print "  (no marker)"
    flock -x $ofd || exit 1
  fi
  legacy_marker=$root/.bench-running
  print -r -- "pid      $$
start    $(date -Is)
checkout $PWD
branch   $branch
label    $label (work.zsh $kind job $id: bench/work.zsh status)" >| $legacy_marker
fi
(( announced )) && print -r -- "work.zsh: $kind '$label' has its locks, $(date -Is)"

# a bench waits for quiet CPUs
busy=
if [[ $max_busy != off ]]; then
  ccd_cpus $locks; quiet=($reply)
  sample() {   # REPLY = "mean max" busy percent of the CPUs in $quiet over $window seconds
    local -A t0 i0; local -a f; local c dt di sum=0 mx=0 b n=0
    while read -rA f; do
      [[ $f[1] == cpu<-> ]] || continue
      t0[${f[1]#cpu}]=$(( f[2] + f[3] + f[4] + f[5] + f[6] + f[7] + f[8] + f[9] )); i0[${f[1]#cpu}]=$(( f[5] + f[6] ))
    done < /proc/stat
    sleep $window
    while read -rA f; do
      [[ $f[1] == cpu<-> ]] || continue
      c=${f[1]#cpu}
      (( ${quiet[(Ie)$c]} )) || continue
      (( dt = f[2] + f[3] + f[4] + f[5] + f[6] + f[7] + f[8] + f[9] - t0[$c], di = f[5] + f[6] - i0[$c] ))
      (( b = dt > 0 ? 100.0 * (dt - di) / dt : 0, sum += b, n++ ))
      (( b > mx )) && mx=$b
    done < /proc/stat
    printf -v REPLY '%.1f %.1f' $(( n ? sum / n : 0 )) $mx
  }
  deadline=$(( EPOCHSECONDS + wait_min * 60 ))
  for (( i = 0; ; i++ )); do
    sample
    m=${REPLY% *}
    busy="${REPLY% *}% mean, ${REPLY#* }% max"
    (( m < max_busy )) && break
    if (( EPOCHSECONDS >= deadline )); then
      print -r -- "work.zsh: bench '$label' not run: CPUs $(cpus_compact $quiet; print -rn $REPLY) still $busy busy after $wait_min min, $(date -Is)"
      logev skip ",\"busy\":\"$busy\",\"load\":\"$(load)\""
      done_logged=1 rc=75
      exit 75
    fi
    (( i == 0 )) && print -r -- "work.zsh: bench '$label' waits for CPUs $(cpus_compact $quiet; print -rn $REPLY) under $max_busy% busy (now $busy)"
  done
fi

state=running started=$EPOCHSECONDS
write_job
cpus_compact $want; cpus=$REPLY
logev start ",\"wait_s\":$(( started - queued )),\"cpus\":\"$cpus\",\"pinned\":$(( pinned )),\"load\":\"$(load)\"${busy:+,\"busy\":\"$busy\"}"
print -r -- "work.zsh: $kind '$label' starts on CPUs $cpus, load $(load)${busy:+, busy $busy}"
pre=()
(( pinned )) && pre=(taskset -c $cpus)
"${pre[@]}" "$@"
rc=$?
print -r -- "work.zsh: $kind '$label' done, rc=$rc, $(dur $(( EPOCHSECONDS - started ))), load $(load), $(date -Is)"
exit $rc
