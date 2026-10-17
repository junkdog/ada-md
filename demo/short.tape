# The short demo at the top of the README, and the site's hero (site/media/hero.webp):
# `vhs demo/short.tape`, from the repository's root, after `cargo build --release`.
# demo/setup.zsh sets up the notes it reads. For the site:
#   ffmpeg -i demo/short.gif -c:v libwebp_anim -quality 80 -loop 0 site/media/hero.webp

Output demo/short.gif

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

# follow [[shenanigans]] from the table
Tab@400ms 3
Sleep 600ms
Enter
Sleep 1.4s

# sort the shenanigans by what they cost, most first
Type "s"
Sleep 600ms
Type "damage"
Sleep 300ms
Down
Sleep 500ms
Enter
Sleep 1.8s

# back to where we were
Backspace
Sleep 1.2s

# search sees through markup: **first nap** _after breakfast_
Type "/"
Sleep 200ms
Type "nap after"
Sleep 400ms
Enter
Sleep 1.4s

# the themes, previewed as they're selected; → lists the light ones
Type "T"
Sleep 700ms
Down
Sleep 900ms
Right
Sleep 1.2s
Enter
Sleep 1.8s
