#!/usr/bin/env zsh
# termcmp.zsh TERM DOC BASE NEW [OUT] (bench/b2): run two ada builds live in a real terminal
# (tmux, kitty or alacritty) through the same key script on $B2_ROOT/corpus/DOC.md, capture the
# screen after every key, and compare the two builds screen by screen by what each cell shows.
# Also prints the scroll sequences (DECSTBM, IL, DL) each build sent.
#
# tmux: a detached server of its own at 120x40, `capture-pane -e -N` (-N keeps the cells an erase cleared on
#   a background, TRM-02), bytes by `pipe-pane`.
# kitty: remote control on an abstract socket, `get-text --ansi`, bytes by `--dump-bytes`.
# alacritty: on XWayland (no WAYLAND_DISPLAY), ada inside relay.py, which takes the keys from a
#   FIFO and logs the bytes; screens by ImageMagick's `import -window`, compared pixel for pixel.
# kitty and alacritty open a window for ~25 s per build. Output goes to OUT (default $TMPDIR).
term=$1 doc=$2 base=${3:A} new=${4:A} out=${5:-${TMPDIR:-/tmp}/termcmp}
here=${0:A:h} corpus=${B2_ROOT:-$here/root}/corpus
out=${out:A}

keys_m=(j j j k k d u / 'eight lines' ENTER j j j j j j j j j j j j k k k k k k d j u k n j j G k k k u g j j
        PGDN j k PGUP j j k k)
keys_l=(j j j j k k k d d u j PGDN j k PGUP / class ENTER j j k n k G k k g j k)

# drive BUILD DIR: one build through the key script, a screen per key into DIR
drive() {
  local bin=$1 dir=$2 name=termcmp-${1:t} pid= n=0 k i
  local -a keys=(${(P)${:-keys_$doc}})
  (( $#keys )) || keys=($keys_l)
  mkdir -p $dir; rm -f $dir/*(N)
  case $term in
    tmux)
      local -a T=(tmux -L termcmp -f /dev/null)
      $T kill-server 2>/dev/null
      # the delay lets pipe-pane attach before ada writes
      $T new-session -d -s t -x 120 -y 40 -c $corpus "sleep 1; exec $bin $doc.md"
      $T set -g status off
      $T pipe-pane -t t -o "cat > $dir/bytes"
      send() { case $1 in ENTER) $T send-keys -t t Enter;; PGDN) $T send-keys -t t NPage;;
               PGUP) $T send-keys -t t PPage;; *) $T send-keys -t t -l -- $1;; esac }
      grab() { $T capture-pane -t t -p -e -N > $dir/$1.txt }
      painted() { $T capture-pane -t t -p | grep -q '[^ ]' }
      stop() { $T kill-server }
      ;;
    kitty)
      local sock=@termcmp-$$
      (cd $corpus && exec kitty --config NONE -o allow_remote_control=socket-only --listen-on unix:$sock \
         -o initial_window_width=120c -o initial_window_height=40c -o remember_window_size=no \
         --dump-bytes $dir/bytes -T $name $bin $doc.md) &
      pid=$!
      local -a K=(timeout 5 kitten @ --to unix:$sock)
      for i in {1..50}; do $K ls >/dev/null 2>&1 && break; sleep 0.2; done
      send() { case $1 in ENTER) $K send-text '\r';; PGDN) $K send-text '\x1b[6~';;
               PGUP) $K send-text '\x1b[5~';; *) $K send-text -- $1;; esac }
      grab() { $K get-text --ansi --extent screen > $dir/$1.txt }
      painted() { $K get-text --extent screen 2>/dev/null | grep -q '[^ ]' }
      stop() { $K send-text q; for i in {1..25}; do kill -0 $pid 2>/dev/null || break; sleep 0.2; done
               kill $pid 2>/dev/null }
      ;;
    alacritty)
      local fifo=$dir/keys.fifo; mkfifo $fifo
      (cd $corpus && exec env -u WAYLAND_DISPLAY alacritty --config-file /dev/null \
         -o window.dimensions.columns=120 -o window.dimensions.lines=40 -o window.dynamic_title=false \
         -T $name -e python3 -I $here/relay.py $fifo $dir/bytes $bin $doc.md) &
      pid=$!
      exec 3<>$fifo
      send() { case $1 in ENTER) print -n '\r' >&3;; PGDN) print -n '\e[6~' >&3;;
               PGUP) print -n '\e[5~' >&3;; *) print -rn -- $1 >&3;; esac }
      grab() { timeout 10 import -window $name $dir/$1.png }
      painted() { [[ -s $dir/bytes ]] && (( $(stat -c %s $dir/bytes) > 4000 )) }
      stop() { print -n q >&3; for i in {1..25}; do kill -0 $pid 2>/dev/null || break; sleep 0.2; done
               kill $pid 2>/dev/null; exec 3>&-; rm -f $fifo }
      ;;
    *) print -u2 "unknown terminal $term"; return 1;;
  esac
  # keys during the image probe restart its timeout (O-06), so wait for the first paint
  for i in {1..50}; do painted && break; sleep 0.2; done
  sleep 0.5
  grab 00
  for k in $keys; do
    [[ -n $pid ]] && ! kill -0 $pid 2>/dev/null && { print "  ${bin:t}: the terminal exited after $n keys"; return 1 }
    n=$((n+1)); send $k; sleep 0.4; grab ${(l:2::0:)n}
  done
  # stop's last kill fails when the terminal already quit, which isn't the build's failure
  stop
  return 0
}

drive $base $out/$term-$doc-base || exit 1
drive $new $out/$term-$doc-new || exit 1
for b in base new; do
  print -n "  $b: $(stat -c %s $out/$term-$doc-$b/bytes) bytes; "
  grep -aoE $'\e\\[[0-9]*[LM]|\e\\[[0-9]+;[0-9]+r' $out/$term-$doc-$b/bytes | sort | uniq -c | sort -rn \
    | tr -s ' \n' ' ' | cat -v; echo
done
n=0 m=0
for f in $out/$term-$doc-base/<->.(txt|png); do
  m=$((m+1)); g=$out/$term-$doc-new/${f:t}
  if [[ $f == *.png ]]; then
    [[ $(compare -metric AE $f $g null: 2>&1) == 0* ]] || { n=$((n+1)); echo "  DIFF ${f:t}" }
  else
    python3 -I $here/sgrcmp.py --screen $f $g >$out/why 2>&1 \
      || { n=$((n+1)); echo "  DIFF ${f:t}: $(head -c 200 $out/why)" }
  fi
done
print "$term $doc: $n of $m screens differ"
