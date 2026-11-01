//! Hardware conformance test.
//!
//! Drives `examples/Conformance` on a real board over USB serial and checks the
//! firmware's bytes against `microrio-proto`. This is the test that catches the
//! classic cross-language bugs — endianness, bit order, struct padding,
//! alignment — on both an 8-bit AVR and a 32-bit ESP32, with no network in the
//! way.
//!
//! Run: `cargo xtask conformance /dev/ttyACM0`

use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

use microrio_proto::frames::{ControlFrame, TelemFrame, Values};
use microrio_proto::schema::{
    encode_schema_req, ChannelDef, NodeInfo, Schema, SchemaAssembler, SchemaChunk,
};
use microrio_proto::{ChanType, Dir, Header, MsgType, State, HEADER_LEN};

const NODE_ID: u16 = 7;

pub fn run(port_name: &str) -> Result<(), String> {
    let port = serialport::new(port_name, 115_200)
        .timeout(Duration::from_millis(1500))
        .open()
        .map_err(|e| format!("cannot open {port_name}: {e}"))?;

    let mut board = Board::new(port)?;
    let mut t = Harness::default();

    println!("== MicroRIO conformance: {port_name} ==\n");

    let expected = expected_schema();

    // 1. Schema hash, computed independently on each side.
    let fw_hash = board.wait_hash()?;
    t.check(
        "schema hash matches the Rust codec",
        fw_hash == expected.hash(),
        &format!("firmware 0x{fw_hash:08x}, rust 0x{:08x}", expected.hash()),
    );

    // 2. Schema handshake, including chunking.
    let schema = board.fetch_schema()?;
    t.check(
        "node info decodes",
        schema.info == expected.info,
        &format!("{:?}", schema.info),
    );
    t.check(
        "channel count matches",
        schema.channels.len() == expected.channels.len(),
        &format!(
            "got {}, expected {}",
            schema.channels.len(),
            expected.channels.len()
        ),
    );
    let mut mismatched = Vec::new();
    for (got, want) in schema.channels.iter().zip(expected.channels.iter()) {
        if got != want {
            mismatched.push(format!("{got:?} != {want:?}"));
        }
    }
    t.check(
        "every channel matches in canonical order",
        mismatched.is_empty(),
        &mismatched.join("; "),
    );
    t.check(
        "schema reassembles to the hash the firmware reported",
        schema.hash() == fw_hash,
        &format!("0x{:08x} vs 0x{fw_hash:08x}", schema.hash()),
    );
    t.check(
        "schema was split into more than one chunk",
        board.last_chunk_count > 1,
        &format!("{} chunk(s) at a 192-byte budget", board.last_chunk_count),
    );

    // 3. Control: Rust encodes, firmware decodes.
    let ctrl_layout = expected.layout(Dir::Control);
    let control = ControlFrame {
        host_time_us: 0x1234_5678,
        state: State::Enabled,
        flags: 0,
        rate_hz: 0,
        ack_record: 0,
        values: Values {
            bools: vec![true, false],
            i32s: vec![4242],
            f32s: vec![0.25, -1.5],
            strs: vec!["teleop".into()],
            cmds: vec![1, 0],
        },
    };
    board.send_control(&expected, &ctrl_layout, &control)?;

    let vals = board.dump()?;
    t.check(
        "control bool slot 0 (led)",
        vals.get("led") == Some(&"1".into()),
        &fmt(&vals, "led"),
    );
    t.check(
        "control bool slot 1 (intake)",
        vals.get("intake") == Some(&"0".into()),
        &fmt(&vals, "intake"),
    );
    t.check(
        "control i32 (target_ticks)",
        vals.get("target_ticks") == Some(&"4242".into()),
        &fmt(&vals, "target_ticks"),
    );
    t.check(
        "control f32 slot 0 (drive_x)",
        near(&vals, "drive_x", 0.25),
        &fmt(&vals, "drive_x"),
    );
    t.check(
        "control f32 slot 1 (drive_y)",
        near(&vals, "drive_y", -1.5),
        &fmt(&vals, "drive_y"),
    );
    t.check(
        "control str (mode)",
        vals.get("mode") == Some(&"teleop".into()),
        &fmt(&vals, "mode"),
    );
    t.check(
        "cmd counter 0 fired",
        vals.get("fired_zero") == Some(&"1".into()),
        &fmt(&vals, "fired_zero"),
    );
    t.check(
        "cmd counter 1 did not fire",
        vals.get("fired_reset") == Some(&"0".into()),
        &fmt(&vals, "fired_reset"),
    );
    t.check(
        "state is ENABLED and link is up",
        vals.get("__state") == Some(&"1".into()) && vals.get("__linked") == Some(&"1".into()),
        &format!(
            "state={:?} linked={:?}",
            vals.get("__state"),
            vals.get("__linked")
        ),
    );

    // A command counter is a level, not a pulse: re-sending the same value
    // must not re-fire.
    board.send_control(&expected, &ctrl_layout, &control)?;
    let vals = board.dump()?;
    t.check(
        "resending the same cmd counter does not re-fire",
        vals.get("fired_zero") == Some(&"0".into()),
        &fmt(&vals, "fired_zero"),
    );

    // 4. Telemetry: firmware encodes, Rust decodes.
    let telem_layout = expected.layout(Dir::Telemetry);
    let frame = board.fetch_telemetry(&telem_layout)?;
    let v = &frame.newest().values;
    t.check(
        "telem bools bitpack correctly",
        v.bools == vec![true, false, true],
        &format!("{:?}", v.bools),
    );
    t.check(
        "telem i32 positive",
        v.i32s[0] == 1234,
        &format!("{}", v.i32s[0]),
    );
    t.check(
        "telem i32 negative",
        v.i32s[1] == -98765,
        &format!("{}", v.i32s[1]),
    );
    t.check(
        "telem f32 positive",
        v.f32s[0] == 12.5,
        &format!("{}", v.f32s[0]),
    );
    t.check(
        "telem f32 negative fraction",
        v.f32s[1] == -0.375,
        &format!("{}", v.f32s[1]),
    );
    t.check(
        "telem str",
        v.strs[0] == "hello harness",
        &v.strs[0].clone(),
    );
    t.check(
        "echoed host timestamp round-trips",
        frame.echo_host_time_us == control.host_time_us,
        &format!("0x{:08x}", frame.echo_host_time_us),
    );
    t.check(
        "with nothing acknowledged, only the newest record is sent",
        frame.records.len() == 1 && frame.newest().record >= 1,
        &format!(
            "{} record(s), newest {}",
            frame.records.len(),
            frame.newest().record
        ),
    );

    // 5. Records and the ack. The node may also send on its own schedule while
    //    it services a packet, so expectations are relative to what it sent.
    let hist = schema.info.history as u32;
    t.check(
        "history is capped by what one frame can carry",
        hist == 3,
        &format!("advertised {hist}, a 192-byte frame fits 3"),
    );

    let r1 = frame.newest().record;
    let mut acked = control.clone();
    acked.ack_record = r1;
    board.send_control(&expected, &ctrl_layout, &acked)?;
    let f2 = board.fetch_telemetry(&telem_layout)?;
    let r2 = f2.newest().record;
    t.check(
        "an up-to-date ack gets only what is new",
        f2.records.len() as u32 == (r2 - r1).min(hist),
        &format!(
            "acked {r1}, newest {r2}, got {} record(s)",
            f2.records.len()
        ),
    );

    acked.ack_record = r2.saturating_sub(10).max(1);
    board.send_control(&expected, &ctrl_layout, &acked)?;
    let f3 = board.fetch_telemetry(&telem_layout)?;
    let numbers: Vec<u32> = f3.records.iter().map(|r| r.record).collect();
    t.check(
        "a lagging ack is backfilled with the full history",
        f3.records.len() as u32 == hist,
        &format!("acked {}, got {numbers:?}", acked.ack_record),
    );
    t.check(
        "backfilled records keep their original node time",
        f3.records
            .windows(2)
            .all(|w| w[0].node_time_us.wrapping_sub(w[1].node_time_us) as i32 > 0),
        &format!(
            "{:?}",
            f3.records
                .iter()
                .map(|r| r.node_time_us)
                .collect::<Vec<_>>()
        ),
    );
    t.check(
        "backfilled records carry the right values",
        f3.records.iter().all(|r| r.values == *v),
        "a resent record differs from the live one",
    );

    acked.ack_record = f3.newest().record + 1000;
    board.send_control(&expected, &ctrl_layout, &acked)?;
    let f4 = board.fetch_telemetry(&telem_layout)?;
    t.check(
        "an ack from the future asks for nothing but the newest",
        f4.records.len() == 1,
        &format!("{} record(s)", f4.records.len()),
    );

    // 6. Rate: the station's request governs the telemetry rate, and 0 hands
    //    it back to the sketch.
    acked.ack_record = f4.newest().record;
    acked.rate_hz = 25;
    board.send_control(&expected, &ctrl_layout, &acked)?;
    let f5 = board.fetch_telemetry(&telem_layout)?;
    t.check(
        "a requested rate is adopted",
        f5.rate_hz == 25,
        &format!("node reports {} Hz", f5.rate_hz),
    );
    acked.rate_hz = 0;
    board.send_control(&expected, &ctrl_layout, &acked)?;
    let f6 = board.fetch_telemetry(&telem_layout)?;
    t.check(
        "rate 0 falls back to the sketch's own",
        f6.rate_hz == 50,
        &format!("node reports {} Hz", f6.rate_hz),
    );

    // 7. One station at a time. A second station may look, but not drive.
    let mut mine = control.clone();
    mine.ack_record = f6.newest().record;
    mine.values.i32s[0] = 4242;
    board.send_control(&expected, &ctrl_layout, &mine)?;

    // An intruder sends something unmistakable from another address.
    let mut theirs = mine.clone();
    theirs.values.i32s[0] = 9999;
    theirs.values.bools[0] = false;
    board.set_from("192.168.0.200", 5005)?;
    let replies = board.send_control_collect(&expected, &ctrl_layout, &theirs)?;
    let vals = board.dump()?;
    t.check(
        "a second station's control is refused",
        vals.get("target_ticks") == Some(&"4242".into()),
        &format!(
            "target_ticks={:?}, should still be the owner's 4242",
            vals.get("target_ticks")
        ),
    );
    let refusal = replies
        .iter()
        .filter_map(|p| Header::decode(p).ok())
        .find(|(h, _)| h.msg == MsgType::Telem)
        .and_then(|(_, b)| TelemFrame::decode(&telem_layout, b).ok());
    t.check(
        "the intruder is told, rather than ignored in silence",
        refusal
            .as_ref()
            .is_some_and(|f| f.flags & microrio_proto::flag::STATION_CONFLICT != 0),
        &match &refusal {
            Some(f) => format!("flags 0x{:02x}", f.flags),
            None => "no answer at all".into(),
        },
    );

    // The owner is told too: being fought over is the operator's business.
    board.set_from("192.168.0.100", 5006)?;
    board.send_control(&expected, &ctrl_layout, &mine)?;
    let owner_frame = board.fetch_telemetry(&telem_layout)?;
    t.check(
        "the owner's telemetry carries the conflict flag",
        owner_frame.flags & microrio_proto::flag::STATION_CONFLICT != 0,
        &format!("flags 0x{:02x}", owner_frame.flags),
    );

    // A schema request from anywhere is answered: looking is always allowed.
    board.set_from("192.168.0.200", 5005)?;
    let seen = board.fetch_schema()?;
    t.check(
        "a second station may still read the schema",
        seen.hash() == expected.hash(),
        "the node refused to describe itself to a second station",
    );

    // And once the owner falls silent for longer than the watchdog, the node
    // is free: a station that has crashed must not lock a board out forever.
    std::thread::sleep(Duration::from_millis(400));
    let mut takeover = theirs.clone();
    takeover.values.i32s[0] = 7;
    board.send_control(&expected, &ctrl_layout, &takeover)?;
    let vals = board.dump()?;
    t.check(
        "a node left silent can be taken over",
        vals.get("target_ticks") == Some(&"7".into()),
        &fmt(&vals, "target_ticks"),
    );

    // A watcher: marked view-only. The node must take nothing from it and,
    // just as importantly, send it nothing — replying is what costs a board
    // its timing, so watchers are fed by the station that owns the node.
    board.set_from("192.168.0.201", 5005)?;
    let mut watcher = mine.clone();
    watcher.flags = microrio_proto::ctrl_flag::VIEW_ONLY;
    watcher.host_time_us = 0xAABB_CCDD;
    watcher.values.i32s[0] = 1234567;
    let replies = board.send_control_collect(&expected, &ctrl_layout, &watcher)?;
    let vals = board.dump()?;
    // 7 is what the owner last set, two checks above.
    t.check(
        "a watcher's values are never applied",
        vals.get("target_ticks") == Some(&"7".into()),
        &fmt(&vals, "target_ticks"),
    );
    let answered = replies
        .iter()
        .filter_map(|p| Header::decode(p).ok())
        .filter_map(|(_, b)| TelemFrame::decode(&telem_layout, b).ok())
        .any(|f| f.echo_host_time_us == 0xAABB_CCDD || f.echo_host_time_us == 0);
    t.check(
        "a watcher is not answered at all, which is what keeps the board quick",
        !answered,
        "the node sent something back to a station that was only watching",
    );

    // The one thing it is listened to for.
    let mut stop = watcher.clone();
    stop.state = State::Estop;
    board.send_control_collect(&expected, &ctrl_layout, &stop)?;
    board.set_from("192.168.0.100", 5006)?;
    board.send_control(&expected, &ctrl_layout, &mine)?;
    let vals = board.dump()?;
    t.check(
        "a watcher may stop the machine, and the owner cannot override it",
        vals.get("__state") == Some(&"2".into()),
        &format!("state={:?} (2 = ESTOP)", vals.get("__state")),
    );
    println!("\n   (waiting out the watcher's 2 s estop)");
    std::thread::sleep(Duration::from_millis(2100));
    board.send_control(&expected, &ctrl_layout, &mine)?;
    let vals = board.dump()?;
    t.check(
        "and letting go of it hands the machine back",
        vals.get("__state") == Some(&"1".into()),
        &format!("state={:?} (1 = ENABLED)", vals.get("__state")),
    );

    // A station that stops answering must not take the board down with it.
    // On a real shield a send to an address that has gone quiet blocks for as
    // long as the chip takes to give up, every tick; the node drops its own
    // rate instead of stalling.
    board.stall(100)?;
    let before = Instant::now();
    for _ in 0..3 {
        board.force_telemetry()?;
    }
    board.stall(0)?;
    let slow_ticks = before.elapsed();
    let recovered = board.force_telemetry()?;
    t.check(
        "an expensive station does not stop the node answering",
        !recovered.is_empty(),
        "the node stopped sending after some slow sends",
    );
    t.check(
        "and the cost is bounded rather than paid for ever",
        slow_ticks < Duration::from_secs(2),
        &format!("three slow ticks took {slow_ticks:?}"),
    );

    // Ownership is symmetric: the first station gets it back the same way,
    // by being the one talking after the other has gone quiet.
    std::thread::sleep(Duration::from_millis(400));
    board.set_from("192.168.0.100", 5006)?;
    board.send_control(&expected, &ctrl_layout, &mine)?;
    let vals = board.dump()?;
    t.check(
        "and handed back again once that one goes quiet",
        vals.get("target_ticks") == Some(&"4242".into()),
        &fmt(&vals, "target_ticks"),
    );

    // 8. Watchdog. No control frames for longer than the timeout must fault the
    //    node on its own, with no help from the station.
    println!("\n   (waiting out the 200 ms watchdog)");
    std::thread::sleep(Duration::from_millis(600));
    let vals = board.dump()?;
    t.check(
        "watchdog faults the node when control stops",
        vals.get("__state") == Some(&"3".into()),
        &format!("state={:?} (3 = FAULT)", vals.get("__state")),
    );
    t.check(
        "link reports down after the watchdog trips",
        vals.get("__linked") == Some(&"0".into()),
        &format!("linked={:?}", vals.get("__linked")),
    );
    t.check(
        "a bool failstate takes over when the watchdog trips",
        vals.get("led") == Some(&"0".into()),
        &fmt(&vals, "led"),
    );
    t.check(
        "an f32 failstate takes over when the watchdog trips",
        near(&vals, "drive_x", -0.5),
        &fmt(&vals, "drive_x"),
    );
    t.check(
        "a control with no failstate keeps its last value",
        near(&vals, "drive_y", -1.5),
        &fmt(&vals, "drive_y"),
    );

    // 9. A frame carrying the wrong schema hash must be discarded, not guessed at.
    let mut bad = control.clone();
    bad.values.i32s[0] = 9999;
    board.send_control_with_hash(&ctrl_layout, &bad, expected.hash() ^ 0xFFFF_FFFF)?;
    let vals = board.dump()?;
    t.check(
        "a wrong-schema frame is ignored",
        vals.get("target_ticks") == Some(&"4242".into()),
        &format!(
            "target_ticks={:?}, should still be the old 4242",
            vals.get("target_ticks")
        ),
    );

    println!();
    t.report()
}

