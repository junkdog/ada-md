# A resize, for real: ada runs in a tmux pane, and alt-h / alt-l (tapes/tmux.conf) move the
# border; build.zsh stops the tmux server afterwards. The table re-lays out as it narrows, turns
# into records, and comes back.
Output "media/resize.gif"
Set Width 1000
Set Height 420

Hide
Type "cd $CLEAN && clear && tmux -L ada-site kill-server 2>/dev/null; tmux -L ada-site -f $SITE/tapes/tmux.conf new-session '$ADA shenanigans.md' \; split-window -h -l 1 'tail -f /dev/null' \; select-pane -t 0"
Enter
Sleep 2s
Show
Sleep 100ms
Screenshot "media/resize.png"

Sleep 1s
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Alt+h
Sleep 350ms
Sleep 1.5s
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Alt+l
Sleep 350ms
Sleep 1s
