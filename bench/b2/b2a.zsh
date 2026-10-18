#!/usr/bin/env zsh
# B2a: print to a pipe at 100 columns, stdin /dev/null. Pinned to four physical cores of CCD0.
# bat highlights the markdown source, as it does in an fzf preview, and lowdown doesn't highlight: both are floors,
# not rivals. rich (rich-cli) and mcat render it.
S=${B2_ROOT:?set B2_ROOT}
B=$S/tools/bin; ADA=${ADA:-ada}
# COLUMNS: rich lays out at -w but crops each line at its console's width, 80 to a pipe, which lost 18% of L's words
export XDG_CONFIG_HOME=$S/home/x/config BAT_CONFIG_PATH=$S/home/x/config/bat/none BAT_CACHE_PATH=$S/home/x/cache/bat COLUMNS=100
cd $S/corpus; mkdir -p $S/b2a; : > $S/b2a/bytes.txt
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
# Peak RSS comes from a short hyperfine call per command (*.rss.NAME.json): a child's ru_maxrss counts what
# hyperfine held when it spawned it, and hyperfine grows with every run it keeps, so a small tool late in a
# long call reports hyperfine's size (ada-optimization, experiment 20)
for d in s m l xl; do
  runs=15; [[ $d == xl ]] && runs=5
  args=(); for c in $P; do args+=(-n ${c%%:*} "${c#*:} $d.md"); done
  taskset -c 2,4,6,8 $B/hyperfine -N --input=null --output=pipe --warmup 2 --min-runs $runs --export-json $S/b2a/$d.json \
    $args > $S/b2a/$d.txt 2>&1
  for c in $P; do
    taskset -c 2,4,6,8 $B/hyperfine -N --input=null --output=pipe --warmup 1 --runs 10 \
      --export-json $S/b2a/$d.rss.${c%%:*}.json -n ${c%%:*} "${c#*:} $d.md" > /dev/null 2>&1
  done
  for c in $P; do
    n=${c%%:*}; cmd=(${=c#*:} $d.md)
    echo "$d $n bytes=$($cmd < /dev/null | wc -c)"   # RSS and CPU come from hyperfine's JSON
  done >> $S/b2a/bytes.txt
done
