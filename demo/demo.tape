# The demo at the top of the README, and the site's hero (site/media/hero.webp):
# `vhs demo/demo.tape`, from the repository's root, after `cargo build --release`.
# demo/setup.zsh sets up the notes it reads. The GIF is git-ignored: what the README and the
# site show is the WebP made from it:
#   ffmpeg -y -i demo/demo.gif -c:v libwebp_anim -quality 80 -loop 0 site/media/hero.webp

Output demo/demo.gif

Require zsh
Set Shell zsh
Set FontSize 14
Set FontFamily "JetBrains Mono, JetBrainsMono Nerd Font Mono"
Set Width 1000
Set Height 600
Set Padding 16
Set Theme "GruvboxDark"
Set TypingSpeed 70ms
Set Framerate 24
Set CursorBlink false

Hide
Type "base=$(demo/setup.zsh) && export XDG_CONFIG_HOME=$base/config PATH=$PWD/target/release:$PATH && cd $base/notes"
Enter
Type "PROMPT='%F{yellow}❯%f ' && clear"
Enter
Sleep 500ms
Show

Type "ada index.md"
Sleep 300ms
Enter
Sleep 1.8s

# o: the outline, filtered, jumps to a heading
Type "o"
Sleep 700ms
Type "sleep"
Sleep 500ms
Enter
Sleep 1.4s

# search sees through markup: **first nap** _after breakfast_
Type "/"
Sleep 200ms
Type "nap after"
Sleep 400ms
Enter
Sleep 1.4s

# l: the links; picking [[feeding]] focuses it, back up in the table, and Enter follows it
Type "l"
Sleep 700ms
Type "feed"
Sleep 500ms
Enter
Sleep 700ms
Enter
Sleep 1.6s

# , the settings: Enter on the theme lists the themes, and "mocha" previews catppuccin-mocha
# behind the popup, the first match at every letter typed. Enter picks it, a keeps it.
Type ","
Sleep 1.4s
Enter
Sleep 900ms
Type "mocha"
Sleep 1s
Enter
Sleep 1.2s
Type "a"
Sleep 1.4s

# two tables on screen: s lists the first's columns, → the second's; by how long things
# take, longest first: 6 days, 2 min, 1m 30s, 45 s, …, 350 µs
Type@150ms "jjj"
Sleep 400ms
Type "s"
Sleep 1s
Right
Sleep 1s
Type "takes"
Sleep 400ms
Down
Sleep 600ms
Enter
Sleep 2.4s

# back to where we were
Backspace
Sleep 1.2s

# Tab: back on [[feeding]], the next link is [[diagrams]], the cheese heist as a mermaid sequence
# diagram; scrolled through, its cast stays on top as a sticky header
Tab
Sleep 700ms
Enter
Sleep 1.4s
Down@100ms 22
Sleep 2.4s
