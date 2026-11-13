#!/usr/bin/env zsh
# Builds the sniffer's corpus manifest (one path a line, existing files only, deduped) from the
# census lists still on disk under bench/b2/root/build, plus samples of files on this machine
# for languages with no census list (Kotlin scripts, Perl, Emacs Lisp, mermaid, known_hosts, git links).
# Negatives (licenses, /usr/share/doc text, command output written to OUT_DIR/neg) are typed Plain.
# usage: corpus.zsh OUT_DIR
emulate -L zsh
setopt null_glob pipe_fail
out=${1:?OUT_DIR}
B=/home/junkdog/opt/dev/rust/ada-md/bench/b2/root/build
S=/home/junkdog/opt/dev/rust/ada-md-secondary/bench/b2/root/build
mkdir -p $out/lists

lists=(
  $B/token-cost/lists/*.txt
  $B/syntect-probe/lists/{markdown,sql,powershell}.txt
  $B/x134/lists-x134/{objc,objcpp}.txt
  $B/x136/lists/{haskell,lhs,sql}.txt
  $B/x137/lists/scala.txt
  $B/x151/lists/nix.txt
  $B/x146/lists/pkgconfig.txt
  $B/x104/lists/cmake.txt
  $B/x099/lists/census.txt
  $B/x120/lists/*.txt
  # the secondary checkout's census lists of record
  $S/x059/lists/crontab.txt
  $S/x060/lists/{ssh,sshd}.txt
  $S/x061/lists/nginx.txt
  $S/x100/lists/ruby.txt
  $S/x101/lists/swift.txt
  $S/x102/lists/{terraform,hcl}.txt
  $S/x135/lists/{groovy,jenkins}.txt
  $S/x138/lists/elixir.txt
  $S/x147/lists/nix.txt
)
for l in $lists; do
  cp $l $out/lists/${${l:h:h:h:h:h:h}:t}-${${l:h:h}:t}-${l:t}
done
# the crontab census's files are named `*crontab.txt`: their type goes in front (TYPE<TAB>PATH)
c=($out/lists/*-x059-crontab.txt)
sed 's/^/Crontab\t/' $c > $c.typed && mv $c.typed $c

# a sample: files matching the name patterns, 1 B..256 KiB, at most 5 a directory, the first 150
# in path order (deterministic)
sample() {
  local name=$1; shift
  find /home/junkdog /usr /etc -xdev \( "$@" \) -type f -size -257k -size +0 \
      -not -path '*/target/*' -not -path '*/.git/*' 2>/dev/null \
    | sort \
    | awk -F/ '{ d=$0; sub("/[^/]*$", "", d); if (++n[d] <= 5) print }' \
    | head -150 > $out/lists/disk-$name.txt
}
sample knownhosts -name 'known_hosts'
sample mermaid -name '*.mmd'
sample kts -name '*.kts'
sample perl -name '*.pl' -o -name '*.pm'
sample emacs -name '*.el'
sample gitlink -path '*/.git' -type f

# negatives, typed Plain: prose and command output, which no rule should claim
neg=$out/neg
mkdir -p $neg
find /usr/share/licenses -xdev -type f -not -name '*.md' -size +0 -size -257k 2>/dev/null | sort \
  | awk -F/ '{ d=$0; sub("/[^/]*$", "", d); if (++n[d] <= 1) print }' | head -150 \
  | sed 's/^/Plain\t/' > $out/lists/neg-licenses.txt
find /usr/share/doc -xdev -type f -name '*.txt' -size +0 -size -257k 2>/dev/null | sort \
  | awk -F/ '{ d=$0; sub("/[^/]*$", "", d); if (++n[d] <= 2) print }' | head -150 \
  | sed 's/^/Plain\t/' > $out/lists/neg-doc-txt.txt
{
  /usr/bin/ls -l /usr/bin | head -300 > $neg/ls-l.out
  /usr/bin/ls -1 /usr/share | head -300 > $neg/ls-1.out
  ps aux | head -200 > $neg/ps.out
  df -h > $neg/df.out
  lsblk > $neg/lsblk.out
  free -h > $neg/free.out
  pacman -Q 2>/dev/null | head -300 > $neg/pacman.out
  for m in ls grep tar ssh zsh; do MANWIDTH=80 man $m 2>/dev/null | col -b > $neg/man-$m.out; done
  print -l $neg/*.out(L+0) | sed 's/^/Plain\t/' > $out/lists/neg-output.txt
}

cat $out/lists/*.txt | sort -u | while IFS= read -r line; do
  [[ -f ${line#*$'\t'} ]] && print -r -- $line
done > $out/corpus.txt
wc -l < $out/corpus.txt
