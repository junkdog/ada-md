# Sorting by value: durations in µs, ms, s, minutes and days, ascending, descending, and back to
# the source order, so the clip ends as it starts.
Output "media/sort.gif"
Set Width 680
Set Height 420

Hide
Type "cd $CLEAN && clear && $ADA --line 16 feeding.md"
Enter
Sleep 1.5s
Show
Sleep 100ms
Screenshot "media/sort.png"

Sleep 1.2s
Type "s"
Sleep 700ms
Type "tak"
Sleep 700ms
Enter
Sleep 2.2s
# the list opens on the other direction, so Enter turns it around
Type "s"
Sleep 900ms
Enter
Sleep 2.2s
Type "s"
Sleep 700ms
Type "source"
Sleep 700ms
Enter
Sleep 1.2s
