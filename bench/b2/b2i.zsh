#!/usr/bin/env zsh
# B2i: image previews, 100x50 cells at 10x20 px (ADR-29's size), printed to a pipe. Pinned to four physical cores
# of CCD0, as b2a.zsh. Two tables per image: kitty graphics, and text (half-blocks or symbols).
#   kitty: ada's unicode placeholders; kitten icat with placeholders, streamed (as ada) and through shared memory
#          (as fzf's fzf-preview.sh); chafa (direct placement: no placeholders); mcat
#   text:  ada's half-blocks; viu's (-b); chafa's default symbols, and chafa limited to half-blocks (vhalf)
# viu can't be told to use kitty without a terminal to ask; mcat has no half-blocks (only --ascii).
# icat's shared-memory runs leave their /dev/shm files behind with no terminal to read them; they are removed.
# A GIF is its first frame, as ada shows it: the others are told to stop there (STILL), or they play it to the pipe
# for good (chafa, viu). mcat plays a GIF as a video with no way to stop at a frame, so it has no GIF cells.
S=${B2_ROOT:-${0:A:h}/root}
B=$S/tools/bin; ADA=${ADA:-ada}
OUT=${B2I_OUT:-$S/b2i}   # B2I_OUT: another directory, for a rerun
export XDG_CONFIG_HOME=$S/home/x/config
# viu writes 24-bit color under COLORTERM=truecolor, as ada and `chafa -c full` do, and 256 colors without it
# (144 KB against 213 for fit.png): set here, so a run doesn't depend on the shell it starts from (before
# 2026-10-29 the runs had none)
export COLORTERM=truecolor
cd $S/corpus/img; mkdir -p $OUT; : > $OUT/bytes.txt
DROP=${B2_DROP_CACHES:-/usr/local/sbin/oxasm-drop-caches}   # as b2a.zsh
free_kib=$(awk '/^MemFree:/ { print $2 }' /proc/meminfo)
if [[ $DROP != off ]] && (( free_kib < ${B2_DROP_BELOW_GB:-8} * 1024 * 1024 )); then
  sudo -n $DROP >&2 || { echo "b2i: sudo -n $DROP failed; or set B2_DROP_CACHES=off" >&2; exit 1 }
fi
ICAT="/usr/bin/kitten icat --use-window-size 100,50,1000,1000 --unicode-placeholder --stdin=no --place=100x50@0x0"
KITTY=("ada:$ADA -p -w 100 --height 50 --image-protocol kitty"
       "icat:$ICAT --transfer-mode=stream"
       "icat-shm:$ICAT --transfer-mode=memory"
       "chafa:/usr/bin/chafa -f kitty -s 100x50 --view-size 100x50"
       "mcat:$B/mcat -i --kitty --silent --no-center --spx 1000x1000 --sc 100x50 --img-width 100c --img-height 50c")
TEXT=("ada:$ADA -p -w 100 --height 50 --image-protocol halfblocks"
      "viu:$B/viu -b -w 100 -h 50"
      "chafa:/usr/bin/chafa -f symbols -c full -s 100x50 --view-size 100x50"
      "chafa-vhalf:/usr/bin/chafa -f symbols -c full --symbols vhalf -s 100x50 --view-size 100x50")
typeset -A STILL=(icat "--loop 0" icat-shm "--loop 0" chafa "--animate off" chafa-vhalf "--animate off" viu -s)
# Peak RSS comes from a short hyperfine call per command (*.rss.NAME.json): a child's ru_maxrss counts what
# hyperfine held when it spawned it, and hyperfine grows with every run it keeps, so a small tool late in a
# long call reports hyperfine's size (ada-optimization, experiment 20)
shm() { find /dev/shm -maxdepth 1 -user $USER -name 'icat-*' -newer $OUT/.start -delete 2>/dev/null }
FILES="fit.png diagram.png photo-12mp.png photo.jpg photo-12mp.jpg photo-24mp.jpg"
FILES+=" demo.gif photo.gif photo-12mp.gif photo-12mp-i.gif"
for img in ${=B2I_FILES:-$FILES}; do
  for t in kitty text; do
    if [[ $t == kitty ]]; then P=($KITTY); else P=($TEXT); fi
    [[ -n $B2I_ONLY ]] && P=(${(M)P:#(${(~j:|:)${=B2I_ONLY}}):*})   # B2I_ONLY="ada icat", as B2P_ONLY
    [[ -n $ADA_REF ]] && P+=(${${(M)P:#ada:*}/#ada:$ADA/adaref:$ADA_REF})   # rerun.zsh: the reference ada, beside ada
    if [[ $img == *.gif ]]; then
      Q=(); for c in ${P:#mcat:*}; do n=${c%%:*}; Q+=("$c${STILL[$n]:+ $STILL[$n]}"); done; P=($Q)
    fi
    args=(); for c in $P; do args+=(-n ${c%%:*} "${c#*:} $img"); done
    : > $OUT/.start
    taskset -c 2,4,6,8 $B/hyperfine -N --input=null --output=pipe --warmup 2 --min-runs 20 \
      --export-json $OUT/$img.$t.json $args > $OUT/$img.$t.txt 2>&1
    for c in $P; do
      taskset -c 2,4,6,8 $B/hyperfine -N --input=null --output=pipe --warmup 1 --runs 10 \
        --export-json $OUT/$img.$t.rss.${c%%:*}.json -n ${c%%:*} "${c#*:} $img" > /dev/null 2>&1
    done
    for c in $P; do
      n=${c%%:*}; cmd=(${=c#*:} $img)
      echo "$img $t $n bytes=$($cmd < /dev/null 2>/dev/null | wc -c)"
    done >> $OUT/bytes.txt
    shm
  done
done