fn fmt(v: &std::collections::HashMap<String, String>, k: &str) -> String {
    format!("{k}={:?}", v.get(k))
}

fn near(v: &std::collections::HashMap<String, String>, k: &str, want: f32) -> bool {
    v.get(k)
        .and_then(|s| s.trim().parse::<f32>().ok())
        .map(|g| (g - want).abs() < 1e-4)
        .unwrap_or(false)
}

/// Mirrors `examples/Conformance/Conformance.ino` exactly.
fn expected_schema() -> Schema {
    let mut channels = Vec::new();
    let mut add = |dir: Dir, ctype: ChanType, slot: u16, name: &str| {
        channels.push(ChannelDef {
            dir,
            ctype,
            slot,
            name: name.to_string(),
        });
    };
    add(Dir::Telemetry, ChanType::Bool, 0, "hb");
    add(Dir::Telemetry, ChanType::Bool, 1, "limit_top");
    add(Dir::Telemetry, ChanType::Bool, 2, "limit_bottom");
    add(Dir::Telemetry, ChanType::I32, 0, "a0");
    add(Dir::Telemetry, ChanType::I32, 1, "uptime_ms");
    add(Dir::Telemetry, ChanType::F32, 0, "battery_v");
    add(Dir::Telemetry, ChanType::F32, 1, "loop_pct");
    add(Dir::Telemetry, ChanType::Str, 0, "status");
    add(Dir::Control, ChanType::Bool, 0, "led");
    add(Dir::Control, ChanType::Bool, 1, "intake");
    add(Dir::Control, ChanType::I32, 0, "target_ticks");
    add(Dir::Control, ChanType::F32, 0, "drive_x");
    add(Dir::Control, ChanType::F32, 1, "drive_y");
    add(Dir::Control, ChanType::Str, 0, "mode");
    add(Dir::Control, ChanType::Cmd, 0, "zero_encoders");
    add(Dir::Control, ChanType::Cmd, 1, "reset_fault");

    Schema {
        info: NodeInfo {
            name: "conformance".into(),
            rate_hz: 50,
            max_frame: 192,
            total_channels: 16,
            history: 3,
        },
        channels,
    }
}

