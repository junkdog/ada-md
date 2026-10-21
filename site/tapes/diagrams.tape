# Mermaid diagrams drawn as text: the cheese heist (a sequence diagram), scrolled slowly so its cast
# stays on top as a sticky header, then the flowchart with its subgraph and the state diagram with its
# composite state, each paused on. g goes back to the top, so the clip ends as it starts.
Output "media/diagrams.gif"
Set Width 680
Set Height 420

Hide
Type "cd $CLEAN && clear && $ADA diagrams.md"
Enter
Sleep 1.5s
Show
Sleep 100ms
Screenshot "media/diagrams.png"

Sleep 1.5s
Down@90ms 36
Sleep 1.5s
Down@50ms 30
Sleep 2s
Down@50ms 40
Sleep 2s
Type "g"
Sleep 1s
