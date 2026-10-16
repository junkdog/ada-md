//! The feeder's firmware: reads the schedule from the SD card, then turns the auger at each
//! meal until the portion is out.

use std::time::Duration;

use feeder::{Clock, Config, Log, Motor};

/// How long one turn of the auger may take before it counts as a jam.
const TURN_TIMEOUT: Duration = Duration::from_millis(800);

fn main() -> Result<(), feeder::Error> {
    let config = Config::load("/sd/feeder.toml")?;
    let mut motor = Motor::new(config.motor.pin);
    let mut log = Log::open("/sd/log.txt")?;
    let clock = Clock::new();

    loop {
        let meal = config.next_meal(clock.now());
        clock.sleep_until(meal.at);
        dispense(&mut motor, &mut log, meal.grams, &config)?;
    }
}

/// Turn the auger until `grams` are out, retrying a jammed turn up to `jam_retry` times.
fn dispense(motor: &mut Motor, log: &mut Log, grams: f32, config: &Config) -> Result<(), feeder::Error> {
    let turns = (grams / config.motor.grams_per_turn).ceil() as u32;
    for _ in 0..turns {
        let mut tries = 0;
        while motor.turn(TURN_TIMEOUT).is_err() {
            tries += 1;
            log.write(format_args!("jam {tries}"))?;
            if tries > config.motor.jam_retry {
                return Err(feeder::Error::Jammed);
            }
            motor.reverse(TURN_TIMEOUT / 2);
        }
    }
    log.write(format_args!("dispensed {grams}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portion_rounds_up_to_whole_turns() {
        let config = Config::default();
        let mut motor = Motor::fake();
        let mut log = Log::memory();
        dispense(&mut motor, &mut log, 20.0, &config).unwrap();
        assert_eq!(motor.turns(), 8);
    }
}
