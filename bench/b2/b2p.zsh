#!/usr/bin/env zsh
# B2p: the files a previewer sees besides markdown (code, a log, huge files), printed to a pipe at 100 columns,
# whole and under a preview's budget. Pinned to four physical cores of CCD0, as b2a.zsh.
#   whole:   every tool prints the whole file
#   preview: ada with --height 50 (its fzf budget: 2,000 rows, PREVIEW_ROWS), bat with --line-range=:2000 and
#            rich with --head 2000 (both count source lines, so a wrapped line costs them more rows); mcat has no
#            budget and isn't run
# The huge files run under `timeout` (B2P_CAP seconds, 60 by default), fewer times: bat takes minutes to print
# 80 MB. A killed run exits non-zero, which hyperfine records (-i); doctables.py shows it as "> cap". Its CPU
# and RSS are lost: timeout signals its own process group too, so it never reaps the child.
# B2P_ONLY="ada rich" runs only those tools, for a rerun merged into the earlier records.
S=${B2_ROOT:?set B2_ROOT}
B=$S/tools/bin; ADA=${ADA:-ada}; CAP=${B2P_CAP:-60}
OUT=${B2P_OUT:-$S/b2p}   # B2P_OUT: another directory, for a rerun
# COLUMNS: rich lays out at -w but crops each line at its console's width, 80 to a pipe (b2a.zsh)
export XDG_CONFIG_HOME=$S/home/x/config BAT_CONFIG_PATH=$S/home/x/config/bat/none BAT_CACHE_PATH=$S/home/x/cache/bat COLUMNS=100
# Peak RSS comes from a short hyperfine call per command (*.rss.NAME.json): a child's ru_maxrss counts what
# hyperfine held when it spawned it, and hyperfine grows with every run it keeps, so a small tool late in a
# long call reports hyperfine's size (ada-optimization, experiment 20)
cd $S/corpus; mkdir -p $OUT; : > $OUT/bytes.txt
DROP=${B2_DROP_CACHES:-/usr/local/sbin/oxasm-drop-caches}   # as b2a.zsh
free_kib=$(awk '/^MemFree:/ { print $2 }' /proc/meminfo)
if [[ $DROP != off ]] && (( free_kib < ${B2_DROP_BELOW_GB:-8} * 1024 * 1024 )); then
  sudo -n $DROP >&2 || { echo "b2p: sudo -n $DROP failed; or set B2_DROP_CACHES=off" >&2; exit 1 }
fi
BAT="/usr/bin/bat --color=always --paging=never --style=numbers --terminal-width=100"
WHOLE=("ada:$ADA -p -w 100" "bat:$BAT" "rich:$B/rich --syntax -w 100 --force-terminal"
       "mcat:$B/mcat -c -P --silent --sc 100xauto")
PREVIEW=("ada:$ADA -p -w 100 --height 50" "bat:$BAT --line-range=:2000"
         "rich:$B/rich --syntax -w 100 --force-terminal --head 2000")
for f in ${=B2P_FILES:-code-s.rs code-m.rs code-l.rs log.log huge.json huge.log}; do
  for mode in whole preview; do
    if [[ $mode == whole ]]; then P=($WHOLE); else P=($PREVIEW); fi
    [[ -n $B2P_ONLY ]] && P=(${(M)P:#(${(~j:|:)${=B2P_ONLY}}):*})
    pre=(); runs=(--warmup 2 --min-runs 15)
    if [[ $f == huge.* ]]; then
      pre=(/usr/bin/timeout -s KILL $CAP)
      [[ $mode == whole ]] && runs=(--warmup 0 --runs 3) || runs=(--warmup 1 --min-runs 10)
    fi
    args=(); for c in $P; do args+=(-n ${c%%:*} "$pre ${c#*:} $f"); done
    taskset -c 2,4,6,8 $B/hyperfine -N -i --input=null --output=pipe $runs --export-json $OUT/$f.$mode.json \
      $args > $OUT/$f.$mode.txt 2>&1
    [[ $f == huge.* && $mode == whole ]] || for c in $P; do   # killed or far above hyperfine's own size
      taskset -c 2,4,6,8 $B/hyperfine -N -i --input=null --output=pipe --warmup 1 --runs 10 \
        --export-json $OUT/$f.$mode.rss.${c%%:*}.json -n ${c%%:*} "$pre ${c#*:} $f" > /dev/null 2>&1
    done
    for c in $P; do
      n=${c%%:*}; cmd=($pre ${=c#*:} $f)
      b=$( { taskset -c 2,4,6,8 $cmd < /dev/null 2>/dev/null; print $? > $OUT/.rc } | wc -c )
      echo "$f $mode $n bytes=$b rc=$(<$OUT/.rc)"
    done >> $OUT/bytes.txt
  done
done
