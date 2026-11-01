//! Codec roundtrip and robustness tests.
//!
//! These run on the host with no hardware. Integration against a real Mega or
//! ESP32 lives elsewhere — this file only proves the bytes are the bytes.

use microrio_proto::frames::{ControlFrame, TelemFrame, TelemRecord, Values};
use microrio_proto::schema::{
    decode_schema_req, encode_schema_req, ChannelDef, Layout, NodeInfo, Schema, SchemaAssembler,
    SchemaChunk,
};
use microrio_proto::*;
use proptest::prelude::*;

// ---------------------------------------------------------------- strategies

fn layout_strategy() -> impl Strategy<Value = Layout> {
    (0usize..40, 0usize..20, 0usize..20, 0usize..4, 0usize..8).prop_map(
        |(n_bool, n_i32, n_f32, n_str, n_cmd)| Layout {
            n_bool,
            n_i32,
            n_f32,
            n_str,
            n_cmd,
        },
    )
}

/// Non-NaN so `PartialEq` on `Values` is meaningful; NaN is covered separately.
fn finite_f32() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(0.0f32),
        Just(-0.0f32),
        Just(f32::MIN),
        Just(f32::MAX),
        Just(f32::INFINITY),
        Just(f32::NEG_INFINITY),
        any::<i32>().prop_map(|v| v as f32),
        (-1.0e6f32..1.0e6f32),
    ]
}

/// Printable ASCII, no NUL (NUL terminates on the wire), within the field width.
fn wire_string() -> impl Strategy<Value = String> {
    proptest::collection::vec(0x20u8..0x7f, 0..STR_LEN).prop_map(|v| String::from_utf8(v).unwrap())
}

fn values_for(layout: Layout) -> impl Strategy<Value = Values> {
    (
        proptest::collection::vec(any::<bool>(), layout.n_bool),
        proptest::collection::vec(any::<i32>(), layout.n_i32),
        proptest::collection::vec(finite_f32(), layout.n_f32),
        proptest::collection::vec(wire_string(), layout.n_str),
        proptest::collection::vec(any::<u16>(), layout.n_cmd),
    )
        .prop_map(|(bools, i32s, f32s, strs, cmds)| Values {
            bools,
            i32s,
            f32s,
            strs,
            cmds,
        })
}

fn state_strategy() -> impl Strategy<Value = State> {
    prop_oneof![
        Just(State::Disabled),
        Just(State::Enabled),
        Just(State::Estop),
        Just(State::Fault),
    ]
}

// ------------------------------------------------------------------- header

proptest! {
    #[test]
    fn header_roundtrips(node_id: u16, schema_hash: u32, seq: u16, t in 1u8..=4) {
        let msg = MsgType::from_u8(t).unwrap();
        let hdr = Header::new(msg, node_id, schema_hash, seq);

        let mut buf = [0u8; HEADER_LEN];
        let n = hdr.encode(&mut buf).unwrap();
        prop_assert_eq!(n, HEADER_LEN);

        let (back, rest) = Header::decode(&buf).unwrap();
        prop_assert_eq!(back, hdr);
        prop_assert!(rest.is_empty());
    }
}

#[test]
fn header_rejects_bad_magic() {
    let mut buf = [0u8; HEADER_LEN];
    Header::new(MsgType::Telem, 1, 0, 0)
        .encode(&mut buf)
        .unwrap();
    buf[0] ^= 0xFF;
    assert_eq!(Header::decode(&buf), Err(Error::BadMagic));
}

#[test]
fn header_rejects_bad_version() {
    let mut buf = [0u8; HEADER_LEN];
    Header::new(MsgType::Telem, 1, 0, 0)
        .encode(&mut buf)
        .unwrap();
    buf[2] = PROTO_VERSION.wrapping_add(7);
    assert_eq!(
        Header::decode(&buf),
        Err(Error::BadVersion(PROTO_VERSION.wrapping_add(7)))
    );
}

#[test]
fn header_rejects_truncation() {
    let mut buf = [0u8; HEADER_LEN];
    Header::new(MsgType::Telem, 1, 0, 0)
        .encode(&mut buf)
        .unwrap();
    for cut in 0..HEADER_LEN {
        assert_eq!(Header::decode(&buf[..cut]), Err(Error::Truncated));
    }
}

#[test]
fn header_encode_rejects_small_buffer() {
    let mut buf = [0u8; HEADER_LEN - 1];
    assert_eq!(
        Header::new(MsgType::Telem, 1, 0, 0).encode(&mut buf),
        Err(Error::BufferTooSmall)
    );
}

// ------------------------------------------------------------------- frames