// --------------------------------------------------------------- the board

struct Board {
    reader: BufReader<Box<dyn serialport::SerialPort>>,
    writer: Box<dyn serialport::SerialPort>,
    last_chunk_count: u16,
}

impl Board {
    fn new(mut port: Box<dyn serialport::SerialPort>) -> Result<Self, String> {
        let writer = port.try_clone().map_err(|e| format!("clone port: {e}"))?;

        // Drop anything buffered from before the reset: a previous run's banner,
        // or the tail of an avrdude upload, would otherwise be mistaken for this
        // boot and we would start talking while the board is still in its
        // bootloader.
        port.clear(serialport::ClearBuffer::Input)
            .map_err(|e| format!("cannot flush the input buffer: {e}"))?;

        // Reset, in a way that is correct for both boards.
        //
        // On an ESP32's CH340 auto-reset circuit, EN is pulled low when
        // DTR=0 and RTS=1, and IO0 is pulled low when DTR=1 and RTS=0. Holding
        // RTS asserted throughout means IO0 is never pulled low, so the chip
        // can only ever come up in normal boot rather than download mode --
        // which is what a naive DTR pulse does, and why this looked like a dead
        // board.
        //
        // An AVR resets on the DTR edge regardless of RTS, so the same pulse
        // serves both.
        let _ = port.write_request_to_send(true);
        let _ = port.write_data_terminal_ready(false);
        std::thread::sleep(Duration::from_millis(150));
        let _ = port.write_data_terminal_ready(true);

        Ok(Board {
            reader: BufReader::new(port),
            writer,
            last_chunk_count: 0,
        })
    }

