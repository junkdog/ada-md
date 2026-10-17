# B2: ada against other terminal markdown viewers

The harness behind the B2 section of the vault note `feature-matrix-comparison` (Performance (B2)): pty-driven
latency, CPU and memory for interactive viewers, plus `hyperfine` for print mode.

## Setup

```zsh
export B2_ROOT=~/b2 ADA=$PWD/target/release/ada
mkdir -p $B2_ROOT/{corpus,tools,b2}
# viewers, into a prefix of their own
cargo install --locked --root $B2_ROOT/tools treemd@0.9.1 leaf-markdown-viewer@1.29.1 md-tui@0.11.0 mdcat@2.18.0 hyperfine
cargo install --locked --root $B2_ROOT/tools mdfried@0.22.8 --no-default-features --features svg,mermaid  # no libchafa
# treemd HEAD, if wanted, as tools/bin/treemd-head; glow from the distribution; lowdown built with cc -O2
# the corpus: s.md, m.md, l.md (vault notes) and xl.md (notes over 4 KB concatenated to 2 MB)
cargo build --release --manifest-path bench/b2/Cargo.toml   # the pty driver, bench/b2/target/release/b2
```

## Run

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
```

`b2 floor [SAMPLES] [OUT.json]` measures the harness's own key → pty → timestamp path against the tty's echo,
with no program on the slave. Run it under `taskset` on a lane's cores, alone and beside a run, to see what a
difference has to exceed. `b2 probe CWD KEY... -- CMD...` dumps the emulated screen after each key.

## Files

| File | Purpose |
|---|---|
| `src/` (`b2`) | pty driver: a reader thread timestamps output and answers terminal queries in order; afterwards, vt100 replays each run to find when the screen showed what |
| `mkspec.py` | jobs: viewers, their keys, documents and sentinels |
| `lanes.py` | splits the jobs into four CPU lanes |
| `report.py`, `rank.py` | tables, and where ada isn't first |
| `b2a.zsh`, `b2a_report.py` | print mode with hyperfine 2 (wall, CPU, peak RSS, instructions) |
| `pick.py` | picking search terms |
