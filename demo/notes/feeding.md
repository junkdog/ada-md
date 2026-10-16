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