/// A frame carrying one record, which is all an Uno ever sends.
fn one_record(values: Values) -> TelemFrame {
    TelemFrame {
        echo_host_time_us: 0,
        state: State::Enabled,
        flags: 0,
        rate_hz: 50,
        records: vec![TelemRecord {
            record: 1,
            node_time_us: 0,
            values,
        }],
    }
}

proptest! {
    #[test]
    fn control_frame_roundtrips(
        layout in layout_strategy(),
        host_time_us: u32,
        state in state_strategy(),
        flags: u8,
        rate_hz: u16,
        ack_record: u32,
    ) {
        let values = values_for(layout).new_tree(&mut TestRunner::default()).unwrap().current();
        let frame = ControlFrame { host_time_us, state, flags, rate_hz, ack_record, values };

        let mut buf = vec![0u8; ControlFrame::body_len(&layout)];
        let n = frame.encode(&layout, &mut buf).unwrap();
        prop_assert_eq!(n, ControlFrame::body_len(&layout));

        let back = ControlFrame::decode(&layout, &buf[..n]).unwrap();
        prop_assert_eq!(back, frame);
    }

    #[test]
    fn telem_frame_roundtrips(
        layout in layout_strategy(),
        echo: u32,
        state in state_strategy(),
        flags: u8,
        rate_hz: u16,
        newest: u32,
        count in 1usize..=8,
        node_time_us: u32,
    ) {
        let mut runner = TestRunner::default();
        let records = (0..count)
            .map(|i| TelemRecord {
                // Numbering wraps like everything else 32-bit on the wire.
                record: newest.wrapping_sub(i as u32),
                node_time_us: node_time_us.wrapping_sub(i as u32 * 20_000),
                values: values_for(layout).new_tree(&mut runner).unwrap().current(),
            })
            .collect();
        let frame = TelemFrame { echo_host_time_us: echo, state, flags, rate_hz, records };

        let mut buf = vec![0u8; TelemFrame::body_len(&layout, count)];
        let n = frame.encode(&layout, &mut buf).unwrap();
        prop_assert_eq!(n, TelemFrame::body_len(&layout, count));

        let back = TelemFrame::decode(&layout, &buf[..n]).unwrap();
        prop_assert_eq!(back, frame);
    }

    #[test]
    fn payload_codec_matches_the_frame_codec(layout in layout_strategy()) {
        let values = values_for(layout).new_tree(&mut TestRunner::default()).unwrap().current();
        let bytes = values.encode_payload(&layout).unwrap();
        prop_assert_eq!(bytes.len(), layout.payload_len());
        prop_assert_eq!(Values::decode_payload(&layout, &bytes).unwrap(), values);
    }
}

#[test]
fn bools_bitpack_lsb_first() {
    let layout = Layout {
        n_bool: 9,
        ..Default::default()
    };
    let mut values = Values::zeroed(&layout);
    values.bools[0] = true;
    values.bools[8] = true;

    let frame = ControlFrame {
        host_time_us: 0,
        state: State::Disabled,
        flags: 0,
        rate_hz: 0,
        ack_record: 0,
        values,
    };
    let mut buf = vec![0u8; ControlFrame::body_len(&layout)];
    frame.encode(&layout, &mut buf).unwrap();

    // 4 time + 1 state + 1 flags + 2 rate + 4 ack, then 2 bytes of packed bools.
    assert_eq!(buf[12], 0b0000_0001);
    assert_eq!(buf[13], 0b0000_0001);
    assert_eq!(layout.bool_bytes(), 2);
}

#[test]
fn nan_survives_as_a_bit_pattern() {
    let layout = Layout {
        n_f32: 1,
        ..Default::default()
    };
    let mut values = Values::zeroed(&layout);
    values.f32s[0] = f32::NAN;

    let frame = one_record(values);
    let mut buf = vec![0u8; TelemFrame::body_len(&layout, 1)];
    let n = frame.encode(&layout, &mut buf).unwrap();
    let back = TelemFrame::decode(&layout, &buf[..n]).unwrap();
    assert!(back.newest().values.f32s[0].is_nan());
}

#[test]
fn oversized_string_is_truncated_not_rejected() {
    let layout = Layout {
        n_str: 1,
        ..Default::default()
    };
    let mut values = Values::zeroed(&layout);
    values.strs[0] = "x".repeat(STR_LEN + 20);

    let frame = one_record(values);
    let mut buf = vec![0u8; TelemFrame::body_len(&layout, 1)];
    let n = frame.encode(&layout, &mut buf).unwrap();
    let back = TelemFrame::decode(&layout, &buf[..n]).unwrap();
    assert_eq!(back.newest().values.strs[0].len(), STR_LEN);
}

