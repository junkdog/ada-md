# Still: a gantt chart, the cat's week in two sections, with done, active and critical tasks and a
# milestone, its date axis fitted to the width.
Output "media/.scratch/gantt.gif"
Set Width 680
Set Height 420

Hide
Type "cd $CLEAN && clear && $ADA week.md"
Enter
Sleep 1.5s
Show
Sleep 500ms
Screenshot "media/gantt.png"
Sleep 500ms
