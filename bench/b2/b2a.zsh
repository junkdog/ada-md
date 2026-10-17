#!/usr/bin/env zsh
# B2a: print to a pipe at 100 columns, stdin /dev/null. Pinned to four physical cores of CCD0.
S=${B2_ROOT:?set B2_ROOT}
B=$S/tools/bin; ADA=${ADA:-ada}
export XDG_CONFIG_HOME=$S/home/x/config
cd $S/corpus; mkdir -p $S/b2a; : > $S/b2a/bytes.txt
# drop the page cache and compact memory once if free memory is low, as `b2 run` does (see DROP_CACHES in
# src/main.rs): print mode is all startup, so not before every run; hyperfine's warmup refills the cache
DROP=${B2_DROP_CACHES:-/usr/local/sbin/oxasm-drop-caches}
free_kib=$(awk '/^MemFree:/ { print $2 }' /proc/meminfo)
if [[ $DROP != off ]] && (( free_kib < ${B2_DROP_BELOW_GB:-8} * 1024 * 1024 )); then
  sudo -n $DROP >&2 || { echo "b2a: sudo -n $DROP failed; or set B2_DROP_CACHES=off" >&2; exit 1 }
fi
for d in s m l xl; do
  runs=15; [[ $d == xl ]] && runs=5
  taskset -c 2,4,6,8 $B/hyperfine -N --input=null --output=pipe --warmup 2 --min-runs $runs --export-json $S/b2a/$d.json \
    -n ada "$ADA -p -w 100 $d.md" \
    -n mdcat "$B/mdcat --ansi --columns 100 $d.md" \
    -n glow "/usr/bin/glow -s dark -w 100 $d.md" \
    -n leaf "$B/leaf --inline ansi:100 $d.md" \
    -n lowdown "$B/lowdown -tterm --term-width=100 $d.md" > $S/b2a/$d.txt 2>&1
  for c in "ada:$ADA -p -w 100" "mdcat:$B/mdcat --ansi --columns 100" "glow:/usr/bin/glow -s dark -w 100" \
           "leaf:$B/leaf --inline ansi:100" "lowdown:$B/lowdown -tterm --term-width=100"; do
    n=${c%%:*}; cmd=(${=c#*:} $d.md)
    echo "$d $n bytes=$($cmd < /dev/null | wc -c)"   # RSS and CPU come from hyperfine's JSON
  done >> $S/b2a/bytes.txt
done
