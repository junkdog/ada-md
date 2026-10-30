#!/usr/bin/env zsh
# Seeds for the session target that paint kitty images (F-11). Each candidate is a markdown
# document with three multi-cell images, a header naming a 120×40 terminal and kitty with a
# cell of 1×2 to 10×20 pixels, then random intents; the candidates whose frames hold kitty
# placeholders (`ADA_FUZZ_REACH`) are kept. The header's bytes are arbitrary 1.5's reading of
# the target's setup: ints little-endian, a bool the low bit, `int_in_range` a byte modulo the
# range. Run from fuzz/ after `cargo +nightly fuzz build -s none session`:
#
#   ./session-image-seeds.zsh [CANDIDATES] [OUT]    # 4000, corpus/session
emulate -L zsh -o err_exit -o pipe_fail
n=${1:-4000} out=${2:-corpus/session}
bin=target/x86_64-unknown-linux-gnu/release/session
tmp=$(mktemp -d -p target seeds.XXXXXX)
doc=$'# Kitty\n\n![i](img.png)\n\ntext before the wide one\n\n![w](wide.jpg)\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n![t](tall.png)\n\n- a [link](a.md)\n'
len=${#doc}  # ASCII, so its length in bytes
cells=('\x01\x02' '\x02\x04' '\x04\x08' '\x0a\x14')
for i in {1..$n}; do
    {
        printf "\\x$(printf %02x $((len & 255)))\\x$(printf %02x $((len >> 8)))"
        printf %s $doc
        # size (u16, u8), max_width None, tables auto, code wrap, line numbers off, no start
        # line, copy on yank, live reload auto, Some((kitty, cell w, cell h)), not stdin, fuzz.md
        printf "\x77\x00\x27\x00\x00\x01\x00\x00\x00\x00\x01\x00${cells[i % 4 + 1]}\x01\x00"
        head -c $((RANDOM % 160 + 16)) /dev/urandom
    } > $tmp/$i
done
mkdir -p $out
kept=0
ADA_FUZZ_REACH=1 $bin -rss_limit_mb=4096 $tmp/* 2>&1 |
    awk '/^Running: /{f=$2} /^reach .*placed=[1-9]/{print f}' |
    while read -r f; do
        cp $f $out/$(sha1sum < $f | cut -c1-40)
        kept=$((kept + 1))
    done
print "kept $kept of $n in $out"
rm -r $tmp
