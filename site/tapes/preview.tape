# ada as fzf's previewer: two notes with tables, code and a log, each printed at the preview
# pane's size. VHS draws images in half-blocks only, so the photo is left out (the card says it).
# --code-wrap off cuts code at the edge of the pane instead of wrapping it.
# --cycle takes the last Down back to the first file, so the clip ends as it starts.
Output "media/preview.gif"
Set Width 1000
Set Height 540

Hide
Type "cd $CLEAN && clear && print -l feeding.md shenanigans.md feeder/src/main.rs feeder/feeder.log | FZF_DEFAULT_OPTS= fzf --layout reverse --cycle --info hidden --preview '$ADA --print --code-wrap off {}' --preview-window right,80%"
Enter
Sleep 2s
Show
Sleep 100ms
Screenshot "media/preview.png"

Sleep 2s
Down
Sleep 2s
Down
Sleep 2s
Down
Sleep 3s
Down
Sleep 1s
