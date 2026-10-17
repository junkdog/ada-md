# printcmp.zsh BASE NEW (bench/b2): every vault note and the corpus at four widths, compared by what shows
# needs ~/b2/corpus; temporary outputs go to $TMPDIR
base=$1 new=$2 S=${TMPDIR:-/tmp} n=0 bad=0 bb=0 nb=0
for f in ~/opt/birb-vault/**/*.md(N) ~/b2/corpus/*.md; do
  for w in 12 40 80 135; do
    n=$((n+1))
    $base -p -w $w $f >$S/a.out 2>&1 </dev/null
    $new -p -w $w $f >$S/b.out 2>&1 </dev/null
    bb=$((bb + $(stat -c %s $S/a.out))) nb=$((nb + $(stat -c %s $S/b.out)))
    python3 -I ${0:h}/sgrcmp.py $S/a.out $S/b.out >$S/why 2>&1 || { bad=$((bad+1)); echo "DIFF w=$w $f: $(<$S/why)" }
  done
done
echo "$n comparisons, $bad differ; bytes $bb -> $nb"
