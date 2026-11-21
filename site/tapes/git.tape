# ada as git's pager, moved through with the outline: `git log -p` over the feeder firmware's four
# commits (log/), `o` lists the commits and their files, each with how it changed; Down to the
# second commit's Rust, then a filter to the TOML schedule's last change. `g` goes back to the
# top, so the clip ends as it starts. Your own git config is left out (its pager, its colors), and
# GIT_PAGER is ada, as core.pager would be.
Output "media/git.gif"
Set Width 680
Set Height 420

Hide
Type "cd $LOG && clear && GIT_CONFIG_GLOBAL=/dev/null GIT_PAGER=$ADA git log -p"
Enter
Sleep 1.5s
Show
Sleep 100ms
Screenshot "media/git.png"

Sleep 1.2s
Type "o"
Sleep 1.2s
Down@400ms 4
Sleep 600ms
Enter
Sleep 3s
Type "o"
Sleep 800ms
Type@150ms "toml"
Sleep 800ms
Enter
Sleep 2.5s
Type "g"
Sleep 1s
