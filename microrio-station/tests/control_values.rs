//! What actually leaves the station on the control side.
//!
//! The control grid shows a `wire` column claiming to be the number being
//! sent. For a while it was not: the offset was applied to the label and
//! never to the frame. These tests pin the two together, and guard the trap
//! that fixing it opened — a default clamp quietly eating a unix timestamp.

use std::collections::HashMap;
use std::net::Ipv4Addr;

use microrio_proto::schema::{ChannelDef, NodeInfo, Schema};
use microrio_proto::{ChanType, Dir};

#[path = "../src/clock.rs"]
mod clock;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/history.rs"]
mod history;
#[path = "../src/logger.rs"]
mod logger;
#[allow(dead_code)]
#[path = "../src/relay.rs"]
mod relay;
#[path = "../src/station.rs"]
mod station;

use config::{ControlConfig, CrossLink, NodeConfig, Profile};
use station::{Intent, Station};

fn ch(dir: Dir, ctype: ChanType, slot: u16, name: &str) -> ChannelDef {
    ChannelDef {
        dir,
        ctype,
        slot,
        name: name.to_string(),
    }
}

fn schema(channels: Vec<ChannelDef>) -> Schema {
    let n = channels.len() as u16;
    Schema {
        info: NodeInfo {
            name: "test".into(),
            rate_hz: 50,
            max_frame: 512,
            total_channels: n,
            history: 1,
        },
        channels,
    }
}

fn node(id: u16) -> NodeConfig {
    NodeConfig {
        id,
        label: format!("n{id}"),
        ip: Ipv4Addr::new(192, 168, 0, 50),
        ..Default::default()
    }
}

fn station_with(nodes: Vec<NodeConfig>) -> std::sync::Arc<Station> {
    Station::new(
        Profile {
            nodes,
            ..Default::default()
        },
        None,
    )
}

/// The regression the offset fix nearly introduced.
///
/// `set_unix` carries about 1.8e9. The old i32 default clamped to ±10 000, so
/// applying the control transform to every channel would have turned clock
/// sync into sending a timestamp in 1970 — and the probe would not have
/// caught it, because the probe does not go through this path.
#[test]
fn unconfigured_i32_is_not_clipped() {
    let st = station_with(vec![node(1)]);
    let s = schema(vec![ch(Dir::Control, ChanType::I32, 0, "set_unix")]);
    let now: i32 = 1_792_981_795;
    st.set_intent(1, "set_unix", Intent::I32(now));

    let out = station::build_values(&st, 1, &s);
    assert_eq!(
        out.values.i32s[0], now,
        "a unix timestamp must reach the wire intact"
    );
}

/// The wire column and the frame must agree, offset included.
#[test]
fn control_offset_and_clamp_reach_the_wire() {
    let mut n = node(1);
    let cc = ControlConfig {
        step: 0.1,
        min: -1.0,
        max: 1.0,
        offset: 0.25,
        ..ControlConfig::default()
    };
    n.controls.insert("drive_x".into(), cc.clone());
    let st = station_with(vec![n]);
    let s = schema(vec![ch(Dir::Control, ChanType::F32, 0, "drive_x")]);

    st.set_intent(1, "drive_x", Intent::F32(0.5));
    let out = station::build_values(&st, 1, &s);
    assert_eq!(out.values.f32s[0], 0.75);
    assert_eq!(out.values.f32s[0] as f64, cc.on_the_wire(0.5));

    // And the clamp is the same clamp, not a second opinion.
    st.set_intent(1, "drive_x", Intent::F32(0.9));
    let out = station::build_values(&st, 1, &s);
    assert_eq!(out.values.f32s[0], 1.0);
    assert_eq!(out.values.f32s[0] as f64, cc.on_the_wire(0.9));
}

