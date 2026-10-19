# B2: ada against other terminal markdown viewers

The harness behind the B2 section of the vault note `feature-matrix-comparison` (Performance (B2)): pty-driven
latency, CPU and memory for interactive viewers, plus `hyperfine` for print mode.

## Setup

```zsh
export B2_ROOT=~/b2 ADA=$PWD/target/release/ada
mkdir -p $B2_ROOT/{corpus,tools,b2}
# viewers, into a prefix of their own
cargo install --locked --root $B2_ROOT/tools treemd@0.9.1 leaf-markdown-viewer@1.29.1 md-tui@0.11.0 mdcat@2.18.0 hyperfine
cargo install --locked --root $B2_ROOT/tools mcat@0.6.5 viu@1.6.1
UV_TOOL_DIR=$B2_ROOT/tools/uv UV_TOOL_BIN_DIR=$B2_ROOT/tools/bin uv tool install rich-cli==1.8.1
# bat, kitty (kitten), chafa and glow from the distribution
cargo install --locked --root $B2_ROOT/tools mdfried@0.22.8 --no-default-features --features svg,mermaid  # no libchafa
# treemd HEAD, if wanted, as tools/bin/treemd-head; glow from the distribution; lowdown built with cc -O2
# the corpus: s.md, m.md, l.md (vault notes) and xl.md (notes over 4 KB concatenated to 2 MB)
cargo build --release --manifest-path bench/b2/Cargo.toml   # the pty driver, bench/b2/target/release/b2
```

## Run

`B2_ROOT=~/b2 ADA=<release ada> zsh bench/b2/full.zsh ~/b2/results/<date>-<sha>-all` runs all of it (the lanes,
print mode, the previewer files, images and the tables) into a record set that `rerun.zsh` takes as REF, with
the bench cores held ready (below). The steps one by one:

```zsh
B2=bench/b2/target/release/b2
python3 -I bench/b2/lanes.py   # writes lane0..3.json
L=(0,2,4,6 8,10,12,14 16,18,20,22 24,26,28,30)   # one thread per physical core, a CCD per pair
for i in 0 1 2 3; do
  taskset -c ${L[$((i+1))]} $B2 run bench/b2/lane$i.json $B2_ROOT/b2/lane$i.jsonl &
done; wait
cat $B2_ROOT/b2/lane*.jsonl > $B2_ROOT/b2/all.jsonl
python3 -I bench/b2/report.py $B2_ROOT/b2/all.jsonl; python3 -I bench/b2/rank.py
zsh bench/b2/b2a.zsh && python3 -I bench/b2/b2a_report.py
zsh bench/b2/b2p.zsh   # the previewer files: code, a log, huge files; whole and under a preview's budget
zsh bench/b2/b2i.zsh   # image previews: kitty graphics, half-blocks and symbols
python3 -I bench/b2/doctables.py > tables.md   # the vault notes' tables, × columns and ranks included
python3 -I bench/b2/plot.py $(git rev-parse --short HEAD) && gnuplot bench/b2/plots/b2.gnuplot   # the README's charts
python3 -I site/perf.py   # the site's perf section, from the same .dat files
```

The four lanes take about 18 minutes, print mode another 3, the previewer files about 13 (the huge files' runs are
capped at 60 s each, `B2P_CAP`), and images 3: the lanes and the previewer files estimated from the 2026-10-18
run's times, without the cells that always hit their cap.

### Known timeouts

`timeouts.py` lists the cells that always hit their cap: md-tui 0.11.0 on XL (284 s to paint, against 90 s), bat
0.26.1 on `huge.log` and mcat 0.6.5 on both huge files, printed whole (over 60 s). They're skipped while the
installed tool's `--version` still names that version: `mkspec.py` leaves the job out and `b2p.zsh` the command,
which saves 11 minutes of lane time (about 5 of wall time) and 12 of the previewer files. The tables don't change:
`doctables.py` shows a skipped b2p cell as `> 60 s`, as when it was killed, and md-tui's XL cells were empty
already (the notes fill them from separate runs). Another version runs them again, with a note on stderr; update
or drop its entry once they finish in time. `B2_TIMEOUTS=off` runs everything.

### Print, previewer files and images

All three use `hyperfine -N` to a pipe at 100 columns, pinned to CPUs 2,4,6,8, one hyperfine call per input with
every tool in it, and take peak RSS from a short call of its own per command: a child's `ru_maxrss` counts what
hyperfine held when it spawned it, and hyperfine grows with every run it keeps, so in a long call a small tool
reports hyperfine's size.

