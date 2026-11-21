#!/usr/bin/env zsh
# The notes the site's clips read: demo/setup.zsh's scratch repository (notes/, with staged and
# unstaged changes, for the gutter clip only); clean/, a clone of its last commit, which has no
# changes and so no gutter, for every other clip; and log/, the feeder firmware's own repository,
# four commits of Rust and TOML, for the git pager's `git log -p`. Prints the scratch directory.
set -euo pipefail

base=$(${0:A:h:h:h}/demo/setup.zsh)
git clone -q $base/notes $base/clean

# log/: the firmware first, then its schedule, a shorter jam timeout with a stop for a paw on the
# lid, and a smaller lunch with a test for the paw; each commit on a fixed date, so the hashes are
# the same every recording
mkdir -p $base/log/src
cp $base/clean/feeder/src/main.rs $base/log/src/main.rs
cd $base/log
git init -q -b main
commit() {
  GIT_AUTHOR_DATE="$1T21:30:00+02:00" GIT_COMMITTER_DATE="$1T21:30:00+02:00" \
    command git -c user.name=ada -c user.email=ada@example.com -c commit.gpgsign=false \
    commit -qam "$2"
}

git add src/main.rs
commit 2026-09-20 "turn the auger at each meal"

cat > feeder.toml <<'EOF'
# The schedule, read from the SD card at boot.
[motor]
pin = 18
grams_per_turn = 2.5
jam_retry = 3

[[meal]]
at = "07:00"
grams = 20

[[meal]]
at = "12:00"
grams = 15

[[meal]]
at = "17:00"  # by hand, at Ada's insistence

[[meal]]
at = "22:00"
grams = 10
EOF
git add feeder.toml
commit 2026-10-03 "keep the schedule on the card"

perl -pi -e 's/from_millis\(800\)/from_millis(650)/' src/main.rs
perl -0pi -e 's/(        while motor\.turn\(TURN_TIMEOUT\)\.is_err\(\) \{\n)/$1            if motor.lid_pressed() {\n                log.write(format_args!("paw on the lid"))?;\n                return Err(feeder::Error::Paw);\n            }\n/' src/main.rs
commit 2026-10-08 "jam sooner, and stop for a paw on the lid"

perl -pi -e 's/^grams = 15$/grams = 12/' feeder.toml
PAW_TEST='
    #[test]
    fn a_paw_on_the_lid_stops_the_meal() {
        let config = Config::default();
        let mut motor = Motor::fake().with_paw();
        let mut log = Log::memory();
        let meal = dispense(&mut motor, &mut log, 20.0, &config);
        assert!(matches!(meal, Err(feeder::Error::Paw)));
    }
' perl -0pi -e 's/(        assert_eq!\(motor\.turns\(\), 8\);\n    \}\n)/$1$ENV{PAW_TEST}/' src/main.rs
commit 2026-10-15 "a smaller lunch, and a test for the paw"

print -r -- $base
