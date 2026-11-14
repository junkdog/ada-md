#!/usr/bin/env zsh
# changelog.zsh [vX.Y.Z] [git-cliff args...]: git cliff by cliff.toml, the raw list of user-facing commits that
# CHANGELOG.md's next section is written from by hand (REL-024; CHANGELOG.md isn't generated from 1.1 on). Each
# pre-release has its own section until its release is tagged, and folds into that release from then on.
# VERSION is the release being written up (git cliff --tag), and counts as tagged, so its rcs fold into it; the
# rest goes to git cliff as is.
#   ./changelog.zsh v1.1.0 --unreleased --strip all   # everything since 1.0.0, the 1.1.0 rcs included
#   ./changelog.zsh --strip all                       # every release's list, newest first
cd ${0:A:h} || exit 1
args=()
released=(${(f)"$(git tag -l 'v[0-9]*' | grep -v -- -)"})
if [[ $1 == v[0-9]* ]]; then
  args=(--tag $1)
  [[ $1 == *-* ]] || released+=($1)
  shift
fi
# a released version's pre-releases fold into it, as git-cliff's own changelog does
(( $#released )) && args+=(--ignore-tags "^(${(j:|:)${(u)released//./\\.}})-rc\\.[0-9]+\$")
exec git cliff $args "$@"
