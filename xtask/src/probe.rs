//! Live link probe: handshake, control, and watchdog against a real node.
//!
//! Headless, so it works over ssh and leaves a clean pass/fail. Useful as a
//! field diagnostic when a node is not behaving and you want to know whether
//! the problem is the link, the schema, or the sketch.
//!
//! Run: `cargo xtask probe 192.168.0.50`

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use microrio_proto::frames::{ControlFrame, TelemFrame, Values};
use microrio_proto::records::Tracker;
use microrio_proto::schema::{encode_schema_req, Schema, SchemaAssembler, SchemaChunk};
use microrio_proto::{ChanType, Dir, Header, MsgType, State, HEADER_LEN};

fn target_addr(ip: &str) -> SocketAddr {
    format!("{ip}:5005").parse().expect("validated at entry")
}

pub fn run(ip: &str, node_id: u16) -> Result<(), String> {
    let target_ip = ip.to_string();
    let target: SocketAddr = format!("{ip}:5005")
        .parse()
        .map_err(|e| format!("bad address {ip}: {e}"))?;
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(400)))
        .map_err(|e| format!("timeout: {e}"))?;

    println!("== MicroRIO probe: node #{node_id} at {target} ==\n");
    let started = Instant::now();
    let mut seq = 0u16;
    let mut buf = [0u8; 2048];

    // 1. Handshake.
    let mut asm = SchemaAssembler::new();
    let mut want = 0u16;
    let mut schema: Option<Schema> = None;
    let deadline = Instant::now() + Duration::from_secs(6);

    while Instant::now() < deadline && schema.is_none() {
        let mut pkt = vec![0u8; HEADER_LEN];
        Header::new(MsgType::SchemaReq, node_id, 0, seq)
            .encode(&mut pkt)
            .map_err(|e| e.to_string())?;
        seq = seq.wrapping_add(1);
        let mut body = [0u8; 2];
        let n = encode_schema_req(want, &mut body).map_err(|e| e.to_string())?;
        pkt.extend_from_slice(&body[..n]);
        sock.send_to(&pkt, target)
            .map_err(|e| format!("send: {e}"))?;

        let Ok((len, _)) = sock.recv_from(&mut buf) else {
            continue;
        };
        let Ok((hdr, chunk_body)) = Header::decode(&buf[..len]) else {
            continue;
        };
        if hdr.msg != MsgType::Schema {
            continue;
        }
        let Ok(chunk) = SchemaChunk::decode(chunk_body) else {
            continue;
        };
        if let Some(s) = asm.push(chunk) {
            schema = Some(s);
        } else {
            want = asm.missing().first().copied().unwrap_or(0);
        }
    }

    let schema = schema.ok_or("node never completed its schema handshake")?;
    println!(
        "   schema  {} channels, hash 0x{:08x}, node \"{}\", {} Hz, {}-byte frames, history {}",
        schema.channels.len(),
        schema.hash(),
        schema.info.name,
        schema.info.rate_hz,
        schema.info.max_frame,
        schema.info.history
    );
    for c in &schema.channels {
        println!(
            "            {:<10} {:<5} slot {:<2} {}",
            format!("{:?}", c.dir).to_lowercase(),
            format!("{:?}", c.ctype).to_lowercase(),
            c.slot,
            c.name
        );
    }
    println!();

    let tl = schema.layout(Dir::Telemetry);
    let cl = schema.layout(Dir::Control);
    let mut fails = 0;

    // 2. Drive the node and read it back.
    let mut check = |ok: bool, what: &str, detail: String| {
        if ok {
            println!("   ok    {what}");
        } else {
            println!("   FAIL  {what}\n           {detail}");
            fails += 1;
        }
    };

    // Hold ENABLED with a known control value, then look at what comes back.
    let led_slot = schema
        .channels
        .iter()
        .find(|c| c.dir == Dir::Control && c.ctype == ChanType::Bool)
        .map(|c| c.slot as usize);
    let f32_slot = schema
        .channels
        .iter()
        .find(|c| c.dir == Dir::Control && c.ctype == ChanType::F32)
        .map(|c| c.slot as usize);

    let mut values = Values::zeroed(&cl);
    if let Some(i) = led_slot {
        values.bools[i] = true;
    }
    if let Some(i) = f32_slot {
        values.f32s[i] = 0.5;
    }

    let mut last: Option<TelemFrame> = None;
    let mut rx = 0usize;
    let mut sent = 0usize;
    let mut rtt_sum = 0u64;
    let mut rtt_n = 0usize;
    let mut stale_echo = 0usize;
    let mut last_seq: Option<u16> = None;
    let mut seq_expected: u32 = 0;
    let mut seq_got: u32 = 0;
    // Numbered records. Every fourth frame that arrives is thrown away here,
    // as if the network had dropped it, so the node's resending is exercised
    // for real rather than waiting for a lossy LAN to show up.
    let mut tracker = Tracker::new(schema.info.history);
    let mut dropped_on_purpose = 0usize;
    let run_until = Instant::now() + Duration::from_millis(1500);

    // Short timeout from here on: the handshake could afford to block, but the
    // control loop must keep its own cadence rather than waiting on a reply.
    sock.set_read_timeout(Some(Duration::from_millis(5)))
        .map_err(|e| format!("timeout: {e}"))?;

    while Instant::now() < run_until {
        let host_time = started.elapsed().as_micros() as u32;
        let frame = ControlFrame {
            host_time_us: host_time,
            state: State::Enabled,
            flags: 0,
            rate_hz: 0,
            ack_record: tracker.ack(),
            values: values.clone(),
        };
        let mut pkt = vec![0u8; HEADER_LEN];
        Header::new(MsgType::Control, node_id, schema.hash(), seq)
            .encode(&mut pkt)
            .map_err(|e| e.to_string())?;
        seq = seq.wrapping_add(1);
        let mut body = vec![0u8; ControlFrame::body_len(&cl)];
        let n = frame.encode(&cl, &mut body).map_err(|e| e.to_string())?;
        pkt.extend_from_slice(&body[..n]);
        let _ = sock.send_to(&pkt, target);
        sent += 1;

        // Take everything already queued, then move on.
        while let Ok((len, _)) = sock.recv_from(&mut buf) {
            if let Ok((hdr, tbody)) = Header::decode(&buf[..len]) {
                if hdr.msg == MsgType::Telem && hdr.schema_hash == schema.hash() {
                    if let Ok(f) = TelemFrame::decode(&tl, tbody) {
                        // The probe is a station like any other, and a node
                        // answers to one at a time. Say so plainly rather
                        // than reporting a wall of loss.
                        if f.flags & microrio_proto::flag::STATION_CONFLICT != 0
                            && f.rtt_us_checked(started.elapsed().as_micros() as u32, 1_000_000)
                                .is_none()
                        {
                            return Err(format!(
                                "node {node_id} belongs to another station right now, and \
                                 refused this one.\n   Close the driver station (or any \
                                 other probe) and run this again. A node accepts control \
                                 from one station at a time; it frees up a watchdog period \
                                 after that station stops talking."
                            ));
                        }
                        match f.rtt_us_checked(started.elapsed().as_micros() as u32, 1_000_000) {
                            Some(d) => {
                                rtt_sum += d as u64;
                                rtt_n += 1;
                            }
                            None => stale_echo += 1,
                        }
                        if let Some(prev) = last_seq {
                            let gap = hdr.seq.wrapping_sub(prev) as u32;
                            if gap > 0 && gap < 1000 {
                                seq_expected += gap;
                                seq_got += 1;
                            }
                        }
                        last_seq = Some(hdr.seq);
                        rx += 1;
                        if rx % 4 == 0 && rx > 4 {
                            dropped_on_purpose += 1;
                        } else {
                            tracker.offer(&f);
                        }
                        last = Some(f);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let f = last.ok_or("no telemetry came back while driving the node")?;
    check(
        rx > 20,
        "telemetry arrives steadily",
        format!("{rx} frames in 1.5 s"),
    );
    check(
        rx as f64 / sent as f64 > 0.8,
        "little or no packet loss",
        format!("{rx} received of {sent} sent"),
    );
    check(
        f.state == State::Enabled,
        "node obeys ENABLED",
        format!("node reports {:?}", f.state),
    );
    check(
        f.flags & microrio_proto::flag::WATCHDOG_TRIPPED == 0,
        "node watchdog is satisfied while we are talking",
        format!("flags 0x{:02x}", f.flags),
    );
    if rtt_n > 0 {
        println!(
            "            mean round trip {:.2} ms over {rtt_n} frames{}",
            rtt_sum as f64 / rtt_n as f64 / 1000.0,
            if stale_echo > 0 {
                format!("  ({stale_echo} skipped: echo not ours yet)")
            } else {
                String::new()
            }
        );
    } else {
        println!("            round trip unknown: the node never echoed our clock");
    }
    check(
        rtt_n > 0,
        "round trip is measurable",
        format!("{stale_echo} frames carried an echo that was not ours"),
    );
    // Gaps in the node's sequence numbers say how much of what it sent never
    // arrived. On a quiet LAN the usual cause is a second station.
    let delivered = (seq_got * 100).checked_div(seq_expected).unwrap_or(100);
    check(
        delivered >= 90,
        "every frame the node sent reached us",
        format!(
            "only {delivered}% arrived ({seq_got} of {seq_expected} by sequence number) - \
             most often another copy of the station talking to the same board, since a \
             node follows whoever spoke last"
        ),
    );

    if schema.info.history > 1 {
        check(
            tracker.lost == 0 && tracker.recovered > 0,
            "dropped frames are backfilled: no record lost",
            format!(
                "{dropped_on_purpose} frames dropped on purpose; {} records recovered, {} lost",
                tracker.recovered, tracker.lost
            ),
        );
        println!(
            "            {dropped_on_purpose} frames dropped on purpose, {} records recovered, 0 lost",
            tracker.recovered
        );
    } else {
        check(
            tracker.lost > 0,
            "a node with no history reports dropped frames as lost",
            format!(
                "{} lost of {}",
                tracker.lost,
                tracker.received + tracker.lost
            ),
        );
    }

    // 2b. Rate. Ask for 25 Hz and count what comes back.
    let ask_hz = 25u16;
    let mut at_rate = 0usize;
    let mut reported = 0u16;
    let rate_until = Instant::now() + Duration::from_millis(1200);
    let count_from = Instant::now() + Duration::from_millis(200);
    while Instant::now() < rate_until {
        let frame = ControlFrame {
            host_time_us: started.elapsed().as_micros() as u32,
            state: State::Enabled,
            flags: 0,
            rate_hz: ask_hz,
            ack_record: tracker.ack(),
            values: values.clone(),
        };
        let mut pkt = vec![0u8; HEADER_LEN];
        Header::new(MsgType::Control, node_id, schema.hash(), seq)
            .encode(&mut pkt)
            .map_err(|e| e.to_string())?;
        seq = seq.wrapping_add(1);
        let mut body = vec![0u8; ControlFrame::body_len(&cl)];
        let n = frame.encode(&cl, &mut body).map_err(|e| e.to_string())?;
        pkt.extend_from_slice(&body[..n]);
        let _ = sock.send_to(&pkt, target);
        while let Ok((len, _)) = sock.recv_from(&mut buf) {
            if let Ok((hdr, tbody)) = Header::decode(&buf[..len]) {
                if hdr.msg == MsgType::Telem && hdr.schema_hash == schema.hash() {
                    if let Ok(f) = TelemFrame::decode(&tl, tbody) {
                        tracker.offer(&f);
                        reported = f.rate_hz;
                        if Instant::now() > count_from {
                            at_rate += 1;
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    check(
        reported == ask_hz && (20..=30).contains(&at_rate),
        "the requested telemetry rate is obeyed",
        format!("asked {ask_hz} Hz; node reports {reported} Hz, {at_rate} frames in 1 s"),
    );

    // 3. Stop talking. The node must fault itself.
    println!("\n   (going quiet for 700 ms to trip the node's watchdog)");
    std::thread::sleep(Duration::from_millis(700));

    // Discard the backlog from before the watchdog tripped. Bounded by time,
    // not by the socket going empty: the node keeps transmitting while faulted,
    // so "drain until empty" would never return.
    let drain_until = Instant::now() + Duration::from_millis(250);
    while Instant::now() < drain_until {
        let _ = sock.recv_from(&mut buf);
    }

    let mut faulted = None;
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline && faulted.is_none() {
        if let Ok((len, _)) = sock.recv_from(&mut buf) {
            if let Ok((hdr, tbody)) = Header::decode(&buf[..len]) {
                if hdr.msg == MsgType::Telem {
                    if let Ok(f) = TelemFrame::decode(&tl, tbody) {
                        faulted = Some(f);
                    }
                }
            }
        }
    }

    match faulted {
        Some(f) => {
            check(
                f.state == State::Fault,
                "node faults itself when control stops",
                format!("node reports {:?}", f.state),
            );
            check(
                f.flags & microrio_proto::flag::WATCHDOG_TRIPPED != 0,
                "node raises the watchdog flag",
                format!("flags 0x{:02x}", f.flags),
            );
        }
        None => check(
            false,
            "node keeps reporting after control stops",
            "no telemetry at all; it stopped talking instead of faulting".into(),
        ),
    }

    // 7. Clock sync, when the node declares the channels for it. Deliberately
    //    skew the node's clock, confirm it moved, then put it back.
    let has = |name: &str, dir: Dir| {
        schema
            .channels
            .iter()
            .any(|c| c.dir == dir && c.name == name)
    };
    if has("unix_time", Dir::Telemetry)
        && has("set_unix", Dir::Control)
        && has("sync_time", Dir::Control)
    {
        println!("\n   (testing clock sync)");
        let unix_slot = schema
            .channels
            .iter()
            .find(|c| c.dir == Dir::Telemetry && c.name == "unix_time")
            .map(|c| c.slot as usize)
            .unwrap();
        let set_slot = schema
            .channels
            .iter()
            .find(|c| c.dir == Dir::Control && c.name == "set_unix")
            .map(|c| c.slot as usize)
            .unwrap();
        let sync_slot = schema
            .channels
            .iter()
            .find(|c| c.dir == Dir::Control && c.name == "sync_time")
            .map(|c| c.slot as usize)
            .unwrap();

        let now_unix = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i32)
                .unwrap_or(0)
        };

        let mut counter = 0u16;
        let mut push_time = |target: i32, counter: &mut u16, seq: &mut u16| -> Option<i32> {
            let mut v = Values::zeroed(&cl);
            if let Some(s) = v.i32s.get_mut(set_slot) {
                *s = target;
            }
            *counter = counter.wrapping_add(1);
            if let Some(s) = v.cmds.get_mut(sync_slot) {
                *s = *counter;
            }
            let frame = ControlFrame {
                rate_hz: 0,
                ack_record: 0,
                host_time_us: started.elapsed().as_micros() as u32,
                state: State::Disabled,
                flags: 0,
                values: v,
            };
            let mut last_seen = None;
            for _ in 0..40 {
                let mut pkt = vec![0u8; HEADER_LEN];
                Header::new(MsgType::Control, node_id, schema.hash(), *seq)
                    .encode(&mut pkt)
                    .ok()?;
                *seq = seq.wrapping_add(1);
                let mut body = vec![0u8; ControlFrame::body_len(&cl)];
                let n = frame.encode(&cl, &mut body).ok()?;
                pkt.extend_from_slice(&body[..n]);
                let _ = sock.send_to(&pkt, target_addr(&target_ip));
                while let Ok((len, _)) = sock.recv_from(&mut buf) {
                    if let Ok((hdr, tb)) = Header::decode(&buf[..len]) {
                        if hdr.msg == MsgType::Telem {
                            if let Ok(f) = TelemFrame::decode(&tl, tb) {
                                last_seen = f.newest().values.i32s.get(unix_slot).copied();
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            last_seen
        };

        let skewed_target = now_unix() - 3600;
        let after_skew = push_time(skewed_target, &mut counter, &mut seq).unwrap_or(0);
        check(
            (after_skew - skewed_target).abs() <= 2,
            "node accepts a clock set over the wire",
            format!("asked for {skewed_target}, node reads {after_skew}"),
        );

        let real = now_unix();
        let after_fix = push_time(real, &mut counter, &mut seq).unwrap_or(0);
        check(
            (after_fix - now_unix()).abs() <= 2,
            "clock sync puts the node back on station time",
            format!("node reads {after_fix}, station {}", now_unix()),
        );
    }

    println!();
    if fails == 0 {
        println!("all probe checks passed.");
        Ok(())
    } else {
        Err(format!("{fails} probe checks failed"))
    }
}
