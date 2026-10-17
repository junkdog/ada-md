# Stills: the same note in each dark theme. No padding, since these themes paint their own
# background.
Output "media/.scratch/themes.gif"
Set Width 680
Set Height 420
Set Padding 0

Hide
Type "cd $CLEAN && clear"
Enter

Type "$ADA -t gruvbox-dark index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-gruvbox-dark.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t catppuccin-mocha index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-catppuccin-mocha.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t catppuccin-macchiato index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-catppuccin-macchiato.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t catppuccin-frappe index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-catppuccin-frappe.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t tokyonight-night index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-tokyonight-night.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t tokyonight-storm index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-tokyonight-storm.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t tokyonight-moon index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-tokyonight-moon.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t rose-pine index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-rose-pine.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t rose-pine-moon index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-rose-pine-moon.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms

Type "$ADA -t dracula index.md"
Enter
Sleep 1.2s
Show
Sleep 200ms
Screenshot "media/theme-dracula.png"
Sleep 300ms
Hide
Type "q"
Sleep 300ms
