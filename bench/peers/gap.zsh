#!/usr/bin/env zsh
# Where ada's Rust pass goes beyond logos' last ladder rung: perf stat of each engine alone
# (11 rounds of 40 MB: 177 passes with the warm-up), then callgrind of ada alone (2 passes).
cd ${0:A:h}/../..   # the checkout this script is in
bin=bench/peers/target/release/ada-peers
out=bench/peers/gap
mkdir -p $out
for e in ada lookup; do
  perf stat -x, -e instructions,cycles,branch-misses -o $out/stat-$e.csv \
    $bin --only rust --engines $e --rounds 11 --mb 40 > $out/run-$e.txt 2>&1
done
valgrind --tool=callgrind --callgrind-out-file=$out/ada.callgrind \
  $bin --only rust --engines ada --rounds 1 --mb 1 > $out/cg-run.txt 2>&1
callgrind_annotate --inclusive=yes $out/ada.callgrind 2>/dev/null | head -60 > $out/ada-inclusive.txt
callgrind_annotate $out/ada.callgrind 2>/dev/null | head -60 > $out/ada-self.txt
echo "=== gap done"
