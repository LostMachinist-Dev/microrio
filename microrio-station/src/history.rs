//! Rolling per-channel history, for plotting.
//!
//! Bounded on purpose: a station left running for a week must not grow without
//! limit. The full-rate record lives in the log on disk; this is only what the
//! graph needs.

use std::collections::{HashMap, VecDeque};

/// How far back a chart can look. The longest window offered is this.
pub const RETAIN_S: f64 = 15.0 * 60.0;

/// Hard ceiling per channel, whatever the rate: 15 minutes at about 110 Hz.
/// A node run faster than that keeps a shorter history rather than more memory.
const CAPACITY: usize = 100_000;

#[derive(Default)]
pub struct History {
    series: HashMap<(u16, String), VecDeque<[f64; 2]>>,
}

impl History {
    pub fn push(&mut self, node_id: u16, channel: &str, t_s: f64, v: f64) {
        let buf = self
            .series
            .entry((node_id, channel.to_string()))
            .or_default();
        // A node that reconnects restarts its clock; drop the old trace rather
        // than drawing a line backwards across the plot.
        if let Some(last) = buf.back() {
            if t_s < last[0] {
                buf.clear();
            }
        }
        buf.push_back([t_s, v]);
        while buf.len() > CAPACITY || buf.front().is_some_and(|p| p[0] < t_s - RETAIN_S) {
            buf.pop_front();
        }
    }

    /// Points with `from <= t <= to`, in time order.
    pub fn range(&self, node_id: u16, channel: &str, from: f64, to: f64) -> Vec<[f64; 2]> {
        let Some(buf) = self.series.get(&(node_id, channel.to_string())) else {
            return Vec::new();
        };
        // Time-ordered, so binary search for the start rather than scanning
        // fifteen minutes of samples every frame.
        let start = buf.partition_point(|p| p[0] < from);
        buf.range(start..)
            .take_while(|p| p[0] <= to)
            .copied()
            .collect()
    }

    /// Newest timestamp held for any channel, in station seconds.
    ///
    /// Charts need one shared "now" so two traces at different node rates do
    /// not each scroll to their own right-hand edge.
    pub fn newest(&self) -> Option<f64> {
        self.series
            .values()
            .filter_map(|b| b.back().map(|p| p[0]))
            .fold(None, |acc: Option<f64>, t| {
                Some(acc.map_or(t, |a: f64| a.max(t)))
            })
    }

    /// Pair two channels into X-Y points over the same window.
    ///
    /// Samples are matched by time, not by index: on one node both channels
    /// land in the same frame and match exactly, but across two nodes the
    /// rates and phases differ, so each Y sample takes the most recent X
    /// sample at or before it — a zero-order hold, which is what the value
    /// actually was at that moment. An X sample older than `tolerance_s` is
    /// not a match at all, so a dead source leaves a gap instead of drawing a
    /// straight line out of stale data.
    pub fn pairs_between(
        &self,
        x: (u16, &str),
        y: (u16, &str),
        from: f64,
        to: f64,
        tolerance_s: f64,
    ) -> Vec<[f64; 2]> {
        let (Some(xs), Some(ys)) = (
            self.series.get(&(x.0, x.1.to_string())),
            self.series.get(&(y.0, y.1.to_string())),
        ) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut xi = 0usize;
        for yp in ys.iter().filter(|p| p[0] >= from && p[0] <= to) {
            // Both deques are in time order, so the X cursor only moves
            // forward: this stays linear rather than searching per sample.
            while xi + 1 < xs.len() && xs[xi + 1][0] <= yp[0] {
                xi += 1;
            }
            let Some(xp) = xs.get(xi) else { continue };
            if xp[0] > yp[0] || yp[0] - xp[0] > tolerance_s {
                continue;
            }
            out.push([xp[1], yp[1]]);
        }
        out
    }

    pub fn clear_all(&mut self) {
        self.series.clear();
    }

    pub fn clear_node(&mut self, node_id: u16) {
        self.series.retain(|(n, _), _| *n != node_id);
    }
}

/// Thin a time series for drawing: at most `buckets` slices, each kept as its
/// lowest and highest point in time order. A five-minute window at 50 Hz is
/// 15 000 points, far more than there are pixels; min/max keeps every spike
/// visible where plain decimation would skip over it.
pub fn thin(points: Vec<[f64; 2]>, buckets: usize) -> Vec<[f64; 2]> {
    if points.len() <= buckets * 2 || buckets == 0 {
        return points;
    }
    let (t0, t1) = (points[0][0], points[points.len() - 1][0]);
    let span = (t1 - t0).max(f64::EPSILON);
    let mut out = Vec::with_capacity(buckets * 2);
    let mut i = 0;
    while i < points.len() {
        let b = (((points[i][0] - t0) / span) * buckets as f64) as usize;
        let mut lo = points[i];
        let mut hi = points[i];
        let mut j = i;
        while j < points.len() && ((((points[j][0] - t0) / span) * buckets as f64) as usize) == b {
            if points[j][1] < lo[1] {
                lo = points[j];
            }
            if points[j][1] > hi[1] {
                hi = points[j];
            }
            j += 1;
        }
        if lo[0] <= hi[0] {
            out.push(lo);
            if hi != lo {
                out.push(hi);
            }
        } else {
            out.push(hi);
            out.push(lo);
        }
        i = j;
    }
    out
}

// ------------------------------------------------------------------- view

/// Range that covers every value, with a little air, and never zero-height.
///
/// A chart's bounds are set outright every frame, so this has to answer for
/// an empty series and for a dead-flat one as well as for real data: an empty
/// or zero-height range is what leaves a plot blank or unreadable.
pub fn fit(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in values.filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (-1.0, 1.0);
    }
    span(lo - (hi - lo) * 0.05, hi + (hi - lo) * 0.05)
}

/// Two typed limits in the order a chart needs them, never zero-height and
/// never backwards however they were entered.
pub fn span(a: f64, b: f64) -> (f64, f64) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let (lo, hi) = (
        if lo.is_finite() { lo } else { -1.0 },
        if hi.is_finite() { hi } else { 1.0 },
    );
    if hi - lo < 1e-9 {
        (lo - 0.5, hi + 0.5)
    } else {
        (lo, hi)
    }
}

/// Where a drag leaves the view, in seconds back from now.
///
/// Dragging right looks further back. It stops at now, so the chart can never
/// be dragged into the future, and at the end of what is kept, so it can never
/// be dragged off into empty space it cannot come back from.
pub fn panned(pan_s: f64, drag_px: f64, width_px: f64, window_s: f64) -> f64 {
    let per_px = window_s / width_px.max(1.0);
    let limit = (RETAIN_S - window_s).max(0.0);
    let out = pan_s + drag_px * per_px;
    if out.is_finite() {
        out.clamp(0.0, limit)
    } else {
        0.0
    }
}
