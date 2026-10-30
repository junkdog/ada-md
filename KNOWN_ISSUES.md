# Known issues

## Images inside tmux are half-blocks

Inside tmux, ada draws images with half-block characters, even in terminals that can show real
images. `--image-protocol` (or `image_protocol` in the config) still forces a protocol, at your
own risk.

**Why.** Kitty graphics don't work inside tmux. Sixel and iTerm2 images sent through tmux's
passthrough land where tmux last left its cursor rather than where ada drew them,
and they vanish whenever tmux redraws the pane (switching windows, copy mode, popups, resizing).
Scrolling and resizing flicker and leave pieces of images behind, sometimes in another pane.
tmux 3.4+ can handle sixel itself, but it drops images whenever their rows are erased and on every
resize ([tmux#4213](https://github.com/tmux/tmux/issues/4213)), so ada can't use that yet either.

**Upstream.** tmux 3.9 reworks image support, for both sixel and kitty graphics
([tmux#5445](https://github.com/tmux/tmux/pull/5445), on the
[3.9 roadmap](https://github.com/tmux/tmux/issues/5406)). ada will revisit this once it's released.

**Forcing a protocol inside tmux.** ada never uses tmux's passthrough for images and doesn't
change your tmux options. A forced sixel goes to tmux itself: a tmux 3.4+ built with sixel draws
it, within the limits above (it can disappear when its rows are erased or the pane resizes), and
other builds show nothing. Forced iTerm2 and kitty images also go to tmux unwrapped, and tmux
draws neither itself.

**Workaround.** Open the note outside tmux, or press `Enter` on the image's link and `Enter` again
to open the image in your viewer.
