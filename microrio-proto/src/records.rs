//! Station-side bookkeeping for numbered telemetry records.
//!
//! The node numbers every telemetry sample and keeps the last few. The station
//! answers with `ack_record`: the highest record it holds with nothing missing
//! below it. The node then resends anything newer than that while it still has
//! it. Control is untouched by any of this — it stays full-state and
//! last-write-wins — so the only thing being made reliable is the record of
//! what happened, never a command.
//!
//! A record that falls out of the node's history before it arrives is lost
//! for good. The tracker counts it and moves the ack past it, so one gap can
//! never hold every later record hostage. An Uno advertises a history of 1,
//! which makes every gap immediately final: it gets true loss measurement and
//! nothing else.

use alloc::collections::BTreeSet;

use crate::flag;
use crate::frames::TelemFrame;

#[derive(Debug, Clone, Default)]
pub struct Tracker {
    /// Highest record held, or given up on, with nothing outstanding below it.
    ack: u32,
    started: bool,
    /// Records newer than `ack` that have arrived out of order.
    ahead: BTreeSet<u32>,
    /// How many records the node can resend, newest included.
    history: u32,
    /// Records that arrived, first time only.
    pub received: u64,
    /// Records that arrived only because they were resent.
    pub recovered: u64,
    /// Records that will never arrive.
    pub lost: u64,
}

/// What [`Tracker::offer`] decided about one frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Accepted {
    /// Indices into `frame.records` that are new, and belong in the log.
    pub new: alloc::vec::Vec<usize>,
    /// The node's numbering went backwards: it rebooted. Everything before
    /// this frame belongs to its previous life.
    pub restarted: bool,
}

impl Tracker {
    pub fn new(history: u8) -> Self {
        Self {
            history: (history as u32).max(1),
            ..Self::default()
        }
    }

    /// The value to send back in the next control frame.
    pub fn ack(&self) -> u32 {
        self.ack
    }

    /// Take a decoded frame and work out which of its records are new.
    pub fn offer(&mut self, frame: &TelemFrame) -> Accepted {
        let mut out = Accepted::default();
        let Some(first) = frame.records.first() else {
            return out;
        };
        let newest = first.record;
        let oldest = frame.records.last().map(|r| r.record).unwrap_or(newest);

        // A reboot restarts the count at 1. A frame entirely below the ack is
        // either that or a stale duplicate; a node that has not heard from us
        // since boot says so in its flags, and a count that has gone back
        // further than the node could ever resend is a reboot either way.
        let restarted = self.started
            && newest <= self.ack
            && (frame.flags & flag::NEVER_LINKED != 0
                || newest.saturating_add(self.history) <= self.ack);
        if !self.started || restarted {
            self.started = true;
            self.ack = oldest.saturating_sub(1);
            self.ahead.clear();
            out.restarted = restarted;
        }

        for (i, rec) in frame.records.iter().enumerate() {
            let r = rec.record;
            if r <= self.ack || !self.ahead.insert(r) {
                continue; // already held
            }
            out.new.push(i);
            self.received += 1;
            if i > 0 {
                self.recovered += 1;
            }
        }

        // Advance over everything held, and over anything that has fallen out
        // of the node's history and so cannot be filled any more.
        let oldest_resendable = newest.saturating_sub(self.history - 1);
        loop {
            let next = self.ack + 1;
            if self.ahead.remove(&next) {
                self.ack = next;
            } else if next < oldest_resendable {
                self.ack = next;
                self.lost += 1;
            } else {
                break;
            }
        }
        out
    }

    /// Percentage of records that made it, counting resends as made it.
    pub fn delivered_pct(&self) -> Option<f64> {
        let total = self.received + self.lost;
        (total > 0).then(|| self.received as f64 * 100.0 / total as f64)
    }

    /// Halve the counters, so the figure tracks the recent past and a fixed
    /// problem clears instead of lingering for the rest of the session.
    pub fn decay(&mut self) {
        self.received /= 2;
        self.recovered /= 2;
        self.lost /= 2;
    }
}
