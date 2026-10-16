# The longer tour, linked from the README: `vhs demo/tour.tape`, from the repository's root,
# after `cargo build --release`. demo/setup.zsh sets up the notes it reads. Each chapter starts
# on a clear screen with a comment saying what it shows.

Output demo/tour.gif

Require zsh
Set Shell zsh
Set FontSize 16
Set Width 1000
Set Height 600
Set Padding 16
Set Theme "GruvboxDark"
Set TypingSpeed 45ms
Set Framerate 24

Hide
Type "base=$(demo/setup.zsh) && export XDG_CONFIG_HOME=$base/config PATH=$PWD/target/release:$PATH && cd $base/notes"
Enter
Type "setopt interactive_comments && PROMPT='%F{yellow}❯%f ' && clear"
Enter
Sleep 500ms
Show

# --- notes, and what git says changed ---------------------------------------------------------

Type "# a note, rendered; the gutter marks what git says changed, staged and unstaged"
Enter
Sleep 300ms
Type "ada index.md"
Enter
Sleep 3s
Type@100ms "jjjjjjjjjj"
Sleep 1.5s
Type "g"
Sleep 800ms
# ]c / [c: the next and previous change
Type "]c"
Sleep 1.5s
Type "]c"
Sleep 1.5s
Type "g"
Sleep 800ms
Type "q"

Hide
Type "clear"
Enter
Show

# --- links and history ------------------------------------------------------------------------

Type "# links: Tab focuses one, Enter follows it, Backspace comes back to the same place"
Enter
Sleep 300ms
Type "ada index.md"
Enter
Sleep 1.5s
Tab@700ms 2
Sleep 1s
Enter
Sleep 2s
# feeding.md: highlighted code; `w` stops wrapping, ← → scroll it sideways
Space
Sleep 2s
Type "w"
Sleep 1.2s
Right@400ms 6
Sleep 1s
Type "w"
Sleep 1s
Backspace
Sleep 1.5s
# `l` lists the links; pick the firmware, which opens at its line
Type "l"
Sleep 1s
Type "firm"
Sleep 800ms
Enter
Sleep 800ms
Enter
Sleep 2.5s
Type "[c"
Sleep 2s
Backspace
Sleep 1.5s
Type "q"

Hide
Type "clear"
Enter
Show

# --- tables -----------------------------------------------------------------------------------

Type "# tables: sized like a browser's, sorted by a column with s, or shown as cards with t"
Enter
Sleep 300ms
Type "ada vet.md"
Enter
Sleep 2s
# the header stays while the table scrolls
Type@150ms "jjjjjj"
Sleep 1.5s
# by how long the wait was: 3h, 2h 30m, 1h 10m, 45 min, …
Type "s"
Sleep 1s
Type "wait"
Sleep 600ms
Down
Sleep 800ms
Enter
Sleep 2.5s
# auto → grid → records
Type "tt"
Sleep 2.5s
Space
Sleep 1.5s
Type "t"
Sleep 1.5s
Type "q"

Hide
Type "clear"
Enter
Show

# --- search and the outline -------------------------------------------------------------------

Type "# search sees through markup and into link targets; o is the outline of headings"
Enter
Sleep 300ms
Type "ada index.md"
Enter
Sleep 1.5s
Type "/"
Sleep 300ms
Type "nap after"
Sleep 800ms
Enter
Sleep 2s
Type "/"
Sleep 300ms
Type "main.rs"
Sleep 800ms
Enter
Sleep 2.5s
Type "o"
Sleep 1s
Type "sleep"
Sleep 800ms
Enter
Sleep 2s
Type "q"

Hide
Type "clear"
Enter
Show

# --- images -----------------------------------------------------------------------------------

Type "# images: inline, or full screen from a link (crisp in kitty, sixel and iTerm2)"
Enter
Sleep 300ms
Type "ada index.md"
Enter
Sleep 1.5s
Type "l"
Sleep 800ms
Type "rug"
Sleep 800ms
Enter
Sleep 800ms
Enter
Sleep 3s
Type "q"
Sleep 800ms
Type "q"

# a background write, which ada picks up while it's open
Hide
Type `(sleep 7 && perl -pi -e 's/^(\| 2026-10-14 .*)$/$1\n| 2026-10-16 | Lakeside | a demo | 1 min | €0 | 5.3 | very patient |/' vet.md) &!`
Enter
Type "clear"
Enter
Show

# --- live reload ------------------------------------------------------------------------------

Type "# live reload: ada reads the file again when it changes, keeping its place"
Enter
Sleep 300ms
Type "ada vet.md"
Enter
Sleep 6s
Type "q"

Hide
Type "clear"
Enter
Show

# --- themes and keys --------------------------------------------------------------------------

Type "# T picks a theme, previewing each one; ? lists every key"
Enter
Sleep 300ms
Type "ada index.md"
Enter
Sleep 1.5s
Type "T"
Sleep 1s
Down@1.5s 2
Sleep 1s
Enter
Sleep 1.5s
Type "?"
Sleep 3s
Type@300ms "jjjjjj"
Sleep 2s
Escape
Sleep 800ms
Type "q"

Hide
Type "clear"
Enter
Show

# --- print ------------------------------------------------------------------------------------

Type "# --print writes to a pipe, at any width: for fzf previews, say"
Enter
Sleep 300ms
Type "ada --print --width 64 feeding.md | head -26"
Enter
Sleep 4s
