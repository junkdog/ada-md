#!/usr/bin/env zsh
# Sets up what the demo tapes record: the notes in demo/notes, copied into a scratch git
# repository with a staged and some unstaged changes for the gutter, and a config of their own,
# so the recording doesn't depend on yours. Prints the scratch directory, which holds notes/
# and config/.
set -euo pipefail

repo=${0:A:h:h}
base=${TMPDIR:-/tmp}/ada-demo
rm -rf $base
mkdir -p $base/config/ada
cp -R $repo/demo/notes $base/notes
mkdir -p $base/notes/pics
cp $repo/ada.jpg $base/notes/pics/ada.jpg

# VHS's terminal (xterm.js) answers that it can do sixel, but draws nothing, so half-blocks.
cat > $base/config/ada/config.toml <<'EOF'
max_width = 96
image_protocol = "halfblocks"
EOF

cd $base/notes
git() { command git -c user.name=ada -c user.email=ada@example.com -c commit.gpgsign=false "$@" }
git init -q -b main
git add -A
git commit -qm "notes"

# staged: a task done today
perl -pi -e 's/^(- \[x\] brush the ruff)$/$1\n- [x] refill the water fountain/' index.md
git add index.md

# unstaged: an edited task and paragraph, a new task, a new shenanigan, and a shorter timeout in the firmware
perl -pi -e 's/patrolling the hallway and inspecting the/patrolling the stairs and inspecting all the/' index.md
perl -pi -e 's/^- \[x\] brush the ruff$/- [x] brush the ruff, twice/' index.md
perl -pi -e 's/^(- \[ \] find out who opened the cheese drawer.*)$/$1\n- [ ] order more salmon/' index.md
perl -pi -e 's/^(\| 2026-08-12 .*)$/$1\n| 2026-10-14 | walked across the keyboard | study | 6 s | €0 | | committed it |/' shenanigans.md
perl -pi -e 's/from_millis\(800\)/from_millis(650)/' feeder/src/main.rs

print -r -- $base
