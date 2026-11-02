#!/usr/bin/env zsh
# exclusive.zsh LABEL CMD [ARGS…]: runs CMD as the machine's only benchmark or fuzz round, so that agents working
# in two checkouts at once (ada-md and ../ada-md-secondary) never measure or fuzz at the same time. The protocol
# around it: ada-optimization.md, "How we measure", "Two checkouts at once".
#   The lock is a flock on $ADA_LOCK_DIR/.bench-lock (the main checkout by default, from either checkout), taken
#   before CMD starts and held until this script and everything CMD started have exited (they inherit it). The
#   kernel releases it however they end, so a killed run never leaves it taken.
#   While it is held, $ADA_LOCK_DIR/.bench-running names the holder (pid, start, checkout, branch, label, load,
#   command). It is removed when the run ends; one left by a SIGKILLed run is reported and replaced by the next run.
#   A second run prints the holder and waits in flock (no polling) until the first ends: start it with
#   run_in_background and wait for its notification. Exits with CMD's status.
#   Builds and tests don't take the lock, so it doesn't make the machine quiet. With EXCLUSIVE_MAX_LOAD set (a
#   number, e.g. 2), the run waits after taking the lock until the 1-minute load average is below it, checking
#   every 15 s for at most EXCLUSIVE_WAIT_MIN minutes (30), and exits 75 without running CMD if it never is. The
#   load is printed at the start and the end either way, so a result can be checked against it.
# e.g. bench/exclusive.zsh "lexer_speed js" env ONLY=javascript taskset -c 2 cargo test -p ada-parse --release \
#        --test lexer_speed -- --ignored --nocapture
#      EXCLUSIVE_MAX_LOAD=2 bench/exclusive.zsh "print A/B" bench/b2/root/build/x/bench.zsh
emulate zsh
setopt no_unset

if (( $# < 2 )); then
  print -u2 "usage: exclusive.zsh LABEL CMD [ARGS…]"
  exit 2
fi
label=$1
shift
dir=${ADA_LOCK_DIR:-$HOME/opt/dev/rust/ada-md}
lock=$dir/.bench-lock
marker=$dir/.bench-running
max_load=${EXCLUSIVE_MAX_LOAD:-}
wait_min=${EXCLUSIVE_WAIT_MIN:-30}
load() { cut -d' ' -f1-3 /proc/loadavg }

exec {fd}>>$lock || exit 1
if ! flock -n $fd; then
  print -r -- "exclusive.zsh: '$label' waits, $(date -Is); the lock is held by:"
  if [[ -r $marker ]]; then
    sed 's/^/  /' $marker
  else
    print "  (no marker: processes left by a stopped run still hold it)"
  fi
  flock $fd || exit 1
  print -r -- "exclusive.zsh: '$label' starts, $(date -Is)"
fi
if [[ -e $marker ]]; then
  print -r -- "exclusive.zsh: replacing a marker left by a killed run:"
  sed 's/^/  /' $marker
fi

print -r -- "pid      $$
start    $(date -Is)
checkout $PWD
branch   $(git branch --show-current 2>/dev/null)
label    $label
load     $(load)${max_load:+ (waits for < $max_load)}
command  ${(j: :)${(q-)@}}" >| $marker
trap 'rm -f -- $marker' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# the load right after a previous holder ends is still that run's, decaying: waiting under the lock lets it fall
if [[ -n $max_load ]]; then
  for (( i = 0; ; i++ )); do
    read l1 _ < /proc/loadavg
    (( l1 < max_load )) && break
    if (( i >= wait_min * 4 )); then
      print -r -- "exclusive.zsh: '$label' not run: load $(load) after $wait_min min, $(date -Is)"
      exit 75
    fi
    (( i == 0 )) && print -r -- "exclusive.zsh: '$label' waits for load < $max_load (now $(load))"
    sleep 15
  done
fi
print -r -- "exclusive.zsh: '$label' load at start: $(load)"

"$@"
rc=$?
print -r -- "exclusive.zsh: '$label' done, rc=$rc, load $(load), $(date -Is)"
exit $rc
