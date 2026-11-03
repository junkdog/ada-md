# capped.zsh, sourced by b2a.zsh, b2p.zsh and b2i.zsh: the 2-minute cap on every command of a comparative
# benchmark (the user, 2026-11-02: bat and mdfried were seen going on for good on large inputs, where ada takes
# milliseconds). hyperfine has no timeout of its own, and wrapping each timed run in `timeout` would add its fork
# and exec to every cell, so each command runs once first under `timeout -s KILL $CAP`: one that doesn't finish
# is left out of the timed calls and noted in OUT/killed.txt ("input mode name"), which doctables.py shows as
# "> CAP s". The probe doubles as a warm-up. B2_CAP sets another cap in seconds (120); OUT/cap.txt records it.
CAP=${B2_CAP:-120}

# capped OUT KEY MODE ARRAY FILE: probe each "name:command" of the array named ARRAY on FILE, run from the current
# directory with stdin /dev/null, and drop the ones killed at the cap from ARRAY, noting "KEY MODE name" (KEY as the
# reports name the input: the document's id for print mode, the file for the others).
capped() {
  local out=$1 key=$2 mode=$3 arr=$4 file=$5 c rc
  local -a keep=()
  print $CAP > $out/cap.txt
  for c in ${(P)arr}; do
    /usr/bin/timeout -s KILL $CAP ${=c#*:} $file < /dev/null > /dev/null 2>&1
    rc=$?
    if (( rc == 137 )); then
      print "$key $mode ${c%%:*}" >> $out/killed.txt
      print -u2 "capped: ${c%%:*} on $file ($mode) didn't finish in $CAP s"
    else
      keep+=("$c")
    fi
  done
  set -A $arr "${keep[@]}"
}
