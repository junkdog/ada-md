#!/usr/bin/env zsh
# The notes the site's clips read: demo/setup.zsh's scratch repository (notes/, with staged and
# unstaged changes, for the gutter clip only) and clean/, a clone of its last commit, which has no
# changes and so no gutter, for every other clip. Prints the scratch directory.
set -euo pipefail

base=$(${0:A:h:h:h}/demo/setup.zsh)
git clone -q $base/notes $base/clean
print -r -- $base