    fn line(&mut self) -> Result<String, String> {
        let mut s = String::new();
        let r = self.reader.read_line(&mut s);
        if std::env::var("MICRORIO_DEBUG").is_ok() {
            eprintln!(
                "      <- {:?} {:?}",
                r.as_ref().map_err(|e| e.kind()),
                s.trim()
            );
        }
        r.map_err(|e| format!("serial read: {e}"))?;
        Ok(s.trim().to_string())
    }

    fn send(&mut self, s: &str) -> Result<(), String> {
        if std::env::var("MICRORIO_DEBUG").is_ok() {
            eprintln!("      -> {s}");
        }
        self.writer
            .write_all(format!("{s}\n").as_bytes())
            .map_err(|e| format!("serial write: {e}"))?;
        self.writer.flush().map_err(|e| format!("flush: {e}"))
    }

    /// Wait for a complete, fresh `HASH` + `READY` pair.
    fn wait_hash(&mut self) -> Result<u32, String> {
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut hash = None;
        while Instant::now() < deadline {
            let l = match self.line() {
                Ok(l) => l,
                Err(_) => continue,
            };
            if let Some(h) = l.strip_prefix("HASH ") {
                // A second HASH means the board reset again; trust the latest.
                hash = u32::from_str_radix(h.trim(), 16).ok();
            } else if l == "READY" {
                if let Some(h) = hash {
                    return Ok(h);
                }
                // READY without a preceding HASH is a half-consumed banner.
                // Keep waiting rather than guessing.
            }
        }
        Err("timed out waiting for the board to boot (is Conformance flashed?)".into())
    }