- `b2a.zsh`: the four notes. bat highlights the source and lowdown doesn't highlight, so neither is ranked.
  `COLUMNS=100` is exported because rich lays out at `-w` but crops each line at its console's width (80 to a pipe).
- `b2p.zsh`: inputs in `corpus/` beside the notes: `code-s.rs`, `code-m.rs` (ada's `src/layout/code.rs` and
  `src/highlight.rs`), `code-l.rs` (zerocopy 0.8.61's `src/lib.rs`), `log.log` (1 MB of `journalctl -o short-iso`),
  `huge.log` (80 MB of the same) and `huge.json` (3.75 MB on one line: `cargo metadata` of eight projects as a JSON
  array). `whole` prints the file; `preview` gives ada `--height 50` (its fzf budget, 2,000 rows), bat
  `--line-range=:2000` and rich `--head 2000`. The huge files run under `timeout -s KILL $B2P_CAP`; a killed run
  shows as `> 60 s`, and its CPU and RSS are lost (timeout signals its own process group, so it never reaps the
  child).
- `b2i.zsh`: `corpus/img/`: `fit.png` (686×811, sent to kitty as it is), `diagram.png` (2473×1808), `photo.jpg`
  (1500×2000), `photo-12mp.png`/`.jpg` and `photo-24mp.jpg` (a vault photo resized with ImageMagick), each as a
  100×50-cell preview at 10×20 px. kitty: ada, `kitten icat` streamed and through shared memory (its leftover
  `/dev/shm/icat-*` files are removed), chafa and mcat; text: ada's half-blocks, viu's, and chafa's symbols.
- `B2P_ONLY="ada rich"` with `B2P_OUT=<dir>` reruns some tools into another directory, and
  `B2P_DIRS="b2p <dir>"` merges it over the full run in `doctables.py`; `B2I_ONLY`, `B2I_OUT` and `B2I_DIRS` the
  same for images, `B2A_ONLY` and `B2A_OUT` for print. `merge.py OUT IN...` merges such directories into one, a
  later one replacing an earlier one's tools, so `doctables.py` reads it as a full run. With `ADA_REF` set, all
  three run it as `adaref` beside ada, in the same `hyperfine` call.
- `runcmp.py OLD.jsonl NEW.jsonl` lines up every ranked interactive cell of two runs, with how far the other
  viewers moved: when they all moved, so did the machine. Each job works on its own copy of its document, in `jobs/` beside the spec.

Memory is defragmented before measuring when less than 8 GiB is free (`MemFree`; `B2_DROP_BELOW_GB` sets another
threshold), as oxasm's and sift's benchmarks do: `b2 run` and `b2a.zsh` check once at their start, `b2 flood`
before each process. When free memory is short or fragmented, transparent huge pages fall back to 4 KiB pages and
a viewer's heap faults more. They run `sudo -n /usr/local/sbin/oxasm-drop-caches`, which drops the page cache and
compacts memory; oxasm's `xtask/scripts/oxasm-drop-caches` says how to install it and its password-free sudoers
rule. `B2_DROP_CACHES` names another script, or `off` skips it. Runs and floods record the viewer's
`anon_huge_kib` (`AnonHugePages`): how much of its heap got huge pages.

`b2 floor [SAMPLES] [OUT.json]` measures the harness's own key → pty → timestamp path against the tty's echo,
with no program on the slave. Run it under `taskset` on a lane's cores, alone and beside a run, to see what a
difference has to exceed. `b2 probe CWD KEY... -- CMD...` dumps the emulated screen after each key.

### Holding the cores ready

B2 times one key at a time after the screen held still, so between keys an idle core drops to its floor clock and
into deep idle (C3 exits in 350 µs, against a key's 0.1–0.4 ms): without help, a cell moves with how busy the rest
of the machine is (two busy lanes read 1.24× four) and with the CCD's ramp. `b2-bench-profile on CPUS PID` holds
the listed cores at EPP `performance` with no idle state deeper than C1, and restores them when PID exits; with
it, two busy lanes read as four (median 1.001) and CCD1 as CCD0, at 27–28 °C (ada-optimization, experiment 28). It
runs as root through a password-free sudoers rule (its header has the install lines). `full.zsh`, `rerun.zsh` and
`stress.zsh` hold it for their cores unless `B2_PROFILE=off`, and write `profile.txt`: records with and without it
don't mix, and `rerun.zsh` refuses a REF made the other way (no `profile.txt`: off).

### ada-only rerun

```zsh
B2_ROOT=~/b2 ADA=<new build> ADA_REF=<the build REF measured> zsh bench/b2/rerun.zsh REF OUT
```

Reruns ada's cells (the lanes, print, the previewer files and images) beside the ada that REF measured, both built
fresh from `git archive`, without rerunning the other viewers. REF is a full record set (`b2/all.jsonl`,
`lane0..3.json`, `b2a/`, `b2p/`, `b2i/`; `full.zsh` writes one, `merge.py` builds one from a run and its reruns),
and OUT becomes one, the next rerun's REF. ada's jobs are split into one-run jobs, five rounds: with the profile,
ada on its lanes in REF and `adaref` on the same lanes of the other CCD at once, swapping sides each round; with
`B2_PROFILE=off`, the two in turn on ada's lanes (about 21 minutes). `abcmp.py` writes `OUT/compare.md`: each cell's new/old, the A/B, and the reference
build's old/REF, the machine's drift since REF. A part or measure whose drift is beyond ±3% (`OUT/keep.txt`)
keeps REF's ada cells in the merge, since the other viewers' cells no longer compare with ada's new ones: in an
ada-only rerun usually the interactive cells (two busy lanes against a full run's four: idle clocks) and peak RSS
(the binary file's page-cache state). `B2_MERGE_ONLY=1` redoes only the merge, `compare.md` and `doctables.md`.

### Flood: scrolling at full speed

`b2 flood` sends scroll keys one per write at a fixed rate, or as fast as the pty takes them (`RATE` 0, which
viewers on crossterm's default event source stall on; ada's fix is in the vault note `ada-optimization`, O-21). It prints one JSON line: frames a
second, keys per frame, bytes and `write` calls per frame, CPU per frame and per thread, and key-to-frame latency.
Frames are counted by ratatui's per-draw cursor hide, so those columns mean something only for ratatui viewers.

```zsh
cd $B2_ROOT/corpus
for r in 30 1000 5000 20000; do   # RATE SECS DOWN UP N: N x DOWN, then N x UP, repeating
  taskset -c 8,10 $B2 flood . $r 2 j k 300 -- taskset -c 0,2,4,6 $ADA l.md
  taskset -c 8,10 $B2 flood . $r 2 '\e[6~' '\e[5~' 20 -- taskset -c 0,2,4,6 $ADA l.md
done
```

## Files

| File | Purpose |
|---|---|
| `src/` (`b2`) | pty driver: a reader thread timestamps output and answers terminal queries in order; afterwards, vt100 replays each run to find when the screen showed what |
| `src/flood.rs` | `b2 flood`: keys at a fixed rate, frames, CPU per thread and latency per key |
| `mkspec.py` | jobs: viewers, their keys, documents and sentinels |
| `lanes.py` | splits the jobs into four CPU lanes |
| `report.py`, `rank.py` | tables, and where ada isn't first |
| `doctables.py` | the vault notes' B2 tables in their layout: × columns, the best value per column bold, ranks, the cold resize |
| `plot.py`, `plots/b2.gnuplot` | the charts' data: `plots/interactive.dat` and `print.dat` (the README's `interactive.png` and `print.png`, and the site), `previewer.dat` and `images.dat` (the site only; `B2P_DIRS` and `B2I_DIRS` merge reruns as for `doctables.py`, and a second argument stamps `previewer.dat` when some files were rerun at another commit: `plot.py 137570c "137570c, logs e9ec3b7"`) |
| `b2a.zsh`, `b2a_report.py` | print mode with hyperfine 2 (wall, CPU, peak RSS, instructions) |
| `b2p.zsh` | the previewer files (code, logs, huge files) printed whole and under a preview's budget |
| `b2i.zsh` | image previews against kitten icat, chafa, viu and mcat |
| `runcmp.py` | two runs' interactive cells side by side, for regressions |
| `timeouts.py` | the cells that always hit their cap, skipped while the tool is still that version |
| `full.zsh` | the whole run into one record set, the bench cores held ready |
| `rerun.zsh`, `abcmp.py`, `merge.py` | ada's cells rerun beside the build the last run measured, compared, and merged over that run's records |
| `stress.zsh` | two ada builds A/B'd under steady load, at once on matched cores: `b2 flood` at 20,000 keys a second and `examples/bench.rs`; for regression checks |
| `b2-bench-profile` | root helper: EPP `performance` and no deep idle on the bench cores for a run, restored after |
| `pick.py` | picking search terms |
| `printcmp.zsh`, `sgrcmp.py` | print mode, two builds compared by what each character shows (SGR interpreted, blanks by background), over the vault and the corpus at four widths |
| `termcmp.zsh`, `relay.py` | two builds live in tmux, kitty or alacritty through one key script, compared screen by screen (`sgrcmp.py --screen`, or pixels for alacritty); `relay.py` feeds keys to ada inside alacritty |
