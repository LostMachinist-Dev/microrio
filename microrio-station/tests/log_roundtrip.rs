//! Logger and CSV export, end to end, with no hardware.
//!
//! The network path is tested against real boards; this covers the part that
//! only touches the disk.

use std::path::PathBuf;

use microrio_proto::frames::Values;
use microrio_proto::schema::{ChannelDef, NodeInfo, Schema};
use microrio_proto::{ChanType, Dir};

#[path = "../src/clock.rs"]
mod clock;

#[path = "../src/logger.rs"]
mod logger;

fn schema() -> Schema {
    let ch = |dir, ctype, slot, name: &str| ChannelDef {
        dir,
        ctype,
        slot,
        name: name.to_string(),
    };
    Schema {
        info: NodeInfo {
            name: "bench".into(),
            rate_hz: 50,
            max_frame: 512,
            total_channels: 6,
            history: 1,
        },
        channels: vec![
            ch(Dir::Telemetry, ChanType::Bool, 0, "hb"),
            ch(Dir::Telemetry, ChanType::I32, 0, "a0"),
            ch(Dir::Telemetry, ChanType::F32, 0, "battery_v"),
            ch(Dir::Telemetry, ChanType::Str, 0, "status"),
            ch(Dir::Control, ChanType::Bool, 0, "led"),
            ch(Dir::Control, ChanType::F32, 0, "drive_x"),
        ],
    }
}

/// The value payload alone, as the station stores it: no time, state or flags.
fn payload(layout: &microrio_proto::schema::Layout, values: Values) -> Vec<u8> {
    values.encode_payload(layout).unwrap()
}

#[test]
fn log_writes_and_exports_both_directions() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("logtest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let s = schema();
    let tl = s.layout(Dir::Telemetry);
    let cl = s.layout(Dir::Control);

    let log_path;
    {
        let mut lg = logger::Logger::create(&dir).unwrap();
        log_path = lg.path().to_path_buf();

        lg.write(&logger::Record::Schema {
            node_id: 3,
            schema: s.clone(),
        })
        .unwrap();

        for i in 0..4u32 {
            lg.write(&logger::Record::Sample {
                t_us: i as u64 * 20_000,
                node_id: 3,
                dir: Dir::Control,
                record: 0,
                payload: payload(
                    &cl,
                    Values {
                        bools: vec![i % 2 == 0],
                        f32s: vec![i as f32 * 0.25],
                        ..Default::default()
                    },
                ),
            })
            .unwrap();

            lg.write(&logger::Record::Sample {
                t_us: i as u64 * 20_000,
                node_id: 3,
                dir: Dir::Telemetry,
                record: 0,
                payload: payload(
                    &tl,
                    Values {
                        bools: vec![i % 2 == 1],
                        i32s: vec![100 + i as i32],
                        f32s: vec![12.0 + i as f32],
                        strs: vec![format!("row{i}")],
                        ..Default::default()
                    },
                ),
            })
            .unwrap();
        }
        lg.flush().unwrap();
    }

    let out = dir.join("csv");
    let files = logger::export_csv(&log_path, &out).unwrap();
    assert_eq!(files.len(), 1, "one node, one CSV");

    let csv = std::fs::read_to_string(&files[0]).unwrap();
    let lines: Vec<&str> = csv.lines().collect();

    assert_eq!(
        lines[0], "time_us,hb,a0,battery_v,status,ctrl_led,ctrl_drive_x",
        "header names every channel, control columns prefixed"
    );
    assert_eq!(lines.len(), 5, "header plus one row per telemetry sample");

    // Row for i=0: telemetry, with the control values sent on the same tick.
    assert_eq!(lines[1], "0,0,100,12,row0,1,0");
    // Row for i=3: hb true, a0 103, battery 15, led false, drive_x 0.75.
    assert_eq!(lines[4], "60000,1,103,15,row3,0,0.75");
}

#[test]
fn export_rejects_a_file_that_is_not_a_log() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("badlog");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("nonsense.bin");
    std::fs::write(&f, b"this is not a MicroRIO log at all").unwrap();
    assert!(logger::export_csv(&f, &dir.join("out")).is_err());
}

