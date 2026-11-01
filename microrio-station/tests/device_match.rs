//! Joystick profiles have to survive moving between machines.

#[allow(dead_code)]
#[path = "../src/clock.rs"]
mod clock;

#[path = "../src/config.rs"]
mod config;
#[allow(dead_code)]
#[path = "../src/history.rs"]
mod history;
#[allow(dead_code)]
#[path = "../src/joystick.rs"]
mod joystick;
#[allow(dead_code)]
#[path = "../src/logger.rs"]
mod logger;
#[allow(dead_code)]
#[allow(dead_code)]
#[path = "../src/relay.rs"]
mod relay;
#[path = "../src/station.rs"]
mod station;

use joystick::match_device;

#[test]
fn exact_name_wins() {
    let cfg = vec!["PS4 Controller".to_string(), "Xbox Pad".to_string()];
    assert_eq!(
        match_device("Xbox Pad", 2, &cfg).map(String::as_str),
        Some("Xbox Pad")
    );
}

#[test]
fn case_differences_do_not_break_a_profile() {
    let cfg = vec!["ps4 controller".to_string(), "Xbox Pad".to_string()];
    assert_eq!(
        match_device("PS4 Controller", 2, &cfg).map(String::as_str),
        Some("ps4 controller")
    );
}

#[test]
fn one_pad_and_one_entry_are_assumed_to_be_each_other() {
    // The same DualShock reports a different name on Windows than on Linux.
    // With a single controller there is nothing else it could be.
    let cfg = vec!["PS4 Controller".to_string()];
    assert_eq!(
        match_device("Wireless Controller", 1, &cfg).map(String::as_str),
        Some("PS4 Controller")
    );
}

#[test]
fn ambiguity_is_left_alone_rather_than_guessed() {
    // Two pads, two entries, no name agreement: silently driving the wrong
    // channel is worse than driving nothing.
    let cfg = vec!["PS4 Controller".to_string(), "Xbox Pad".to_string()];
    assert_eq!(match_device("Wireless Controller", 2, &cfg), None);

    // One configured entry but two pads plugged in is equally ambiguous.
    let one = vec!["PS4 Controller".to_string()];
    assert_eq!(match_device("Wireless Controller", 2, &one), None);
}
