# Known issues

## Images inside tmux are half-blocks

Inside tmux, ada draws images with half-block characters, even in terminals that can show real
images. `--image-protocol` (or `image_protocol` in the config) still forces a protocol, at your
own risk.

**Why.** Kitty graphics don't work inside tmux. Sixel and iTerm2 images reach the terminal through
tmux's passthrough, so they land where tmux last left its cursor rather than where ada drew them,
and they vanish whenever tmux redraws the pane (switching windows, copy mode, popups, resizing).
Scrolling and resizing flicker and leave pieces of images behind, sometimes in another pane.
tmux 3.4+ can handle sixel itself, but it drops images whenever their rows are erased and on every
resize ([tmux#4213](https://github.com/tmux/tmux/issues/4213)), so ada can't use that yet either.

**Upstream.** tmux 3.9 reworks image support, for both sixel and kitty graphics
([tmux#5445](https://github.com/tmux/tmux/pull/5445), on the
[3.9 roadmap](https://github.com/tmux/tmux/issues/5406)). ada will revisit this once it's released.

**Side effect.** Detecting the terminal's graphics support runs
`tmux set -p allow-passthrough on` for ada's pane, and the option stays on after ada exits.

**Workaround.** Open the note outside tmux, or press `Enter` on the image's link and `Enter` again
to open the image in your viewer.