#[test]
fn values_must_match_layout() {
    let layout = Layout {
        n_i32: 3,
        ..Default::default()
    };
    let frame = ControlFrame {
        host_time_us: 0,
        state: State::Disabled,
        flags: 0,
        rate_hz: 0,
        ack_record: 0,
        values: Values::zeroed(&Layout {
            n_i32: 2,
            ..Default::default()
        }),
    };
    let mut buf = vec![0u8; 128];
    assert_eq!(frame.encode(&layout, &mut buf), Err(Error::LayoutMismatch));
}

#[test]
fn truncated_body_is_rejected_not_guessed() {
    let layout = Layout {
        n_i32: 4,
        n_f32: 4,
        ..Default::default()
    };
    let frame = ControlFrame {
        host_time_us: 1,
        state: State::Enabled,
        flags: 0,
        rate_hz: 50,
        ack_record: 9,
        values: Values::zeroed(&layout),
    };
    let mut buf = vec![0u8; ControlFrame::body_len(&layout)];
    let n = frame.encode(&layout, &mut buf).unwrap();
    for cut in 0..n {
        assert_eq!(
            ControlFrame::decode(&layout, &buf[..cut]),
            Err(Error::Truncated),
            "a {cut}-byte body should not decode"
        );
    }
}

proptest! {
    #[test]
    fn rtt_is_correct_across_the_32_bit_wrap(echo: u32, elapsed in 0u32..60_000_000) {
        let mut frame = one_record(Values::default());
        frame.echo_host_time_us = echo;
        prop_assert_eq!(frame.rtt_us(echo.wrapping_add(elapsed)), elapsed);
    }
}

// ------------------------------------------------------------------- schema

fn demo_schema(n_channels: usize, max_frame: u16) -> Schema {
    let mut channels = Vec::new();
    for i in 0..n_channels {
        let (dir, ctype) = match i % 4 {
            0 => (Dir::Telemetry, ChanType::Bool),
            1 => (Dir::Telemetry, ChanType::F32),
            2 => (Dir::Control, ChanType::I32),
            _ => (Dir::Control, ChanType::Cmd),
        };
        let slot = channels
            .iter()
            .filter(|c: &&ChannelDef| c.dir == dir && c.ctype == ctype)
            .count() as u16;
        channels.push(ChannelDef {
            dir,
            ctype,
            slot,
            name: format!("channel_{i:03}"),
        });
    }
    Schema {
        info: NodeInfo {
            name: "drivebase".into(),
            rate_hz: DEFAULT_RATE_HZ,
            max_frame,
            total_channels: n_channels as u16,
            history: 8,
        },
        channels,
    }
}

#[test]
fn schema_survives_chunking_and_reassembly() {
    for &(n, frame) in &[
        (1usize, 128u16),
        (8, 128),
        (40, 256),
        (120, 512),
        (200, 1400),
    ] {
        let schema = demo_schema(n, frame);
        let chunks = schema.encode_chunks(7, 0).unwrap();

        for c in &chunks {
            assert!(
                c.len() <= frame as usize,
                "chunk of {} bytes exceeds the node's {frame}-byte budget",
                c.len()
            );
        }

        let mut asm = SchemaAssembler::new();
        let mut rebuilt = None;
        for raw in &chunks {
            let (hdr, body) = Header::decode(raw).unwrap();
            assert_eq!(hdr.msg, MsgType::Schema);
            assert_eq!(hdr.node_id, 7);
            assert_eq!(hdr.schema_hash, schema.hash());
            rebuilt = asm.push(SchemaChunk::decode(body).unwrap());
        }
        assert_eq!(rebuilt.as_ref(), Some(&schema), "n={n} frame={frame}");
    }
}

#[test]
fn assembler_tolerates_reorder_and_duplicates() {
    let schema = demo_schema(90, 256);
    let chunks = schema.encode_chunks(1, 0).unwrap();
    assert!(
        chunks.len() > 2,
        "need several chunks for this to mean anything"
    );

    let mut asm = SchemaAssembler::new();
    let mut order: Vec<usize> = (0..chunks.len()).collect();
    order.reverse();
    order.push(0); // duplicate
    order.push(chunks.len() - 1); // duplicate

    let mut done = None;
    for i in order {
        let (_, body) = Header::decode(&chunks[i]).unwrap();
        done = asm.push(SchemaChunk::decode(body).unwrap()).or(done);
    }
    assert_eq!(done.as_ref(), Some(&schema));
}

