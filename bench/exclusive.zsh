#!/usr/bin/env zsh
# exclusive.zsh LABEL CMD [ARGS…]: the old name for `bench/work.zsh bench LABEL CMD…`, kept while notes and
# scripts still call it (ada-optimization.md, "How we measure", "Two checkouts at once"). Use work.zsh: bench for
# timings, fuzz for fuzz rounds, build for builds.
#   As before, it checks for a quiet machine only when EXCLUSIVE_MAX_LOAD is set (any value: work.zsh's check is on
#   how busy the CPUs are, not on the load average); EXCLUSIVE_WAIT_MIN still bounds the wait.
emulate zsh
if [[ -n ${EXCLUSIVE_MAX_LOAD:-} ]]; then
  export WORK_MAX_BUSY=${WORK_MAX_BUSY:-5}
else
  export WORK_MAX_BUSY=${WORK_MAX_BUSY:-off}
fi
export WORK_WAIT_MIN=${WORK_WAIT_MIN:-${EXCLUSIVE_WAIT_MIN:-30}}
exec zsh ${0:A:h}/work.zsh bench "$@"
