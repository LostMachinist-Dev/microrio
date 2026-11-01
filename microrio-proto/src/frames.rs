//! Control and telemetry frames.
//!
//! Both carry **full state** every tick: there is no delta encoding and no
//! retransmission. A dropped frame heals on the next one. This is deliberate —
//! a retransmitted stale command is worse than a dropped command.
//!
//! Payload order is fixed by the schema layout: bools bitpacked first, then
//! i32, f32, str and cmd, each in slot order. Nothing self-describes, because
//! the schema already described it.

use alloc::string::String;
use alloc::vec::Vec;

use crate::schema::Layout;
use crate::{
    Error, Reader, Result, State, Writer, CONTROL_FIXED, RECORD_FIXED, STR_LEN, TELEM_FIXED,
};

/// One tick's worth of values for a single direction.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Values {
    pub bools: Vec<bool>,
    pub i32s: Vec<i32>,
    pub f32s: Vec<f32>,
    /// Fixed-width on the wire, NUL-padded; trailing NULs are stripped on decode.
    pub strs: Vec<String>,
    /// Monotonic counters. The node fires when one differs from what it last saw.
    pub cmds: Vec<u16>,
}

impl Values {
    /// Allocate a zeroed set matching `layout`.
    pub fn zeroed(layout: &Layout) -> Self {
        Self {
            bools: alloc::vec![false; layout.n_bool],
            i32s: alloc::vec![0; layout.n_i32],
            f32s: alloc::vec![0.0; layout.n_f32],
            strs: alloc::vec![String::new(); layout.n_str],
            cmds: alloc::vec![0; layout.n_cmd],
        }
    }

    /// Encode just the values, as the log stores them.
    pub fn encode_payload(&self, layout: &Layout) -> Result<Vec<u8>> {
        let mut buf = alloc::vec![0u8; layout.payload_len()];
        let mut w = Writer::new(&mut buf);
        self.encode(layout, &mut w)?;
        Ok(buf)
    }

    /// Decode a bare payload, as the log stores it.
    pub fn decode_payload(layout: &Layout, payload: &[u8]) -> Result<Self> {
        Self::decode(layout, &mut Reader::new(payload))
    }

    fn matches(&self, layout: &Layout) -> bool {
        self.bools.len() == layout.n_bool
            && self.i32s.len() == layout.n_i32
            && self.f32s.len() == layout.n_f32
            && self.strs.len() == layout.n_str
            && self.cmds.len() == layout.n_cmd
    }

    fn encode(&self, layout: &Layout, w: &mut Writer<'_>) -> Result<()> {
        if !self.matches(layout) {
            return Err(Error::LayoutMismatch);
        }

        // Bools, LSB-first within each byte.
        let mut packed = alloc::vec![0u8; layout.bool_bytes()];
        for (i, b) in self.bools.iter().enumerate() {
            if *b {
                packed[i / 8] |= 1 << (i % 8);
            }
        }
        w.bytes(&packed)?;

        for v in &self.i32s {
            w.i32(*v)?;
        }
        for v in &self.f32s {
            w.f32(*v)?;
        }
        for s in &self.strs {
            let mut field = [0u8; STR_LEN];
            let bytes = s.as_bytes();
            let n = bytes.len().min(STR_LEN);
            field[..n].copy_from_slice(&bytes[..n]);
            w.bytes(&field)?;
        }
        for v in &self.cmds {
            w.u16(*v)?;
        }
        Ok(())
    }

    fn decode(layout: &Layout, r: &mut Reader<'_>) -> Result<Self> {
        let packed = r.take(layout.bool_bytes())?;
        let bools = (0..layout.n_bool)
            .map(|i| packed[i / 8] & (1 << (i % 8)) != 0)
            .collect();

        let mut i32s = Vec::with_capacity(layout.n_i32);
        for _ in 0..layout.n_i32 {
            i32s.push(r.i32()?);
        }
        let mut f32s = Vec::with_capacity(layout.n_f32);
        for _ in 0..layout.n_f32 {
            f32s.push(r.f32()?);
        }
        let mut strs = Vec::with_capacity(layout.n_str);
        for _ in 0..layout.n_str {
            let field = r.take(STR_LEN)?;
            let end = field.iter().position(|&b| b == 0).unwrap_or(STR_LEN);
            // A node with a flaky sensor can emit non-UTF-8; show it rather than
            // dropping the whole frame.
            strs.push(String::from_utf8_lossy(&field[..end]).into_owned());
        }
        let mut cmds = Vec::with_capacity(layout.n_cmd);
        for _ in 0..layout.n_cmd {
            cmds.push(r.u16()?);
        }

        Ok(Values {
            bools,
            i32s,
            f32s,
            strs,
            cmds,
        })
    }
}

/// Station to node. Carries the authoritative state the node must obey.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlFrame {
    /// Station's monotonic clock, truncated to 32 bits. The node echoes it back
    /// so the station can measure round-trip without any clock synchronisation.
    pub host_time_us: u32,
    pub state: State,
    pub flags: u8,
    /// How often the node should send telemetry. 0 leaves the sketch's own
    /// rate in force. This is the telemetry rate only — and so the logging
    /// rate — never the board's loop.
    pub rate_hz: u16,
    /// Highest telemetry record the station holds with nothing missing below
    /// it. Records the station gave up on count as held, so this never stalls
    /// behind a gap that cannot be filled. 0 means none yet.
    pub ack_record: u32,
    pub values: Values,
}

impl ControlFrame {
    /// Body width for a layout, excluding the frame header.
    pub fn body_len(layout: &Layout) -> usize {
        CONTROL_FIXED + layout.payload_len()
    }