    fn read_tx(&mut self) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            let l = match self.line() {
                Ok(l) => l,
                Err(_) => continue,
            };
            if let Some(h) = l.strip_prefix("TX ") {
                return hex_decode(h.trim());
            }
        }
        Err("no TX frame from the board".into())
    }

    fn wait_ok(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            if let Ok(l) = self.line() {
                if l == "OK" {
                    return Ok(());
                }
                if l == "ERR" {
                    return Err("board rejected the command".into());
                }
            }
        }
        Err("no OK from the board".into())
    }

    fn fetch_schema(&mut self) -> Result<Schema, String> {
        let mut asm = SchemaAssembler::new();
        let mut want = 0u16;

        for _ in 0..32 {
            let mut body = [0u8; 2];
            let n = encode_schema_req(want, &mut body).map_err(|e| e.to_string())?;
            let mut pkt = vec![0u8; HEADER_LEN];
            Header::new(MsgType::SchemaReq, NODE_ID, 0, 0)
                .encode(&mut pkt)
                .map_err(|e| e.to_string())?;
            pkt.extend_from_slice(&body[..n]);

            self.send(&format!("RX {}", hex_encode(&pkt)))?;
            let raw = self.read_tx()?;
            self.wait_ok()?;

            let (hdr, chunk_body) = Header::decode(&raw).map_err(|e| e.to_string())?;
            if hdr.msg != MsgType::Schema {
                return Err(format!("expected a schema chunk, got {:?}", hdr.msg));
            }
            let chunk = SchemaChunk::decode(chunk_body).map_err(|e| e.to_string())?;
            self.last_chunk_count = chunk.chunk_count;

            if let Some(schema) = asm.push(chunk) {
                return Ok(schema);
            }
            want = *asm
                .missing()
                .first()
                .ok_or("assembler reports nothing missing but produced no schema")?;
        }
        Err("schema never completed after 32 chunk requests".into())
    }

    /// Make every send cost this long in milliseconds, as an unresolvable
    /// address does on a real shield.
    fn stall(&mut self, ms: u32) -> Result<(), String> {
        self.send(&format!("STALL {ms}"))?;
        self.wait_ok()
    }

    /// Which station the next injected packets appear to come from.
    fn set_from(&mut self, ip: &str, port: u16) -> Result<(), String> {
        self.send(&format!("FROM {ip} {port}"))?;
        self.wait_ok()
    }

    /// Inject a control frame and collect whatever the node sends back, if
    /// anything, without insisting that it sends something.
    fn send_control_collect(
        &mut self,
        schema: &Schema,
        layout: &microrio_proto::schema::Layout,
        frame: &ControlFrame,
    ) -> Result<Vec<Vec<u8>>, String> {
        let mut pkt = vec![0u8; HEADER_LEN];
        Header::new(MsgType::Control, NODE_ID, schema.hash(), 0)
            .encode(&mut pkt)
            .map_err(|e| e.to_string())?;
        let mut body = vec![0u8; ControlFrame::body_len(layout)];
        let n = frame.encode(layout, &mut body).map_err(|e| e.to_string())?;
        pkt.extend_from_slice(&body[..n]);
        self.send(&format!("RX {}", hex_encode(&pkt)))?;

        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let Ok(l) = self.line() else { continue };
            if let Some(h) = l.strip_prefix("TX ") {
                out.push(hex_decode(h.trim())?);
            } else if l == "OK" {
                return Ok(out);
            }
        }
        Err("no OK after injecting a control frame".into())
    }

    fn send_control(
        &mut self,
        schema: &Schema,
        layout: &microrio_proto::schema::Layout,
        frame: &ControlFrame,
    ) -> Result<(), String> {
        self.send_control_with_hash(layout, frame, schema.hash())
    }

    fn send_control_with_hash(
        &mut self,
        layout: &microrio_proto::schema::Layout,
        frame: &ControlFrame,
        hash: u32,
    ) -> Result<(), String> {
        let mut pkt = vec![0u8; HEADER_LEN];
        Header::new(MsgType::Control, NODE_ID, hash, 0)
            .encode(&mut pkt)
            .map_err(|e| e.to_string())?;
        let mut body = vec![0u8; ControlFrame::body_len(layout)];
        let n = frame.encode(layout, &mut body).map_err(|e| e.to_string())?;
        pkt.extend_from_slice(&body[..n]);

        self.send(&format!("RX {}", hex_encode(&pkt)))?;
        self.wait_ok()
    }

    /// Force one telemetry tick and collect every packet it produced: the
    /// owner's, and any watcher's.
    fn force_telemetry(&mut self) -> Result<Vec<Vec<u8>>, String> {
        self.send("TELEM")?;
        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let Ok(l) = self.line() else { continue };
            if let Some(h) = l.strip_prefix("TX ") {
                out.push(hex_decode(h.trim())?);
            } else if l == "OK" {
                return Ok(out);
            }
        }
        Err("no OK after forcing telemetry".into())
    }

    fn fetch_telemetry(
        &mut self,
        layout: &microrio_proto::schema::Layout,
    ) -> Result<TelemFrame, String> {
        self.send("TELEM")?;
        let raw = self.read_tx()?;
        let _ = self.wait_ok();
        let (hdr, body) = Header::decode(&raw).map_err(|e| e.to_string())?;
        if hdr.msg != MsgType::Telem {
            return Err(format!("expected telemetry, got {:?}", hdr.msg));
        }
        TelemFrame::decode(layout, body).map_err(|e| format!("decoding telemetry: {e}"))
    }

    fn dump(&mut self) -> Result<std::collections::HashMap<String, String>, String> {
        self.send("DUMP")?;
        let mut map = std::collections::HashMap::new();
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            let l = match self.line() {
                Ok(l) => l,
                Err(_) => continue,
            };
            if let Some(kv) = l.strip_prefix("VAL ") {
                if let Some((k, v)) = kv.split_once('=') {
                    map.insert(k.trim().to_string(), v.trim().to_string());
                }
            } else if let Some(rest) = l.strip_prefix("STATE ") {
                let mut it = rest.split_whitespace();
                if let Some(s) = it.next() {
                    map.insert("__state".into(), s.to_string());
                }
                if let Some(s) = it.next() {
                    map.insert("__linked".into(), s.to_string());
                }
            } else if l == "OK" {
                return Ok(map);
            }
        }
        Err("DUMP never completed".into())
    }
}

