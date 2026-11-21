# ada as man's pager, moved through with the outline: the feeder's own man page, `o` lists its
# sections and subsections; Down to OPTIONS, then a filter to BUGS. `g` goes back to the top, so
# the clip ends as it starts. MANPAGER is ada itself, not your own.
Output "media/man.gif"
Set Width 680
Set Height 420

Hide
Type "cd $CLEAN && clear && MANPAGER=$ADA man -l feeder/feeder.1"
Enter
Sleep 1.5s
Show
Sleep 100ms
Screenshot "media/man.png"

Sleep 1.2s
Type "o"
Sleep 1.2s
Down@400ms 3
Sleep 600ms
Enter
Sleep 3s
Type "o"
Sleep 800ms
Type@150ms "bug"
Sleep 800ms
Enter
Sleep 2.5s
Type "g"
Sleep 1s
