# The header row stays on screen while a long table scrolls: down and back up, about 4 s.
Output "media/sticky.gif"
Set Width 680
Set Height 420

Hide
Type "cd $CLEAN && clear && $ADA --line 61 feeding.md"
Enter
Sleep 1.5s
Show
Sleep 100ms
Screenshot "media/sticky.png"

Sleep 600ms
Down@70ms 18
Sleep 600ms
Up@70ms 18
Sleep 400ms