/// A link carries node 1's reading onto node 2's output, scaled.
#[test]
fn crosslink_drives_the_destination() {
    let st = station_with(vec![node(1), node(2)]);
    st.profile.lock().unwrap().crosslinks.push(CrossLink {
        from_node: 1,
        from_channel: "a0".into(),
        to_node: 2,
        to_channel: "setpoint".into(),
        slope: 0.002,
        offset: -1.0,
        min: -1.0,
        max: 1.0,
        active: true,
    });

    let src = schema(vec![ch(Dir::Telemetry, ChanType::I32, 0, "a0")]);
    let dst = schema(vec![ch(Dir::Control, ChanType::F32, 0, "setpoint")]);

    {
        let mut views = st.views.lock().unwrap();
        let v = views.entry(1).or_default();
        v.schema = Some(src.clone());
        let mut values = microrio_proto::frames::Values::zeroed(&src.layout(Dir::Telemetry));
        values.i32s[0] = 750;
        v.telemetry = Some(values);
        v.last_rx_unix_ms = station::now_ms();
    }

    // The operator's own value loses to the link, which is the point of it.
    st.set_intent(2, "setpoint", Intent::F32(0.9));

    let out = station::build_values(&st, 2, &dst);
    assert!(out.source_lost.is_none());
    assert!(
        (out.values.f32s[0] - 0.5).abs() < 1e-6,
        "750 * 0.002 - 1.0 = 0.5, got {}",
        out.values.f32s[0]
    );
}

/// A source that has gone quiet must fault the node it was driving, not keep
/// commanding it from the last value anyone saw.
#[test]
fn crosslink_with_a_stale_source_faults() {
    let st = station_with(vec![node(1), node(2)]);
    st.profile.lock().unwrap().crosslinks.push(CrossLink {
        from_node: 1,
        from_channel: "a0".into(),
        to_node: 2,
        to_channel: "setpoint".into(),
        ..Default::default()
    });

    let src = schema(vec![ch(Dir::Telemetry, ChanType::I32, 0, "a0")]);
    let dst = schema(vec![ch(Dir::Control, ChanType::F32, 0, "setpoint")]);

    {
        let mut views = st.views.lock().unwrap();
        let v = views.entry(1).or_default();
        v.schema = Some(src.clone());
        let mut values = microrio_proto::frames::Values::zeroed(&src.layout(Dir::Telemetry));
        values.i32s[0] = 750;
        v.telemetry = Some(values);
        // Last heard from a full second ago: five times the staleness limit.
        v.last_rx_unix_ms = station::now_ms() - 1000;
    }

    let out = station::build_values(&st, 2, &dst);
    assert!(
        out.source_lost.is_some(),
        "a stale source must be reported, not silently held"
    );
}

/// Enable is per node, and the estop outranks all of it.
#[test]
fn enable_is_per_node_and_estop_is_global() {
    use microrio_proto::State;
    let st = station_with(vec![node(1), node(2)]);

    st.set_node_enabled(1, true);
    assert_eq!(st.effective_state(1), State::Enabled);
    assert_eq!(st.effective_state(2), State::Disabled);
    assert_eq!(st.enabled_count(), 1);

    st.set_estop();
    assert_eq!(st.effective_state(1), State::Estop);
    assert_eq!(st.enabled_count(), 0);

    // Enabling is refused while the estop is latched.
    st.set_node_enabled(1, true);
    assert_eq!(st.effective_state(1), State::Estop);

    // And clearing it never re-enables anything on its own.
    st.clear_estop();
    assert_eq!(st.effective_state(1), State::Disabled);
    assert_eq!(st.enabled_count(), 0);
}

/// Intents for a channel the node does not declare are ignored rather than
/// landing in some other channel's slot.
#[test]
fn unknown_channels_are_ignored() {
    let st = station_with(vec![node(1)]);
    let s = schema(vec![ch(Dir::Control, ChanType::F32, 0, "real")]);
    st.set_intent(1, "imaginary", Intent::F32(1.0));
    let out = station::build_values(&st, 1, &s);
    assert_eq!(out.values.f32s, vec![0.0]);
}

#[allow(dead_code)]
fn unused(_: HashMap<u16, u16>) {}

fn stick(st: &Station, node_id: u16, channel: &str, deflection: f32, unipolar: bool) {
    st.joy.lock().unwrap().entry(node_id).or_default().insert(
        channel.into(),
        station::JoyInput {
            source: "LeftStickX".into(),
            deflection,
            unipolar,
        },
    );
}

