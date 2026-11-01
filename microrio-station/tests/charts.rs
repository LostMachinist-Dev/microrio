//! Chart axes, and the time-matching behind an X-Y plot.

#[path = "../src/config.rs"]
mod config;
#[path = "../src/history.rs"]
mod history;

use config::{Axis, AxisKind, ChartConfig};
use history::History;

#[test]
fn axis_kinds_report_whether_they_are_time() {
    assert!(Axis::elapsed().is_time());
    assert!(Axis {
        kind: AxisKind::Clock,
        node_id: 0,
        channel: String::new()
    }
    .is_time());
    assert!(!Axis::channel(2, "a0").is_time());
    assert_eq!(Axis::channel(2, "a0").label(), "n2·a0");
}

/// On one node both channels land in the same frame, so pairing is exact.
#[test]
fn pairs_on_one_node_match_sample_for_sample() {
    let mut h = History::default();
    for i in 0..10 {
        let t = i as f64 * 0.02;
        h.push(1, "x", t, i as f64);
        h.push(1, "y", t, (i * 2) as f64);
    }
    let pts = h.pairs_between(
        (1, "x"),
        (1, "y"),
        h.newest().unwrap() - 10.0,
        h.newest().unwrap(),
        0.5,
    );
    assert_eq!(pts.len(), 10);
    assert_eq!(pts[3], [3.0, 6.0]);
    assert_eq!(pts[9], [9.0, 18.0]);
}

/// Across two boards the rates differ, so each Y sample takes the most recent
/// X sample at or before it rather than the one at the same index.
#[test]
fn pairs_across_nodes_hold_the_last_x() {
    let mut h = History::default();
    // X at 10 Hz, Y at 20 Hz.
    for i in 0..5 {
        h.push(1, "x", i as f64 * 0.1, i as f64);
    }
    for i in 0..10 {
        h.push(2, "y", i as f64 * 0.05, 100.0 + i as f64);
    }
    let pts = h.pairs_between(
        (1, "x"),
        (2, "y"),
        h.newest().unwrap() - 10.0,
        h.newest().unwrap(),
        0.5,
    );

    // y at t=0.15 falls between x samples at 0.10 and 0.20, so it holds the
    // one at 0.10 — value 1.0 — rather than interpolating or skipping.
    let at_015 = pts.iter().find(|p| p[1] == 103.0).expect("y@0.15 present");
    assert_eq!(at_015[0], 1.0);

    // Every pair uses an x that already existed, never a future one.
    assert!(pts.iter().all(|p| (0.0..=4.0).contains(&p[0])));
}

/// A source that stopped must leave a gap, not a flat line drawn out of a
/// value that is minutes old.
#[test]
fn a_stale_x_is_not_paired() {
    let mut h = History::default();
    h.push(1, "x", 0.0, 7.0);
    // Y keeps reporting long after X stopped.
    for i in 0..5 {
        h.push(2, "y", 2.0 + i as f64 * 0.05, i as f64);
    }
    let pts = h.pairs_between(
        (1, "x"),
        (2, "y"),
        h.newest().unwrap() - 10.0,
        h.newest().unwrap(),
        0.5,
    );
    assert!(
        pts.is_empty(),
        "x is two seconds stale against a 0.5 s tolerance, so nothing pairs"
    );
}

#[test]
fn newest_spans_every_series() {
    let mut h = History::default();
    h.push(1, "a", 1.0, 0.0);
    h.push(2, "b", 3.5, 0.0);
    h.push(1, "c", 2.0, 0.0);
    assert_eq!(h.newest(), Some(3.5));
    assert_eq!(History::default().newest(), None);
}

#[test]
fn thinning_keeps_every_spike() {
    // Fifteen thousand points of flat line with one spike up and one down.
    let mut pts: Vec<[f64; 2]> = (0..15_000).map(|i| [i as f64 * 0.02, 0.0]).collect();
    pts[7_001][1] = 5.0;
    pts[11_003][1] = -3.0;
    let thin = history::thin(pts, 800);
    assert!(thin.len() <= 1600, "{} points", thin.len());
    assert!(thin.iter().any(|p| p[1] == 5.0), "the spike survives");
    assert!(thin.iter().any(|p| p[1] == -3.0), "the dip survives");
    assert!(
        thin.windows(2).all(|w| w[0][0] <= w[1][0]),
        "still in time order, so the line does not double back"
    );
}

#[test]
fn history_holds_fifteen_minutes() {
    let mut h = History::default();
    // Twenty minutes at 50 Hz.
    for i in 0..60_000 {
        h.push(1, "a", i as f64 * 0.02, i as f64);
    }
    let all = h.range(1, "a", 0.0, f64::MAX);
    let span = all.last().unwrap()[0] - all.first().unwrap()[0];
    assert!((span - history::RETAIN_S).abs() < 0.05, "kept {span} s");
}

#[test]
fn range_is_inclusive_and_ordered() {
    let mut h = History::default();
    for i in 0..100 {
        h.push(1, "a", i as f64, i as f64);
    }
    let r = h.range(1, "a", 10.0, 20.0);
    assert_eq!(r.first().unwrap()[0], 10.0);
    assert_eq!(r.last().unwrap()[0], 20.0);
    assert_eq!(r.len(), 11);
}

// ------------------------------------------------- the view, and its limits
//
// A chart whose bounds can wander is a chart that can be left unreadable with
// no way back, which is exactly what a stray scroll used to do.

#[test]
fn auto_bounds_are_always_drawable() {
    // No data at all.
    let (lo, hi) = history::fit(std::iter::empty());
    assert!(lo < hi && lo.is_finite() && hi.is_finite());

    // A dead-flat trace still gets a range with height.
    let (lo, hi) = history::fit([3.0, 3.0, 3.0].into_iter());
    assert!(hi - lo > 0.0, "{lo}..{hi}");
    assert!(lo <= 3.0 && hi >= 3.0);

    // Rubbish in the data does not poison the range.
    let (lo, hi) = history::fit([f64::NAN, 1.0, f64::INFINITY, -1.0].into_iter());
    assert!(lo < -1.0 && hi > 1.0, "{lo}..{hi}");
}

#[test]
fn typed_limits_are_taken_in_either_order() {
    assert_eq!(history::span(-1.0, 1.0), (-1.0, 1.0));
    let (lo, hi) = history::span(5.0, -5.0);
    assert_eq!((lo, hi), (-5.0, 5.0), "entered backwards, drawn forwards");
    let (lo, hi) = history::span(2.0, 2.0);
    assert!(hi > lo, "equal limits still have height");
}

#[test]
fn panning_cannot_leave_the_chart_anywhere_it_cannot_return_from() {
    let w = 1000.0;
    // Drag right by a tenth of the width: a tenth of the window back in time.
    assert!((history::panned(0.0, 100.0, w, 60.0) - 6.0).abs() < 1e-9);
    // Dragging left from live stops at now, never into the future.
    assert_eq!(history::panned(0.0, -500.0, w, 60.0), 0.0);
    // And it stops at the end of what is kept.
    let far = history::panned(0.0, 1e9, w, 60.0);
    assert_eq!(far, history::RETAIN_S - 60.0);
    // A window as long as the history cannot pan at all.
    assert_eq!(history::panned(0.0, 1e9, w, history::RETAIN_S), 0.0);
    // Nonsense from a zero-width window does not produce a NaN view.
    assert!(history::panned(0.0, 10.0, 0.0, 60.0).is_finite());
}
