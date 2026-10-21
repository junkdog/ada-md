# Mermaid diagrams drawn as text: the cheese heist (a sequence diagram), scrolled slowly so its cast
# stays on top as a sticky header, then the flowchart with its subgraph and the state diagram with its
# composite state, each held whole on screen. g goes back to the top, so the clip ends as it starts.
Output "media/diagrams.gif"
Set Width 1000
Set Height 680

Hide
Type "cd $CLEAN && clear && $ADA diagrams.md"
Enter
Sleep 1.5s
Show
Sleep 100ms
Screenshot "media/diagrams.png"

Sleep 1.5s
Down@90ms 22
Sleep 1.5s
Down@40ms 31
Sleep 2.5s
Down@40ms 32
Sleep 2.5s
Type "g"
Sleep 1s