    pub fn encode(&self, layout: &Layout, buf: &mut [u8]) -> Result<usize> {
        let mut w = Writer::new(buf);
        w.u32(self.host_time_us)?;
        w.u8(self.state as u8)?;
        w.u8(self.flags)?;
        w.u16(self.rate_hz)?;
        w.u32(self.ack_record)?;
        self.values.encode(layout, &mut w)?;
        Ok(w.len())
    }

    pub fn decode(layout: &Layout, body: &[u8]) -> Result<Self> {
        let mut r = Reader::new(body);
        let host_time_us = r.u32()?;
        let state = State::from_u8(r.u8()?)?;
        let flags = r.u8()?;
        let rate_hz = r.u16()?;
        let ack_record = r.u32()?;
        let values = Values::decode(layout, &mut r)?;
        Ok(Self {
            host_time_us,
            state,
            flags,
            rate_hz,
            ack_record,
            values,
        })
    }

    /// The control payload alone, as the log stores it.
    pub fn payload(body: &[u8]) -> &[u8] {
        body.get(CONTROL_FIXED..).unwrap_or(&[])
    }
}

/// One telemetry sample, numbered by the node.
///
/// Numbering is what lets a frame carry more than the newest sample: the
/// station says which records it already has, and the node resends what is
/// missing while it still holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemRecord {
    /// Counts up from 1 at boot, one per telemetry tick.
    pub record: u32,
    /// Node's own clock when the sample was taken, truncated to 32 bits. A
    /// resent record keeps its original time, so it can be put back where it
    /// belongs in the log.
    pub node_time_us: u32,
    pub values: Values,
}

/// Node to station. Carries measured values plus enough to judge link health.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemFrame {
    /// The most recent `host_time_us` this node received. Round-trip falls out
    /// of comparing it against the station's clock now.
    pub echo_host_time_us: u32,
    /// What the node believes its state to be, which may lag what was commanded.
    pub state: State,
    /// See [`crate::flag`].
    pub flags: u8,
    /// The telemetry rate the node is actually sending at.
    pub rate_hz: u16,
    /// Newest first, numbered contiguously downwards. Never empty.
    pub records: Vec<TelemRecord>,
}

impl TelemFrame {
    /// Body width for a frame carrying `count` records.
    pub fn body_len(layout: &Layout, count: usize) -> usize {
        TELEM_FIXED + count * (RECORD_FIXED + layout.payload_len())
    }

    /// The live sample: the one the operator should be looking at.
    pub fn newest(&self) -> &TelemRecord {
        &self.records[0]
    }

    pub fn encode(&self, layout: &Layout, buf: &mut [u8]) -> Result<usize> {
        let Some(first) = self.records.first() else {
            return Err(Error::BadRecords);
        };
        if self.records.len() > u8::MAX as usize {
            return Err(Error::BadRecords);
        }
        for (i, rec) in self.records.iter().enumerate() {
            if rec.record != first.record.wrapping_sub(i as u32) {
                return Err(Error::BadRecords);
            }
        }
        let mut w = Writer::new(buf);
        w.u32(self.echo_host_time_us)?;
        w.u8(self.state as u8)?;
        w.u8(self.flags)?;
        w.u16(self.rate_hz)?;
        w.u32(first.record)?;
        w.u8(self.records.len() as u8)?;
        for rec in &self.records {
            w.u32(rec.node_time_us)?;
            rec.values.encode(layout, &mut w)?;
        }
        Ok(w.len())
    }

    pub fn decode(layout: &Layout, body: &[u8]) -> Result<Self> {
        let mut r = Reader::new(body);
        let echo_host_time_us = r.u32()?;
        let state = State::from_u8(r.u8()?)?;
        let flags = r.u8()?;
        let rate_hz = r.u16()?;
        let newest = r.u32()?;
        let count = r.u8()? as usize;
        if count == 0 {
            return Err(Error::BadRecords);
        }
        let mut records = Vec::with_capacity(count);
        for i in 0..count {
            let node_time_us = r.u32()?;
            let values = Values::decode(layout, &mut r)?;
            records.push(TelemRecord {
                record: newest.wrapping_sub(i as u32),
                node_time_us,
                values,
            });
        }
        Ok(Self {
            echo_host_time_us,
            state,
            flags,
            rate_hz,
            records,
        })
    }

    /// Raw round-trip in microseconds, handling the 32-bit wrap.
    ///
    /// Wrap is every ~71 minutes; a real round-trip is milliseconds, so
    /// wrapping subtraction is correct **provided the echo is one of ours**.
    /// It may not be: a node that was talking to a previous station keeps
    /// echoing that station's clock until our first control frame lands, and
    /// subtracting an unrelated clock yields a meaningless number. Prefer
    /// [`rtt_us_checked`](Self::rtt_us_checked).
    pub fn rtt_us(&self, now_host_us: u32) -> u32 {
        now_host_us.wrapping_sub(self.echo_host_time_us)
    }

    /// Round-trip, or `None` when the echo cannot be one of ours.
    ///
    /// Rejects a zero echo (the node has not received a control frame yet) and
    /// anything above `max_plausible_us`, which is how a stale echo from a
    /// previous station session shows up: as a wrapped value near `u32::MAX`.
    pub fn rtt_us_checked(&self, now_host_us: u32, max_plausible_us: u32) -> Option<u32> {
        if self.echo_host_time_us == 0 {
            return None;
        }
        let d = self.rtt_us(now_host_us);
        (d <= max_plausible_us).then_some(d)
    }
}
