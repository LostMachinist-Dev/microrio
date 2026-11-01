//! Record numbering, backfill and loss accounting.

use microrio_proto::flag;
use microrio_proto::frames::{TelemFrame, TelemRecord, Values};
use microrio_proto::records::Tracker;
use microrio_proto::schema::Layout;
use microrio_proto::{Error, State};

/// A frame carrying records `newest` down to `newest - count + 1`.
fn frame(newest: u32, count: u32) -> TelemFrame {
    TelemFrame {
        echo_host_time_us: 0,
        state: State::Enabled,
        flags: 0,
        rate_hz: 50,
        records: (0..count)
            .map(|i| TelemRecord {
                record: newest - i,
                node_time_us: (newest - i) * 20_000,
                values: Values::default(),
            })
            .collect(),
    }
}

/// Record numbers the tracker accepted from a frame.
fn accepted(t: &mut Tracker, f: &TelemFrame) -> Vec<u32> {
    t.offer(f)
        .new
        .iter()
        .map(|&i| f.records[i].record)
        .collect()
}

#[test]
fn records_must_be_contiguous_and_newest_first() {
    let layout = Layout::default();
    let mut buf = vec![0u8; 256];

    let mut f = frame(10, 3);
    f.records.swap(0, 2);
    assert_eq!(f.encode(&layout, &mut buf), Err(Error::BadRecords));

    let mut empty = frame(10, 1);
    empty.records.clear();
    assert_eq!(empty.encode(&layout, &mut buf), Err(Error::BadRecords));
}

#[test]
fn a_zero_count_frame_does_not_decode() {
    let layout = Layout::default();
    let mut buf = vec![0u8; 64];
    let n = frame(5, 1).encode(&layout, &mut buf).unwrap();
    buf[12] = 0; // the count byte
    assert_eq!(
        TelemFrame::decode(&layout, &buf[..n]),
        Err(Error::BadRecords)
    );
}

#[test]
fn a_clean_link_acks_every_record() {
    let mut t = Tracker::new(8);
    for r in 1..=100 {
        assert_eq!(accepted(&mut t, &frame(r, 1)), vec![r]);
        assert_eq!(t.ack(), r);
    }
    assert_eq!((t.received, t.lost, t.recovered), (100, 0, 0));
    assert_eq!(t.delivered_pct(), Some(100.0));
}

#[test]
fn a_dropped_frame_is_backfilled_and_nothing_is_counted_lost() {
    let mut t = Tracker::new(8);
    accepted(&mut t, &frame(1, 1));
    accepted(&mut t, &frame(2, 1));
    // Frame carrying 3 was dropped. The node, still seeing ack 2, sends 4 and 3.
    assert_eq!(accepted(&mut t, &frame(4, 2)), vec![4, 3]);
    assert_eq!(t.ack(), 4);
    assert_eq!((t.received, t.recovered, t.lost), (4, 1, 0));
}

#[test]
fn duplicates_are_not_logged_twice() {
    let mut t = Tracker::new(8);
    accepted(&mut t, &frame(3, 3));
    // Our ack did not reach the node, so it resends all three plus one.
    assert_eq!(accepted(&mut t, &frame(4, 4)), vec![4]);
    assert_eq!(t.received, 4);
}

#[test]
fn a_gap_older_than_the_history_is_lost_and_does_not_stall_the_ack() {
    let mut t = Tracker::new(4);
    accepted(&mut t, &frame(10, 1));
    // Ten records went missing; the node only kept the last four.
    assert_eq!(accepted(&mut t, &frame(21, 4)), vec![21, 20, 19, 18]);
    assert_eq!(t.ack(), 21);
    assert_eq!(t.lost, 7); // 11..=17
}

#[test]
fn an_uno_with_no_history_still_measures_true_loss() {
    let mut t = Tracker::new(1);
    accepted(&mut t, &frame(1, 1));
    accepted(&mut t, &frame(2, 1));
    accepted(&mut t, &frame(5, 1));
    assert_eq!(t.ack(), 5);
    assert_eq!((t.received, t.lost), (3, 2));
    assert_eq!(t.delivered_pct(), Some(60.0));
}

#[test]
fn the_first_frame_seen_starts_the_count_rather_than_scoring_the_past() {
    // Joining a node that has been up for an hour is not an hour of loss.
    let mut t = Tracker::new(8);
    assert_eq!(accepted(&mut t, &frame(180_000, 1)), vec![180_000]);
    assert_eq!(t.lost, 0);
    assert_eq!(t.ack(), 180_000);
}

#[test]
fn a_reboot_restarts_the_count() {
    let mut t = Tracker::new(8);
    for r in 1..=500 {
        accepted(&mut t, &frame(r, 1));
    }
    let mut f = frame(1, 1);
    f.flags = flag::NEVER_LINKED;
    let got = t.offer(&f);
    assert!(got.restarted);
    assert_eq!(got.new, vec![0]);
    assert_eq!(t.ack(), 1);
}

#[test]
fn a_reboot_is_seen_even_after_the_node_has_relinked() {
    // The node got a control frame before we saw its first telemetry, so the
    // flag is already clear. The count going back that far still says reboot.
    let mut t = Tracker::new(8);
    for r in 1..=500 {
        accepted(&mut t, &frame(r, 1));
    }
    assert!(t.offer(&frame(3, 1)).restarted);
}

#[test]
fn a_late_duplicate_is_not_mistaken_for_a_reboot() {
    let mut t = Tracker::new(8);
    for r in 1..=20 {
        accepted(&mut t, &frame(r, 1));
    }
    let got = t.offer(&frame(17, 1));
    assert!(!got.restarted);
    assert!(got.new.is_empty());
    assert_eq!(t.ack(), 20);
}
