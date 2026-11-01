//! What the station says about a node that has told it nothing.
//!
//! A board left holding the USB test sketch answers nothing on the network,
//! and for an afternoon the only thing on screen was "waiting for the node's
//! schema". That reads as a fault in the station and sends you hunting
//! through its settings, when every cause is at the other end of the wire.
//! These pin the message to what is actually known.

use std::net::Ipv4Addr;

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

use config::NodeConfig;
use station::{no_schema_note, NodeView};

fn node() -> NodeConfig {
    NodeConfig {
        ip: Ipv4Addr::new(192, 168, 0, 50),
        port: 5005,
        ..NodeConfig::default()
    }
}

#[test]
fn a_fresh_link_is_described_as_asking() {
    let view = NodeView {
        schema_reqs: 3,
        ..Default::default()
    };
    let note = no_schema_note(&node(), &view);
    assert!(note.contains("192.168.0.50:5005"), "{note}");
    assert!(
        note.contains("Asking"),
        "a handshake a few frames old is still a handshake: {note}"
    );
}

#[test]
fn a_silent_board_is_named_as_silent_with_the_three_reasons() {
    let view = NodeView {
        schema_reqs: 400,
        ..Default::default()
    };
    let note = no_schema_note(&node(), &view);
    assert!(note.contains("192.168.0.50:5005"), "{note}");
    assert!(note.contains("400"), "say how many times it has asked: {note}");
    // The three things to check, all of them on the board's end.
    for reason in ["switched off", "different address", "not a"] {
        assert!(note.contains(reason), "missing '{reason}': {note}");
    }
}

#[test]
fn chunks_coming_back_means_the_handshake_is_running() {
    let view = NodeView {
        schema_reqs: 400,
        schema_rx: 2,
        ..Default::default()
    };
    let note = no_schema_note(&node(), &view);
    assert!(note.contains("2 chunks"), "{note}");
    assert!(
        !note.contains("switched off"),
        "a board that is answering is not off: {note}"
    );
}
