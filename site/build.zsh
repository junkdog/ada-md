#!/usr/bin/env zsh
# Records the site's clips and stills into site/media: each tape in site/tapes, after
# settings.tape, run from site/ so the paths in the tapes are relative to it. Clips become
# animated WebP (KEEP_GIF=1 keeps the GIFs, to look at frames). Needs vhs, tmux, ffmpeg, and
# ada built in release (cargo build --release).
#
#   site/build.zsh             # every tape
#   site/build.zsh sort links  # just these
set -euo pipefail

site=${0:A:h}
base=$($site/tapes/setup.zsh)
export SITE=$site NOTES=$base/notes CLEAN=$base/clean XDG_CONFIG_HOME=$base/config
export ADA=${site:h}/target/release/ada
cd $site
mkdir -p media/.scratch

if (( $# )); then
  tapes=($@)
else
  tapes=(tapes/*.tape(:t:r))
  tapes=(${tapes:#settings})
fi
for t in $tapes; do
  print -r -- "== $t"
  cat tapes/settings.tape tapes/$t.tape > media/.scratch/$t.tape
  vhs -q media/.scratch/$t.tape
  tmux -L ada-site kill-server 2>/dev/null || true  # resize.tape's
done
for g in media/*.gif(N); do
  ffmpeg -v error -y -i $g -c:v libwebp_anim -quality 80 -loop 0 ${g:r}.webp
  [[ -n ${KEEP_GIF:-} ]] || rm $g
done
rm -rf media/.scratch
