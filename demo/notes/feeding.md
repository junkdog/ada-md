# Feeding

Four meals a day from the feeder, plus whatever she talks someone into. Back to [[index]].

| Time | Portion | Food |
|---|--:|---|
| 07:00 | 20 g | dry, salmon |
| 12:00 | 15 g | dry, salmon |
| 17:00 | 40 g | wet, chicken |
| 22:00 | 10 g | dry, salmon |

> [!TIP]
> The 17:00 meal goes in the bowl by hand. The feeder's lid can't hold wet food, and she has
> learned to open it.

## How long things take

| What | Takes | Per day |
|---|--:|--:|
| reading the hopper's sensor | 350 µs | 1,440 |
| a turn of the motor | 850 ms | 18 |
| Ada, hearing the motor | 0.4 s | 18 |
| writing the log to the SD card | 12 ms | 96 |
| a jam, before the retry | 1m 30s | 0.3 |
| opening the lid | 1.2 s | 4 |
| the 17:00 meal, by hand | 2 min | 1 |
| a firmware update | 45 s | |
| the hopper, full to empty | 6 days | |

## The feeder's config

The schedule lives on the feeder's SD card. The firmware is in
[feeder/src/main.rs](feeder/src/main.rs).

```toml
# feeder.toml
[motor]
grams_per_turn = 2.5
jam_retry = 3

[[meal]]
at = "07:00"
grams = 20

[[meal]]
at = "12:00"
grams = 15

[[meal]]
at = "22:00"
grams = 10
```

## Checking the log

```zsh
# portions dispensed today, with the times the motor jammed and retried
grep "$(date +%F)" /mnt/feeder/log.txt | awk '$3 == "dispensed" { g += $4 } $3 == "jam" { print "jam at", $2 } END { print g " g in total" }'
```

## The log

What the feeder dispensed each day, what was left in the bowl at night, and how often the motor jammed.

| Date | Dispensed | Left | Jams | Notes |
|---|--:|--:|--:|---|
| 2026-09-15 | 45 g | 0 g | 0 |  |
| 2026-09-16 | 45 g | 0 g | 0 |  |
| 2026-09-17 | 45 g | 2 g | 0 |  |
| 2026-09-18 | 45 g | 5 g | 0 | hopper refilled |
| 2026-09-19 | 45 g | 0 g | 0 |  |
| 2026-09-20 | 45 g | 0 g | 0 |  |
| 2026-09-21 | 45 g | 8 g | 0 |  |
| 2026-09-22 | 45 g | 3 g | 0 |  |
| 2026-09-23 | 45 g | 0 g | 1 | jam at 12:00 |
| 2026-09-24 | 45 g | 0 g | 0 |  |
| 2026-09-25 | 45 g | 3 g | 0 |  |
| 2026-09-26 | 45 g | 0 g | 0 |  |
| 2026-09-27 | 45 g | 0 g | 0 | guests; extra treats |
| 2026-09-28 | 45 g | 3 g | 0 |  |
| 2026-09-29 | 45 g | 0 g | 0 |  |
| 2026-09-30 | 45 g | 0 g | 0 |  |
| 2026-10-01 | 45 g | 0 g | 0 |  |
| 2026-10-02 | 45 g | 2 g | 2 | jam at 07:00, twice |
| 2026-10-03 | 45 g | 2 g | 0 |  |
| 2026-10-04 | 45 g | 0 g | 0 |  |
| 2026-10-05 | 45 g | 0 g | 0 |  |
| 2026-10-06 | 45 g | 0 g | 0 | hopper refilled |
| 2026-10-07 | 45 g | 3 g | 0 |  |
| 2026-10-08 | 45 g | 2 g | 0 |  |
| 2026-10-09 | 45 g | 0 g | 0 |  |
| 2026-10-10 | 35 g | 8 g | 0 | skipped the 22:00 meal |
| 2026-10-11 | 45 g | 3 g | 0 |  |
| 2026-10-12 | 45 g | 0 g | 0 |  |
| 2026-10-13 | 45 g | 0 g | 0 |  |
| 2026-10-14 | 45 g | 5 g | 0 | new salmon bag |
