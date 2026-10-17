# A search across bold, italic and plain text, matched as you type; Esc puts the previous (no)
# search back, so the clip ends as it starts.
Output "media/search.gif"
Set Width 680
Set Height 420

Hide
Type "cd $CLEAN && clear && $ADA --line 26 index.md"
Enter
Sleep 1.5s
Show
Sleep 100ms
Screenshot "media/search.png"

Sleep 1s
Type "/"
Sleep 500ms
Type@90ms "first nap after breakfast, in the sun"
Sleep 2.2s
Escape
Sleep 1.2s
