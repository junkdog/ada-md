#!/usr/bin/env zsh
# changelog.zsh [vX.Y.Z] [git-cliff args...]: git cliff by cliff.toml (REL-23), with each pre-release in its own
# section until its release is tagged, and folded into that release from then on. VERSION is the tag about to be
# made (git cliff --tag), and counts as tagged; the rest goes to git cliff as is.
#   ./changelog.zsh v1.0.0 -o CHANGELOG.md       # the release commit, before tagging v1.0.0 (also for an rc)
#   ./changelog.zsh -o CHANGELOG.md              # the tagged history, and what's unreleased
#   ./changelog.zsh --current --strip all        # the checked-out tag's notes (release.yml)
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
