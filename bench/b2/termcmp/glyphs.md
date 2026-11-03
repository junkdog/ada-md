# Wide glyphs, emoji, an image and a table

A test document for `termcmp.zsh` (BEN-008): what experiment 5's vt100 tool drove and its scroll scripts
didn't. Run it from this directory, so the image paths resolve: `termcmp.zsh tmux glyphs.md BASE NEW`, with
`TERMCMP_ARGS="--image-protocol kitty"` (kitty), `sixel` (xterm) or `halfblocks` (any).

## Wide graphemes

漢字かなカナ한글 mixed with ASCII: 東京 is two cells a character, and so is ｆｕｌｌｗｉｄｔｈ.
A line of nothing but wide ones, long enough to wrap at 120 columns: 天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往秋收冬藏闰余成岁律吕调阳云腾致雨露结为霜金生丽水玉出昆冈剑号巨阙珠称夜光果珍李柰菜重芥姜海咸河淡鳞潜羽翔
Combining marks: e̊ ñ̃ q̣̇ Z̴̡̛͓̈́a̵̛͙̓l̷̢̐g̶̤̓o̵͈͗, and Hangul precomposed, then as jamo: 한, 한.

## Emoji

Single: 🦀 🐛 🎨 📚 🧪. With VS16: ❤️ ☀️ ✔️. Skin tones: 👍🏽 👋🏿. ZWJ sequences: 👩‍💻 👨‍👩‍👧‍👦 🏳️‍🌈 🧑🏻‍🚀.
Flags: 🇸🇪 🇯🇵 🇺🇦. Keycaps: 1️⃣ #️⃣. Text presentation: ☺ ✌ ⚠.

**Bold 🦀 crab**, *italic 漢字*, `code 🧪 東京`, and a [link with 👩‍💻](https://example.com).

## A table

| glyphs | cells | note |
|---|---|---|
| 東京 | 4 | two wide |
| 👨‍👩‍👧‍👦 | 2 | one ZWJ cluster |
| 🇸🇪🇯🇵 | 4 | two flags |
| é vs é | 6 | precomposed, then combining |
| ｆｕｌｌ | 8 | fullwidth Latin |

## An image

![a small image](../../../ada.jpg)

Text after the image, so a scroll past it moves both.

![a diagram](../root/corpus/img/diagram.png)

## More lines to scroll

1. 一 one
2. 二 two 🦀
3. 三 three 👍🏽
4. 四 four 🇸🇪
5. 五 five
6. 六 six
7. 七 seven
8. 八 eight
9. 九 nine
10. 十 ten

> A quote with 漢字 and 🧑🏻‍🚀 inside it, long enough that it wraps at the narrower widths and the wrap
> lands next to a wide glyph: 天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往秋收冬藏闰余成岁律吕调阳云腾致雨露结为霜

The end.
