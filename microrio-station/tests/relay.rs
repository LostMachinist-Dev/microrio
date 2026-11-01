//! Relayed telemetry: what a watching station sees.
//!
//! The board never sends to a watcher — replying is what costs a board its
//! timing — so everything a watcher shows arrives this way, from the station
//! that owns the node.

use std::net::Ipv4Addr;

#[path = "../src/clock.rs"]
#[allow(dead_code)]
mod clock;
#[path = "../src/config.rs"]
#[allow(dead_code)]
mod config;
#[path = "../src/relay.rs"]
#[allow(dead_code)]
mod relay;

use microrio_proto::frames::{TelemFrame, TelemRecord, Values};
use microrio_proto::schema::{ChannelDef, NodeInfo, Schema};
use microrio_proto::{ChanType, Dir, Header, MsgType, State, HEADER_LEN};

fn schema() -> Schema {
    Schema {
        info: NodeInfo {
            name: "bench".into(),
            rate_hz: 50,
            max_frame: 512,
            total_channels: 2,
            history: 4,
        },
        channels: vec![
            ChannelDef {
                dir: Dir::Telemetry,
                ctype: ChanType::F32,
                slot: 0,
                name: "sine".into(),
            },
            ChannelDef {
                dir: Dir::Control,
                ctype: ChanType::F32,
                slot: 0,
                name: "drive_x".into(),
            },
        ],
    }
}

/// A telemetry frame exactly as it comes off the wire, which is exactly what
/// gets relayed: the same bytes, so a watcher decodes them with the same code.
fn wire_frame(s: &Schema, value: f32) -> Vec<u8> {
    let layout = s.layout(Dir::Telemetry);
    let frame = TelemFrame {
        echo_host_time_us: 1234,
        state: State::Enabled,
        flags: 0,
        rate_hz: 50,
        records: vec![TelemRecord {
            record: 7,
            node_time_us: 99,
            values: Values {
                f32s: vec![value],
                ..Default::default()
            },
        }],
    };
    let mut pkt = vec![0u8; HEADER_LEN];
    Header::new(MsgType::Telem, 3, s.hash(), 11)
        .encode(&mut pkt)
        .unwrap();
    let mut body = vec![0u8; TelemFrame::body_len(&layout, 1)];
    let n = frame.encode(&layout, &mut body).unwrap();
    pkt.extend_from_slice(&body[..n]);
    pkt
}

#[tokio::test]
async fn a_watcher_sees_the_schema_and_the_frames() {
    // A port of its own, so this test cannot be confused by a station running
    // on the same machine.
    const PORT: u16 = 5099;
    let mut listener = relay::Listener::bind(PORT).expect("listener");
    let mut send = relay::Relay::bind(PORT, Ipv4Addr::new(127, 0, 0, 1))
        .await
        .expect("relay");

    let s = schema();
    send.schema(3, &s).await;
    send.frame(&wire_frame(&s, 0.75)).await;

    let mut got_schema = None;
    let mut got_value = None;
    for _ in 0..8 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), listener.next()).await {
            Ok(relay::Relayed::Schema(id, body)) => {
                let chunk = microrio_proto::schema::SchemaChunk::decode(&body).unwrap();
                got_schema = Some((id, chunk.channels.len(), chunk.info.map(|i| i.history)));
            }
            Ok(relay::Relayed::Telemetry(id, hash, body)) => {
                assert_eq!(id, 3);
                assert_eq!(hash, s.hash(), "a watcher can tell a reflashed board");
                let f = TelemFrame::decode(&s.layout(Dir::Telemetry), &body).unwrap();
                got_value = Some(f.newest().values.f32s[0]);
            }
            Err(_) => break,
        }
        if got_schema.is_some() && got_value.is_some() {
            break;
        }
    }

    assert_eq!(
        got_schema,
        Some((3, 2, Some(4))),
        "the schema arrives, so a watcher that never speaks to the board can \
         still name its channels"
    );
    assert_eq!(got_value, Some(0.75), "and the readings arrive unchanged");
}

#[tokio::test]
async fn the_schema_is_repeated_but_not_every_frame() {
    const PORT: u16 = 5098;
    let mut listener = relay::Listener::bind(PORT).expect("listener");
    let mut send = relay::Relay::bind(PORT, Ipv4Addr::new(127, 0, 0, 1))
        .await
        .expect("relay");
    let s = schema();

    // Called every frame, as the station does; it must send one and then hold
    // off, or a watcher's picture would cost more than the telemetry does.
    for _ in 0..5 {
        send.schema(3, &s).await;
    }
    let mut schemas = 0;
    while let Ok(Ok(msg)) = tokio::time::timeout(std::time::Duration::from_millis(300), async {
        Ok::<_, ()>(listener.next().await)
    })
    .await
    {
        if matches!(msg, relay::Relayed::Schema(..)) {
            schemas += 1;
        }
    }
    assert_eq!(schemas, 1, "sent once, not five times");
}
