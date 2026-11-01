//! Joystick conditioning.
//!
//! Pure maths, so it is worth pinning down: this is what stands between a
//! drifting stick and an actuator that creeps when nobody is touching it.
//! A mapping cleans the input up (invert, trim, deadband); the channel then
//! shapes and ranges it (expo, min..max), as it does for a typed value.

#[allow(dead_code)]
#[path = "../src/config.rs"]
mod config;

use config::{ControlConfig, Mapping};

fn base() -> Mapping {
    Mapping {
        source: "LeftStickX".into(),
        node_id: 1,
        channel: "drive_x".into(),
        trim: 0.0,
        deadband: 0.0,
        invert: false,
    }
}

fn chan(min: f64, max: f64, expo: f64) -> ControlConfig {
    ControlConfig {
        min,
        max,
        expo,
        ..ControlConfig::default()
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn plain_mapping_is_the_identity() {
    let m = base();
    for raw in [-1.0f32, -0.5, 0.0, 0.25, 1.0] {
        assert!((m.deflection(raw, false) - raw).abs() < 1e-6);
    }
}

#[test]
fn a_stick_spans_the_channel_range_centre_to_centre() {
    let c = chan(0.0, 100.0, 0.0);
    assert!(close(c.from_stick(-1.0, false), 0.0));
    assert!(
        close(c.from_stick(0.0, false), 50.0),
        "centre of the stick is centre of the range"
    );
    assert!(close(c.from_stick(1.0, false), 100.0));
}

#[test]
fn a_trigger_rests_at_min_and_reaches_max() {
    // L2 on a DualShock is a button with travel, 0 at rest.
    let c = chan(0.0, 1.0, 0.0);
    assert!(
        close(c.from_stick(0.0, true), 0.0),
        "at rest is min, not the middle"
    );
    assert!(close(c.from_stick(0.5, true), 0.5));
    assert!(close(c.from_stick(1.0, true), 1.0));
}

#[test]
fn expo_softens_the_start_and_still_reaches_full_scale() {
    let linear = chan(-1.0, 1.0, 0.0);
    let soft = chan(-1.0, 1.0, 0.6);
    // Small inputs are damped: a quarter stick gives well under a quarter.
    let q = soft.from_stick(0.25, false);
    assert!(q > 0.0 && q < 0.25 * 0.5, "quarter stick gave {q}");
    assert!(q < linear.from_stick(0.25, false));
    // Full deflection is untouched in both directions.
    assert!(close(soft.from_stick(1.0, false), 1.0));
    assert!(close(soft.from_stick(-1.0, false), -1.0));
    // Symmetric about the centre.
    assert!(close(
        soft.from_stick(-0.4, false),
        -soft.from_stick(0.4, false)
    ));
    // Monotonic: more stick is never less output.
    let mut last = f64::MIN;
    for i in 0..=100 {
        let v = soft.from_stick(-1.0 + i as f64 * 0.02, false);
        assert!(v >= last);
        last = v;
    }
}

#[test]
fn expo_on_a_trigger_is_soft_from_rest() {
    let soft = chan(0.0, 1.0, 0.6);
    assert!(soft.from_stick(0.25, true) < 0.25 * 0.5);
    assert!(close(soft.from_stick(1.0, true), 1.0));
}

#[test]
fn invert_flips_a_stick_about_its_centre() {
    let m = Mapping {
        invert: true,
        ..base()
    };
    assert!((m.deflection(1.0, false) + 1.0).abs() < 1e-6);
    assert!((m.deflection(-0.25, false) - 0.25).abs() < 1e-6);
}

#[test]
fn invert_on_a_trigger_swaps_rest_and_full() {
    let m = Mapping {
        invert: true,
        ..base()
    };
    assert!((m.deflection(0.0, true) - 1.0).abs() < 1e-6);
    assert!(
        m.deflection(1.0, true).abs() < 1e-6,
        "not -1: a trigger has no negative side"
    );
}

#[test]
fn deadband_kills_drift_without_creating_a_jump() {
    let m = Mapping {
        deadband: 0.1,
        ..base()
    };
    // The resting drift this very controller shows must produce exactly zero.
    assert_eq!(
        m.deflection(-0.008, false),
        0.0,
        "stick drift must not move anything"
    );
    assert_eq!(m.deflection(0.05, false), 0.0);
    // Just past the deadband the output starts from zero, not from 0.1.
    assert!(
        m.deflection(0.11, false).abs() < 0.02,
        "first live position is near zero"
    );
    // Full deflection still reaches the ends.
    assert!((m.deflection(1.0, false) - 1.0).abs() < 1e-6);
    assert!((m.deflection(-1.0, false) + 1.0).abs() < 1e-6);
}

#[test]
fn trim_recentres_a_stick() {
    let m = Mapping {
        trim: 0.5,
        ..base()
    };
    assert!((m.deflection(0.0, false) - 0.5).abs() < 1e-6);
    assert!(
        (m.deflection(1.0, false) - 1.0).abs() < 1e-6,
        "clamped, not 1.5"
    );
}

#[test]
fn an_old_profile_mapping_with_min_max_expo_still_loads() {
    let old = r#"
        source = "LeftStickX"
        node_id = 1
        channel = "drive_x"
        min = -1.0
        max = 1.0
        trim = 0.5
        expo = 0.3
    "#;
    let m: Mapping = toml::from_str(old).unwrap();
    assert_eq!(m.trim, 0.5);
}