// ------------------------------------------------------------- bookkeeping

#[derive(Default)]
struct Harness {
    passed: usize,
    failed: Vec<String>,
}

impl Harness {
    fn check(&mut self, what: &str, ok: bool, detail: &str) {
        if ok {
            self.passed += 1;
            println!("   ok    {what}");
        } else {
            self.failed.push(format!("{what} -- {detail}"));
            println!("   FAIL  {what}\n           {detail}");
        }
    }

    fn report(self) -> Result<(), String> {
        // The board is left holding the test sketch, which has no network at
        // all. Forgetting that reads exactly like a broken station: every node
        // dark, every page waiting for a schema that cannot come. So say it
        // both ways round, pass or fail.
        let restore = concat!(
            "This board now holds Conformance.ino, which does not touch the network.\n",
            "  Reflash its node sketch before using it with the station:\n",
            "    arduino-cli compile -b arduino:avr:mega --library arduino/MicroRIO \\\n",
            "        -u -p <port> arduino/MicroRIO/examples/SimNode\n",
            "    arduino/flash-wifi-node.sh <port>        # the ESP32",
        );
        if self.failed.is_empty() {
            println!("{} checks passed.", self.passed);
            println!("\n  {restore}");
            Ok(())
        } else {
            println!("{} passed, {} FAILED:", self.passed, self.failed.len());
            for f in &self.failed {
                println!("   - {f}");
            }
            println!("\n  {restore}");
            Err(format!("{} conformance checks failed", self.failed.len()))
        }
    }
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err(format!("odd-length hex: {s:?}"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("bad hex: {e}")))
        .collect()
}
