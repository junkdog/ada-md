# The short demo at the top of the README: `vhs demo/short.tape`, from the repository's root,
# after `cargo build --release`. demo/setup.zsh sets up the notes it reads.

Output demo/short.gif

Require zsh
Set Shell zsh
Set FontSize 16
Set Width 1000
Set Height 600
Set Padding 16
Set Theme "GruvboxDark"
Set TypingSpeed 70ms
Set Framerate 24

Hide
Type "base=$(demo/setup.zsh) && export XDG_CONFIG_HOME=$base/config PATH=$PWD/target/release:$PATH && cd $base/notes"
Enter
Type "PROMPT='%F{yellow}❯%f ' && clear"
Enter
Sleep 500ms
Show

Type "ada index.md"
Sleep 400ms
Enter
Sleep 2.5s

# follow [[shenanigans]] from the table
Tab@600ms 3
Sleep 1s
Enter
Sleep 2s

# sort the shenanigans by what they cost, most first
Type "s"
Sleep 1s
Type "damage"
Sleep 500ms
Down
Sleep 800ms
Enter
Sleep 2.5s

# back to where we were
Backspace
Sleep 1.5s

# search sees through markup: **first nap** _after breakfast_
Type "/"
Sleep 300ms
Type "nap after"
Sleep 600ms
Enter
Sleep 2s

# the themes, previewed as they're selected
Type "T"
Sleep 1s
Down@1.2s 2
Sleep 600ms
Enter
Sleep 2.5s

Type "q"
Sleep 1s