#[test]
fn assembler_reports_what_is_missing() {
    let schema = demo_schema(90, 256);
    let chunks = schema.encode_chunks(1, 0).unwrap();

    let mut asm = SchemaAssembler::new();
    let (_, body) = Header::decode(&chunks[0]).unwrap();
    assert!(asm.push(SchemaChunk::decode(body).unwrap()).is_none());

    let missing = asm.missing();
    assert_eq!(missing.len(), chunks.len() - 1);
    assert!(!missing.contains(&0));
}

#[test]
fn hash_ignores_node_identity_but_tracks_structure() {
    let a = demo_schema(12, 256);

    // Same channels, different node name and rate: same schema.
    let mut b = a.clone();
    b.info.name = "elevator".into();
    b.info.rate_hz = 100;
    assert_eq!(a.hash(), b.hash());

    // Renamed channel: different schema.
    let mut c = a.clone();
    c.channels[3].name = "renamed".into();
    assert_ne!(a.hash(), c.hash());

    // Reordered channels: different schema, because slot order is the layout.
    let mut d = a.clone();
    d.channels.swap(0, 4);
    assert_ne!(a.hash(), d.hash());

    // Changed type: different schema.
    let mut e = a.clone();
    e.channels[1].ctype = ChanType::I32;
    assert_ne!(a.hash(), e.hash());
}

#[test]
fn layout_counts_each_direction_separately() {
    let schema = demo_schema(8, 256);
    let t = schema.layout(Dir::Telemetry);
    let c = schema.layout(Dir::Control);

    assert_eq!(t.n_bool, 2);
    assert_eq!(t.n_f32, 2);
    assert_eq!(t.n_i32, 0);
    assert_eq!(c.n_i32, 2);
    assert_eq!(c.n_cmd, 2);
    assert_eq!(c.n_bool, 0);

    assert_eq!(t.payload_len(), 1 + 2 * 4);
    assert_eq!(c.payload_len(), 2 * 4 + 2 * 2);
}

#[test]
fn validate_rejects_cmd_in_the_telemetry_direction() {
    let mut schema = demo_schema(4, 256);
    schema.channels.push(ChannelDef {
        dir: Dir::Telemetry,
        ctype: ChanType::Cmd,
        slot: 0,
        name: "bad".into(),
    });
    assert_eq!(schema.validate(), Err(Error::CmdMustBeControl));
}

#[test]
fn validate_rejects_an_unworkably_small_frame() {
    let schema = demo_schema(4, (MIN_MAX_FRAME - 1) as u16);
    assert_eq!(
        schema.validate(),
        Err(Error::FrameSizeTooSmall((MIN_MAX_FRAME - 1) as u16))
    );
}

#[test]
fn validate_rejects_overlong_and_empty_names() {
    let mut long = demo_schema(2, 256);
    long.channels[0].name = "n".repeat(MAX_NAME_LEN + 1);
    assert_eq!(long.validate(), Err(Error::BadName));

    let mut empty = demo_schema(2, 256);
    empty.channels[0].name = String::new();
    assert_eq!(empty.validate(), Err(Error::BadName));
}

proptest! {
    #[test]
    fn schema_req_roundtrips(chunk_index: u16) {
        let mut buf = [0u8; 2];
        let n = encode_schema_req(chunk_index, &mut buf).unwrap();
        prop_assert_eq!(decode_schema_req(&buf[..n]).unwrap(), chunk_index);
    }
}

#[test]
fn schema_chunk_rejects_a_truncated_body() {
    let schema = demo_schema(20, 256);
    let chunks = schema.encode_chunks(1, 0).unwrap();
    let (_, body) = Header::decode(&chunks[0]).unwrap();
    for cut in 0..body.len() {
        assert!(
            SchemaChunk::decode(&body[..cut]).is_err(),
            "a {cut}-byte chunk body should not decode"
        );
    }
}

use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

#[test]
fn checked_rtt_rejects_an_echo_that_is_not_ours() {
    let frame = |echo| {
        let mut f = one_record(Values::default());
        f.echo_host_time_us = echo;
        f
    };

    // A normal round trip.
    assert_eq!(
        frame(1_000_000).rtt_us_checked(1_005_000, 1_000_000),
        Some(5_000)
    );

    // The node has not had a control frame yet.
    assert_eq!(frame(0).rtt_us_checked(5_000, 1_000_000), None);

    // A previous station's clock: wrapping subtraction gives a huge number,
    // which is exactly the garbage that must not reach the display.
    let stale = frame(60_000_000);
    assert!(stale.rtt_us(2_000_000) > 4_000_000_000);
    assert_eq!(stale.rtt_us_checked(2_000_000, 1_000_000), None);
}
