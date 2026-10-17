# Following [[shenanigans#Unsolved]] and coming back to the same spot, the same link focused: the clip
# starts with it focused, so it ends as it starts.
Output "media/links.gif"
Set Width 680
Set Height 420

Hide
Type "cd $CLEAN && clear && $ADA index.md"
Enter
Sleep 1.5s
Tab
Sleep 500ms
Show
Sleep 100ms
Screenshot "media/links.png"

Sleep 1.2s
Enter
Sleep 2s
Sleep 800ms
Sleep 1s
Backspace
Sleep 1.5s
