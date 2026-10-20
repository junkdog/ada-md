# Image scroll check

A document for recording one session and replaying it in every terminal (CS-3 of the clean-slate
plan): sixel in foot and xterm, iTerm2 in wezterm, kitty as the reference. Record and replay it
from this directory (`bench/images`), so the replay finds the images at the same relative paths.
They are B2's corpus images (`bench/b2/root`, built by B2) and the repo's `ada.jpg`. `scroll.rec`
is the session the user recorded in kitty at 130×45 (2026-10-20): line by line down to the end and
back up, twice; replay it in a window of that size:

```zsh
ada --record scroll.rec image-scroll.md
ada --replay scroll.rec --replay-live --replay-fast
```

`frame_dump --proto kitty,sixel,iterm2,halfblocks bench/images/image-scroll.md` dumps every frame
of its own script on this document for each protocol, to compare two builds byte for byte.

Scroll down line by line past every image, then page down and up, jump to the end and back to the
top, and quit. Things to look for while it plays: pixels left behind where an image was, an image
drawn a row off from its frame, text showing through a transparent image, the status bar drawn
over, and an image that never comes back after scrolling away.

## Text before the first image

The first image enters from the bottom of the window, a row at a time. Each row it shows is drawn
again from its first cell, so this is where a cut that ends inside a band of six pixels shows up.

Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore
et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut
aliquip ex ea commodo consequat.

Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla
pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit
anim id est laborum.

## A tall photo

A 1500 × 2000 JPEG: taller than the window, so it is cut at the top and the bottom at once while it
passes.

![a tall photo](../b2/root/corpus/img/photo.jpg)

Text right under the photo. When the photo has scrolled up and away, this line should be clean,
with nothing of the photo's last band left on it.

## A wide diagram

A 2473 × 1808 PNG with flat colors, fitted to the window's width.

![a wide diagram](../b2/root/corpus/img/diagram.png)

- an item under the diagram
- a second item
- a third item, a little longer than the others, so that it wraps in a narrow window and moves
  where the diagram ends

```rust
fn main() {
    // a code block between two images: its background should be untouched by either
    println!("hello, sixel");
}
```

## A transparent image

An RGBA PNG (686 × 811): its transparent pixels should show the document's background, never the
text that was on those cells before the image scrolled over them.

![an image with transparent pixels](../b2/root/corpus/img/fit.png)

| column | what it shows |
|---|---|
| left | a table right after the transparent image |
| right | its borders should be whole once the image has scrolled past |

## Two images back to back

The small one directly under the large one: each erases and draws its own cells only.

![a 12 megapixel photo](../b2/root/corpus/img/photo-12mp.png)

![a small image](../../ada.jpg)

## A missing image

![this file does not exist](../b2/root/corpus/img/missing.png)

The frame for an image that can't be read: text only, nothing drawn by a protocol.

## Text after the last image

Enough text to scroll every image off the top of the window.

Lorem ipsum dolor sit amet, consectetur adipiscing elit. Integer nec odio. Praesent libero. Sed
cursus ante dapibus diam. Sed nisi. Nulla quis sem at nibh elementum imperdiet. Duis sagittis ipsum.
Praesent mauris.

Fusce nec tellus sed augue semper porta. Mauris massa. Vestibulum lacinia arcu eget nulla. Class
aptent taciti sociosqu ad litora torquent per conubia nostra, per inceptos himenaeos.

Curabitur sodales ligula in libero. Sed dignissim lacinia nunc. Curabitur tortor. Pellentesque
nibh. Aenean quam. In scelerisque sem at dolor. Maecenas mattis. Sed convallis tristique sem.

Proin ut ligula vel nunc egestas porttitor. Morbi lectus risus, iaculis vel, suscipit quis, luctus
non, massa. Fusce ac turpis quis ligula lacinia aliquet. Mauris ipsum.

Nulla metus metus, ullamcorper vel, tincidunt sed, euismod in, nibh. Quisque volutpat condimentum
velit. Class aptent taciti sociosqu ad litora torquent per conubia nostra.

Nam nec ante. Sed lacinia, urna non tincidunt mattis, tortor neque adipiscing diam, a cursus ipsum
ante quis turpis. Nulla facilisi. Ut fringilla. Suspendisse potenti.

The end.