/// Start/stop writes only the marked stretch, stamped in the station's zone.
///
/// The background log runs the whole session; what the operator picked is a
/// window inside it. Getting this wrong by one row either way would hand
/// someone a CSV that disagrees with the clock they watched it on.
#[test]
fn window_export_carries_local_time_and_only_the_window() {
    use microrio_proto::schema::Layout;

    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("windowlog");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let s = schema();
    let telem: Layout = s.layout(Dir::Telemetry);
    let ctrl: Layout = s.layout(Dir::Control);

    // 2026-10-26 00:00:00 UTC, so the row stamps are checkable by eye.
    let epoch_us: i64 = 1_792_972_800_000_000;

    let log_path;
    {
        let mut lg = logger::Logger::create(&dir).unwrap();
        log_path = lg.path().to_path_buf();
        lg.write(&logger::Record::Meta {
            started_unix_us: epoch_us,
        })
        .unwrap();
        lg.write(&logger::Record::Schema {
            node_id: 1,
            schema: s.clone(),
        })
        .unwrap();

        // Ten telemetry frames, 20 ms apart, with a control frame each tick.
        for i in 0..10u64 {
            let t_us = i * 20_000;
            lg.write(&logger::Record::Sample {
                t_us,
                node_id: 1,
                dir: Dir::Control,
                record: 0,
                payload: payload(
                    &ctrl,
                    Values {
                        bools: vec![true],
                        f32s: vec![i as f32],
                        ..Default::default()
                    },
                ),
            })
            .unwrap();
            lg.write(&logger::Record::Sample {
                t_us,
                node_id: 1,
                dir: Dir::Telemetry,
                record: 0,
                payload: payload(
                    &telem,
                    Values {
                        bools: vec![true],
                        i32s: vec![i as i32],
                        f32s: vec![0.0],
                        strs: vec![String::new()],
                        ..Default::default()
                    },
                ),
            })
            .unwrap();
        }
        lg.flush().unwrap();
    }

    // Mark frames 3 through 6 inclusive.
    let out = dir.join("window.csv");
    let opts = logger::ExportOpts {
        zone: Some(clock::Zone::parse("UTC")),
        from_us: Some(3 * 20_000),
        to_us: Some(6 * 20_000),
    };
    let files = logger::export_window(&log_path, &out, &opts).unwrap();
    assert_eq!(files, vec![out.clone()], "the chosen path is used verbatim");

    let csv = std::fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = csv.lines().collect();

    assert_eq!(
        lines[0], "time_us,time_local,hb,a0,battery_v,status,ctrl_led,ctrl_drive_x",
        "the wall-clock column sits next to the elapsed one"
    );
    assert_eq!(lines.len(), 5, "header plus frames 3,4,5,6 — not all ten");

    // Midnight UTC plus 60 ms.
    assert!(
        lines[1].starts_with("60000,2026-10-26 00:00:00.060000,"),
        "first row is frame 3, stamped in the chosen zone: {}",
        lines[1]
    );
    assert!(
        lines[4].starts_with("120000,2026-10-26 00:00:00.120000,"),
        "last row is frame 6: {}",
        lines[4]
    );

    // Control carried forward: frame 3's row holds the control sent at t=60ms.
    assert!(
        lines[1].ends_with(",1,3"),
        "control values travel with the row"
    );
}

/// A log from before the wall-clock marker existed still exports, just
/// without the column — old sessions must not become unreadable.
#[test]
fn a_log_without_meta_exports_without_the_local_column() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("nometa");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let s = schema();
    let telem = s.layout(Dir::Telemetry);
    let log_path;
    {
        let mut lg = logger::Logger::create(&dir).unwrap();
        log_path = lg.path().to_path_buf();
        lg.write(&logger::Record::Schema {
            node_id: 1,
            schema: s.clone(),
        })
        .unwrap();
        lg.write(&logger::Record::Sample {
            t_us: 0,
            node_id: 1,
            dir: Dir::Telemetry,
            record: 0,
            payload: payload(
                &telem,
                Values {
                    bools: vec![false],
                    i32s: vec![5],
                    f32s: vec![1.0],
                    strs: vec!["x".into()],
                    ..Default::default()
                },
            ),
        })
        .unwrap();
        lg.flush().unwrap();
    }

    let out = dir.join("old.csv");
    let opts = logger::ExportOpts {
        zone: Some(clock::Zone::parse("UTC")),
        ..Default::default()
    };
    logger::export_window(&log_path, &out, &opts).unwrap();
    let csv = std::fs::read_to_string(&out).unwrap();
    assert_eq!(
        csv.lines().next().unwrap(),
        "time_us,hb,a0,battery_v,status,ctrl_led,ctrl_drive_x",
        "no epoch in the log means no wall-clock column, rather than a wrong one"
    );
}

