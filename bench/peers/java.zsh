#!/usr/bin/env zsh
# Java beside logos, Lexilla (LexCPP) and KSyntaxHighlighting, and what its comments and strings
# cost: the four engines on the corpus as it is, then ada and logos and Lexilla with the comments'
# text removed, then with the strings' contents removed too (strip.rs); perf stat of ada and logos
# alone on the corpus as it is; callgrind of ada alone, by function. Run under
# `bench/work.zsh bench --cpus 2`; output in bench/peers/java/.
cd ${0:A:h}/../..   # the checkout this script is in
bin=bench/peers/target/release/ada-peers
out=bench/peers/java
mkdir -p $out
grep MemFree /proc/meminfo > $out/meminfo.txt
$bin --only java --rounds 11 --mb 40 > $out/full.txt 2>&1
$bin --only java --engines ada,logos,lexilla --rounds 11 --mb 40 --strip comments > $out/no-comments.txt 2>&1
$bin --only java --engines ada,logos,lexilla --rounds 11 --mb 40 --strip comments,strings > $out/no-comments-strings.txt 2>&1
for e in ada logos; do
  perf stat -x, -e instructions,cycles,branch-misses -o $out/stat-$e.csv \
    $bin --only java --engines $e --rounds 11 --mb 40 > $out/stat-run-$e.txt 2>&1
done
valgrind --tool=callgrind --callgrind-out-file=$out/ada.callgrind \
  $bin --only java --engines ada --rounds 1 --mb 1 > $out/cg-run.txt 2>&1
callgrind_annotate --inclusive=yes $out/ada.callgrind 2>/dev/null | head -80 > $out/ada-inclusive.txt
callgrind_annotate $out/ada.callgrind 2>/dev/null | head -80 > $out/ada-self.txt
echo "=== java done"
