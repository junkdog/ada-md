# B2's charts for the README, from interactive.dat and print.dat (bench/b2/plot.py writes them):
#   gnuplot bench/b2/plots/b2.gnuplot
# Gruvbox dark. Grouped bars on a log scale: a document per group, a viewer per bar, each bar's value above it.

dir = system("dirname " . ARG0)

# -- Border & Axis Styling --
set border 3 lc rgb '#a89984' lw 1.5
set grid ytics lc rgb '#3c3836' lw 1
set tics textcolor rgb '#ebdbb2' nomirror
set key textcolor rgb '#ebdbb2'
set title textcolor rgb '#fabd2f'
set xlabel textcolor rgb '#a89984'
set ylabel textcolor rgb '#a89984'
set datafile missing '?'
set style fill solid 0.9 border rgb '#282828'

# -- Accent Color Palette: ada bright, the others in gruvbox's darker shades --
col_ada = '#fabd2f'
array COL[6] = ['#fabd2f', '#98971a', '#b16286', '#458588', '#689d6a', '#cc241d']
array PCOL[5] = ['#fabd2f', '#98971a', '#d65d0e', '#cc241d', '#665c54']
array NAME[6] = ['ada', 'leaf', 'mdfried', 'md-tui', 'treemd HEAD', 'glow']
array PNAME[5] = ['ada', 'leaf', 'mdcat', 'glow', 'lowdown (no highlighting)']

group = 0.8   # share of a document's slot the bars take

unset key   # each chart draws one legend row of its own, above the panels (a panel's key wraps)
fmt(x) = x >= 100 ? sprintf('%.0f', x) : x >= 10 ? sprintf('%.1f', x) : x >= 1 ? sprintf('%.2f', x) : sprintf('%.3f', x)

# each block's smallest and largest value, into LO and HI; run before any range is set, since stats obeys them
array LO[6]; array HI[6]

# ---------------------------------------------------------------------------- interactive
set terminal pngcairo size 1800,1150 enhanced font 'sans,13' background '#282828'
set output dir . '/interactive.png'
array TITLE[6] = ['first paint (ms)', 'CPU per line scroll (ms)', 'search (ms)', 'resize, cold (ms)', \
                  'held page-down: CPU over 2 s (ms)', 'peak RSS (MiB)']
N = 6; w = group / N
do for [p = 0:5] {
    LO[p+1] = 1e300; HI[p+1] = -1e300
    do for [c = 2:N+1] {
        stats dir . '/interactive.dat' index p using c nooutput
        if (exists("STATS_records") && STATS_records > 0) { LO[p+1] = (STATS_min < LO[p+1] ? STATS_min : LO[p+1]); HI[p+1] = (STATS_max > HI[p+1] ? STATS_max : HI[p+1]) }
    }
}
do for [i = 1:N] {
    x = 0.5 + (i - (N + 1) / 2.0) * 0.12
    set object i rect from screen x - 0.05, 0.886 to screen x - 0.038, 0.905 fc rgb COL[i] fs solid noborder
    set label i NAME[i] at screen x - 0.032, 0.896 left tc rgb '#ebdbb2' font ',13'
}
set multiplot layout 2,3 title "{/:Bold ada against other terminal markdown viewers}\n{/*0.75 B2: 120×40 pty, four physical cores each; medians; lower is better}" \
    font ',18' tc rgb '#fabd2f' margins 0.05, 0.98, 0.08, 0.80 spacing 0.07, 0.15
set logscale y
set format y '10^{%L}'
set xrange [-0.5:3.5]
unset xtics
do for [p = 0:5] {
    lo = LO[p+1]; hi = HI[p+1]
    ymin = 10 ** floor(log10(lo) - 0.15); ymax = 10 ** ceil(log10(hi) + 0.8)   # room for the labels
    set yrange [ymin:ymax]
    set title '{/:Bold ' . TITLE[p+1] . '}' font ',14'
    plot for [i = 1:N] dir . '/interactive.dat' index p \
            using ($0 + (i - (N + 1) / 2.0) * w):(column(i + 1)):($0 + (i - (N + 1) / 2.0) * w - w / 2):($0 + (i - (N + 1) / 2.0) * w + w / 2):(ymin):(column(i + 1)) \
            with boxxyerror lc rgb COL[i] title NAME[i], \
         '' index p using 0:(ymin * 1.001):(stringcolumn(1)) with labels offset 0,-0.9 tc rgb '#ebdbb2' font ',13' notitle, \
         for [i = 1:N] '' index p using ($0 + (i - (N + 1) / 2.0) * w):(column(i + 1)):(fmt(column(i + 1))) \
            with labels rotate by 90 left offset 0,0.35 tc rgb (i == 1 ? col_ada : '#d5c4a1') font ',10' notitle
}
unset multiplot

# ---------------------------------------------------------------------------- print
set output dir . '/print.png'
set terminal pngcairo size 1800,760 enhanced font 'sans,13' background '#282828'
array PTITLE[2] = ['print to a pipe: wall time (ms)', 'print to a pipe: peak RSS (MiB)']
N = 5; w = group / N
unset logscale y
set autoscale
do for [p = 0:1] {
    LO[p+1] = 1e300; HI[p+1] = -1e300
    do for [c = 2:N+1] {
        stats dir . '/print.dat' index p using c nooutput
        if (exists("STATS_records") && STATS_records > 0) { LO[p+1] = (STATS_min < LO[p+1] ? STATS_min : LO[p+1]); HI[p+1] = (STATS_max > HI[p+1] ? STATS_max : HI[p+1]) }
    }
}
set logscale y
set xrange [-0.5:3.5]
unset object; unset label
do for [i = 1:N] {
    x = 0.5 + (i - (N + 1) / 2.0) * 0.13 - 0.04
    set object i rect from screen x - 0.05, 0.815 to screen x - 0.04, 0.837 fc rgb PCOL[i] fs solid noborder
    set label i PNAME[i] at screen x - 0.034, 0.826 left tc rgb '#ebdbb2' font ',13'
}
set multiplot layout 1,2 title "{/:Bold ada --print against other printers}\n{/*0.75 hyperfine, 100 columns, to a pipe; medians; lower is better}" \
    font ',18' tc rgb '#fabd2f' margins 0.05, 0.98, 0.13, 0.70 spacing 0.07, 0.1
do for [p = 0:1] {
    lo = LO[p+1]; hi = HI[p+1]
    ymin = 10 ** floor(log10(lo) - 0.15); ymax = 10 ** ceil(log10(hi) + 0.8)   # room for the labels
    set yrange [ymin:ymax]
    set title '{/:Bold ' . PTITLE[p+1] . '}' font ',14'
    plot for [i = 1:N] dir . '/print.dat' index p \
            using ($0 + (i - (N + 1) / 2.0) * w):(column(i + 1)):($0 + (i - (N + 1) / 2.0) * w - w / 2):($0 + (i - (N + 1) / 2.0) * w + w / 2):(ymin):(column(i + 1)) \
            with boxxyerror lc rgb PCOL[i] title PNAME[i], \
         '' index p using 0:(ymin * 1.001):(stringcolumn(1)) with labels offset 0,-0.9 tc rgb '#ebdbb2' font ',13' notitle, \
         for [i = 1:N] '' index p using ($0 + (i - (N + 1) / 2.0) * w):(column(i + 1)):(fmt(column(i + 1))) \
            with labels rotate by 90 left offset 0,0.35 tc rgb (i == 1 ? col_ada : '#d5c4a1') font ',10' notitle
}
unset multiplot