#[test]
fn a_resent_record_is_exported_in_the_order_it_was_taken() {
    // Record 2 was dropped and resent, so it reaches the log after record 3,
    // stamped with its original time. The CSV must still read 1, 2, 3, 4.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("backfill");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let s = schema();
    let tl = s.layout(Dir::Telemetry);
    let log_path;
    {
        let mut lg = logger::Logger::create(&dir).unwrap();
        log_path = lg.path().to_path_buf();
        lg.write(&logger::Record::Schema {
            node_id: 1,
            schema: s.clone(),
        })
        .unwrap();
        for rec in [1u32, 3, 2, 4] {
            lg.write(&logger::Record::Sample {
                t_us: rec as u64 * 20_000,
                node_id: 1,
                dir: Dir::Telemetry,
                record: rec,
                payload: payload(
                    &tl,
                    Values {
                        bools: vec![false],
                        i32s: vec![rec as i32],
                        f32s: vec![0.0],
                        strs: vec![String::new()],
                        ..Default::default()
                    },
                ),
            })
            .unwrap();
        }
        lg.flush().unwrap();
    }

    let files = logger::export_csv(&log_path, &dir.join("csv")).unwrap();
    let csv = std::fs::read_to_string(&files[0]).unwrap();
    let a0: Vec<&str> = csv
        .lines()
        .skip(1)
        .map(|l| l.split(',').nth(2).unwrap())
        .collect();
    assert_eq!(a0, vec!["1", "2", "3", "4"]);
}

#[test]
fn control_that_stopped_is_not_painted_across_the_rest_of_the_session() {
    // What a station that drops to watching produces: it commanded briefly,
    // then only listened. Its last command must not follow every later row.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("stalectrl");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let s = schema();
    let (tl, cl) = (s.layout(Dir::Telemetry), s.layout(Dir::Control));
    let log_path;
    {
        let mut lg = logger::Logger::create(&dir).unwrap();
        log_path = lg.path().to_path_buf();
        lg.write(&logger::Record::Schema {
            node_id: 1,
            schema: s.clone(),
        })
        .unwrap();
        lg.write(&logger::Record::Sample {
            t_us: 0,
            node_id: 1,
            dir: Dir::Control,
            record: 0,
            payload: payload(
                &cl,
                Values {
                    bools: vec![true],
                    f32s: vec![0.75],
                    ..Default::default()
                },
            ),
        })
        .unwrap();
        // Telemetry carries on for five seconds with nothing commanded.
        for i in 0..250u32 {
            lg.write(&logger::Record::Sample {
                t_us: i as u64 * 20_000,
                node_id: 1,
                dir: Dir::Telemetry,
                record: i + 1,
                payload: payload(
                    &tl,
                    Values {
                        bools: vec![false],
                        i32s: vec![i as i32],
                        f32s: vec![0.0],
                        strs: vec![String::new()],
                        ..Default::default()
                    },
                ),
            })
            .unwrap();
        }
        lg.flush().unwrap();
    }

    let files = logger::export_csv(&log_path, &dir.join("csv")).unwrap();
    let csv = std::fs::read_to_string(&files[0]).unwrap();
    let rows: Vec<Vec<&str>> = csv
        .lines()
        .skip(1)
        .map(|l| l.split(',').collect())
        .collect();
    let led = rows[0].len() - 2;

    assert_eq!(
        rows[0][led], "1",
        "the row taken under the command keeps it"
    );
    assert_eq!(
        rows.last().unwrap()[led],
        "",
        "five seconds later it is blank, not still claiming to be driving"
    );
}
