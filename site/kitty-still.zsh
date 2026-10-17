#!/usr/bin/env zsh
# Takes site/img/kitty-images.png: ada showing demo/notes/ada.md with its photo inline, in a real
# kitty window, since VHS's terminal can't draw images. Run it on a desktop with kitty and KDE's
# spectacle: it opens a 680×420 window (the clips' size, font and background), captures it, and
# closes it. KWin is asked to activate the window, but don't type meanwhile. The result is committed; CI
# can't take it.
set -euo pipefail

site=${0:A:h}
base=$($site/tapes/setup.zsh)
mkdir -p $base/kitty/ada
print 'max_width = 96' > $base/kitty/ada/config.toml
cat > $base/kitty/run.zsh <<RUN
#!/usr/bin/env zsh
cd $base/clean
XDG_CONFIG_HOME=$base/kitty exec ${site:h}/target/release/ada ada.md
RUN
chmod +x $base/kitty/run.zsh

kitty --config NONE -o remember_window_size=no -o initial_window_width=680 -o initial_window_height=420 \
  -o font_family='JetBrainsMono Nerd Font Mono' -o font_size=10.5 -o window_padding_width=8 \
  -o background='#272727' -o foreground='#ebdbb2' -o cursor_blink_interval=0 \
  --class ada-kitty-still $base/kitty/run.zsh &
kitty=$!
sleep 3

# KDE doesn't give a new window the focus while you type elsewhere, so ask KWin to activate it
cat > $base/kitty/activate.js <<'JS'
for (const w of workspace.windowList())
  if (w.resourceClass == "ada-kitty-still") workspace.activeWindow = w;
JS
id=$(qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript $base/kitty/activate.js ada-kitty-still)
qdbus6 org.kde.KWin /Scripting/Script$id org.kde.kwin.Script.run
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript ada-kitty-still >/dev/null
sleep 0.5

# spectacle takes the active window; if kitty still lost the focus, that is some other window or
# the whole screen, so anything but kitty's 680×420 is deleted at once and never kept
shot=$base/kitty/shot.png
spectacle -b -a -n -e -o $shot
sleep 1
kill $kitty
if [[ $(file -b $shot) == "PNG image data, 680 x 420,"* ]]; then
  mv $shot $site/img/kitty-images.png
else
  rm -f $shot
  print -u2 "kitty didn't have the focus, so the capture was something else and is deleted; run it again"
  exit 1
fi
