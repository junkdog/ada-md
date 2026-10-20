#!/usr/bin/env zsh
# B2a: print to a pipe at 100 columns, stdin /dev/null. Pinned to four physical cores of CCD0.
# bat highlights the markdown source, as it does in an fzf preview, and lowdown doesn't highlight: both are floors,
# not rivals. rich (rich-cli) and mcat render it.
S=${B2_ROOT:-${0:A:h}/root}
B=$S/tools/bin; ADA=${ADA:-ada}
OUT=${B2A_OUT:-$S/b2a}   # B2A_OUT and B2A_ONLY="ada": a rerun of some tools, as b2p.zsh's; merge.py merges it
# COLUMNS: rich lays out at -w but crops each line at its console's width, 80 to a pipe, which lost 18% of L's words
export XDG_CONFIG_HOME=$S/home/x/config BAT_CONFIG_PATH=$S/home/x/config/bat/none BAT_CACHE_PATH=$S/home/x/cache/bat COLUMNS=100
cd $S/corpus; mkdir -p $OUT; : > $OUT/bytes.txt
# drop the page cache and compact memory once if free memory is low, as `b2 run` does (see DROP_CACHES in
# src/main.rs): print mode is all startup, so not before every run; hyperfine's warmup refills the cache
DROP=${B2_DROP_CACHES:-/usr/local/sbin/oxasm-drop-caches}
free_kib=$(awk '/^MemFree:/ { print $2 }' /proc/meminfo)
if [[ $DROP != off ]] && (( free_kib < ${B2_DROP_BELOW_GB:-8} * 1024 * 1024 )); then
  sudo -n $DROP >&2 || { echo "b2a: sudo -n $DROP failed; or set B2_DROP_CACHES=off" >&2; exit 1 }
fi
P=("ada:$ADA -p -w 100"
   "mdcat:$B/mdcat --ansi --columns 100"
   "glow:/usr/bin/glow -s dark -w 100"
   "leaf:$B/leaf --inline ansi:100"
   "lowdown:$B/lowdown -tterm --term-width=100"
   "bat:/usr/bin/bat --color=always --paging=never --style=numbers --terminal-width=100"
   "rich:$B/rich -m -w 100 --force-terminal"
   "mcat:$B/mcat -c -P --silent -f --sc 100xauto")
[[ -n $B2A_ONLY ]] && P=(${(M)P:#(${(~j:|:)${=B2A_ONLY}}):*})
[[ -n $ADA_REF ]] && P+=(${${(M)P:#ada:*}/#ada:$ADA/adaref:$ADA_REF})   # rerun.zsh: the reference ada, beside ada
# Peak RSS comes from a short hyperfine call per command (*.rss.NAME.json): a child's ru_maxrss counts what
# hyperfine held when it spawned it, and hyperfine grows with every run it keeps, so a small tool late in a
# long call reports hyperfine's size (ada-optimization, experiment 20)
for d in s m l xl; do
  runs=15; [[ $d == xl ]] && runs=5
  args=(); for c in $P; do args+=(-n ${c%%:*} "${c#*:} $d.md"); done
  taskset -c 2,4,6,8 $B/hyperfine -N --input=null --output=pipe --warmup 2 --min-runs $runs --export-json $OUT/$d.json \
    $args > $OUT/$d.txt 2>&1
  for c in $P; do
    taskset -c 2,4,6,8 $B/hyperfine -N --input=null --output=pipe --warmup 1 --runs 10 \
      --export-json $OUT/$d.rss.${c%%:*}.json -n ${c%%:*} "${c#*:} $d.md" > /dev/null 2>&1
  done
  for c in $P; do
    n=${c%%:*}; cmd=(${=c#*:} $d.md)
    echo "$d $n bytes=$($cmd < /dev/null | wc -c)"   # RSS and CPU come from hyperfine's JSON
  done >> $OUT/bytes.txt
done