/// The joystick goes out through the same expo, range, offset and clamp the
/// control row shows, so the grid can display what the stick is sending.
#[test]
fn a_joystick_goes_through_the_channel_settings() {
    let mut n = node(1);
    let cc = ControlConfig {
        min: 0.0,
        max: 10.0,
        offset: 1.0,
        expo: 0.0,
        ..ControlConfig::default()
    };
    n.controls.insert("speed".into(), cc.clone());
    let st = station_with(vec![n]);
    let s = schema(vec![ch(Dir::Control, ChanType::F32, 0, "speed")]);

    // A typed value is overridden while the stick is mapped.
    st.set_intent(1, "speed", Intent::F32(3.0));
    stick(&st, 1, "speed", 0.0, false);
    let out = station::build_values(&st, 1, &s);
    assert_eq!(
        out.values.f32s[0], 6.0,
        "centre stick is mid-range, plus offset"
    );

    stick(&st, 1, "speed", 1.0, false);
    let out = station::build_values(&st, 1, &s);
    assert_eq!(
        out.values.f32s[0], 10.0,
        "full stick is max + offset, clamped to max"
    );
}

/// Hold overrides the stick with a fixed value, starting where it was.
#[test]
fn hold_freezes_a_joystick_channel_without_a_jump() {
    let mut n = node(1);
    n.controls.insert(
        "speed".into(),
        ControlConfig {
            min: 0.0,
            max: 10.0,
            ..ControlConfig::default()
        },
    );
    let st = station_with(vec![n.clone()]);
    let def = ch(Dir::Control, ChanType::F32, 0, "speed");
    let s = schema(vec![def.clone()]);

    stick(&st, 1, "speed", 0.5, false); // 7.5
    st.set_hold(1, &def, &n.controls["speed"], true);
    stick(&st, 1, "speed", -1.0, false); // the stick moves on
    let out = station::build_values(&st, 1, &s);
    assert_eq!(out.values.f32s[0], 7.5, "held where the stick was");

    st.set_hold(1, &def, &n.controls["speed"], false);
    let out = station::build_values(&st, 1, &s);
    assert_eq!(
        out.values.f32s[0], 0.0,
        "released: the stick is back in charge"
    );
}

/// A trigger mapped to a bool reads as pressed past halfway.
#[test]
fn a_button_drives_a_bool() {
    let st = station_with(vec![node(1)]);
    let s = schema(vec![ch(Dir::Control, ChanType::Bool, 0, "led")]);
    stick(&st, 1, "led", 1.0, true);
    assert!(station::build_values(&st, 1, &s).values.bools[0]);
    stick(&st, 1, "led", 0.2, true);
    assert!(!station::build_values(&st, 1, &s).values.bools[0]);
}

// ------------------------------------------------- two stations, one node

use station::{Contest, ContestWatch};

const CONFLICT: u8 = microrio_proto::flag::STATION_CONFLICT;

#[test]
fn a_node_to_itself_is_not_contested() {
    let mut w = ContestWatch::default();
    for _ in 0..200 {
        w.sent();
        assert_eq!(w.frame(0, true), Contest::Alone);
    }
}

#[test]
fn the_owner_is_told_someone_is_knocking() {
    // Our own clock comes back, so we are driving it; the flag says another
    // station tried.
    let mut w = ContestWatch::default();
    w.sent();
    assert_eq!(w.frame(CONFLICT, true), Contest::Shared);
}

#[test]
fn a_refusal_says_we_are_not_in_control() {
    // The flag with someone else's clock is the refusal itself.
    let mut w = ContestWatch::default();
    for _ in 0..5 {
        w.sent();
    }
    assert_eq!(w.frame(CONFLICT, false), Contest::Refused);
}

#[test]
fn old_firmware_is_judged_by_whose_clock_comes_back() {
    // No flag, because the board predates ownership. Our control is going
    // nowhere and the echoes belong to someone else.
    let mut w = ContestWatch::default();
    let mut last = Contest::Alone;
    for _ in 0..40 {
        w.sent();
        last = w.frame(0, false);
    }
    assert_eq!(last, Contest::Refused);

    // The same, but the node is answering both of us in turn: half the
    // telemetry goes elsewhere while our control still lands. That is the
    // old split case, and it must not read as being locked out.
    let mut w = ContestWatch::default();
    let mut last = Contest::Alone;
    for _ in 0..40 {
        w.sent();
        last = w.frame(0, false);
        w.sent();
        last = w.frame(0, true);
    }
    assert_eq!(last, Contest::Shared);
}

#[test]
fn a_handshake_is_not_mistaken_for_a_second_station() {
    // Before our first control frame lands the node echoes zero or whatever
    // it last heard. A few of those must not raise an alarm.
    let mut w = ContestWatch::default();
    for _ in 0..10 {
        assert_eq!(w.frame(0, false), Contest::Alone);
    }
}
