#!/usr/bin/env zsh
# exclusive.zsh LABEL CMD [ARGS…]: runs CMD as the machine's only benchmark or fuzz round, so that the two
# checkouts (ada-md and ../ada-md-secondary) never measure or fuzz at once (ada-parse.md, "Parallel development").
#   The lock is a flock on $ADA_LOCK_DIR/.bench-lock (the main checkout by default, from either checkout), taken
#   before CMD starts and held until this script and everything CMD started have exited (they inherit it). The
#   kernel releases it however they end, so a killed run never leaves it taken.
#   While it is held, $ADA_LOCK_DIR/.bench-running names the holder (pid, start, checkout, branch, label, command).
#   It is removed when the run ends; one left by a SIGKILLed run is reported and replaced by the next run.
#   A second run prints the holder and waits in flock (no polling) until the first ends: start it with
#   run_in_background and wait for its notification. Exits with CMD's status.
# e.g. bench/exclusive.zsh "lexer_speed js" env ONLY=javascript taskset -c 2 cargo test -p ada-parse --release \
#        --test lexer_speed -- --ignored --nocapture
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
command  ${(j: :)${(q-)@}}" >| $marker
trap 'rm -f -- $marker' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

"$@"
rc=$?
print -r -- "exclusive.zsh: '$label' done, rc=$rc, $(date -Is)"
exit $rc
