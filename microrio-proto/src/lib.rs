//! MicroRIO wire protocol.
//!
//! Pure codec: schema negotiation, control frames, telemetry frames. No I/O, no
//! allocation in the hot path, `no_std`-capable so a future Rust firmware can
//! share it with the driver station.
//!
//! # Direction
//!
//! Every channel has a direction, and it decides what the GUI does with it:
//!
//! - [`Dir::Telemetry`] — controller to station. The GUI *displays* it.
//! - [`Dir::Control`] — station to controller. The GUI *populates* it.
//!
//! On the controller, "read" means read what the station sent. Control values do
//! not come from the board's own hardware; something in the GUI has to drive them.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod frames;
pub mod records;
pub mod schema;

include!(concat!(env!("OUT_DIR"), "/consts.rs"));

use core::fmt;

/// Everything that can go wrong decoding a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Buffer ended before the field did.
    Truncated,
    /// Not a MicroRIO frame.
    BadMagic,
    /// Protocol version we do not speak. Rejected, never negotiated.
    BadVersion(u8),
    /// Message type byte is not one we know.
    BadMsgType(u8),
    /// Channel type byte is not one we know.
    BadChanType(u8),
    /// Direction byte is not 0 or 1.
    BadDir(u8),
    /// State byte is outside the defined set.
    BadState(u8),
    /// A name was longer than [`MAX_NAME_LEN`] or not valid UTF-8.
    BadName,
    /// Encode target was too small.
    BufferTooSmall,
    /// Payload length disagreed with the layout the schema implies.
    LayoutMismatch,
    /// A `cmd` channel was declared in the telemetry direction.
    CmdMustBeControl,
    /// Node advertised a max frame below [`MIN_MAX_FRAME`].
    FrameSizeTooSmall(u16),
    /// A telemetry frame with no records, too many, or numbers that are not
    /// contiguous newest-first.
    BadRecords,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => write!(f, "frame ended mid-field"),
            Error::BadMagic => write!(f, "not a MicroRIO frame"),
            Error::BadVersion(v) => write!(f, "unsupported protocol version {v}"),
            Error::BadMsgType(t) => write!(f, "unknown message type {t}"),
            Error::BadChanType(t) => write!(f, "unknown channel type {t}"),
            Error::BadDir(d) => write!(f, "invalid direction {d}"),
            Error::BadState(s) => write!(f, "invalid state {s}"),
            Error::BadName => write!(f, "name too long or not UTF-8"),
            Error::BufferTooSmall => write!(f, "encode buffer too small"),
            Error::LayoutMismatch => write!(f, "payload length disagrees with schema layout"),
            Error::CmdMustBeControl => write!(f, "cmd channels are control-direction only"),
            Error::FrameSizeTooSmall(n) => write!(f, "advertised max frame {n} is below the floor"),
            Error::BadRecords => write!(f, "telemetry records missing or not contiguous"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;

/// Which way a channel flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    /// Controller to station. The GUI displays it.
    Telemetry = 0,
    /// Station to controller. The GUI populates it.
    Control = 1,
}

impl Dir {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Dir::Telemetry),
            1 => Ok(Dir::Control),
            other => Err(Error::BadDir(other)),
        }
    }
}

/// Channel value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChanType {
    Bool = 0,
    I32 = 1,
    F32 = 2,
    /// Fixed [`STR_LEN`] bytes, NUL-padded.
    Str = 3,
    /// Monotonic counter. A one-shot that survives packet loss.
    Cmd = 4,
}

impl ChanType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(ChanType::Bool),
            1 => Ok(ChanType::I32),
            2 => Ok(ChanType::F32),
            3 => Ok(ChanType::Str),
            4 => Ok(ChanType::Cmd),
            other => Err(Error::BadChanType(other)),
        }
    }

    /// Wire width in bytes. Bools are bitpacked and report 0; see [`schema::Layout`].
    pub fn size(self) -> usize {
        match self {
            ChanType::Bool => 0,
            ChanType::I32 | ChanType::F32 => 4,
            ChanType::Str => STR_LEN,
            ChanType::Cmd => 2,
        }
    }
}

/// Operating state. Outputs are live only in [`State::Enabled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Disabled = 0,
    Enabled = 1,
    /// Operator estop. Clears only on an explicit operator reset.
    Estop = 2,
    /// Something broke: UI heartbeat went stale, or the node's watchdog tripped.
    /// Clears when the cause clears.
    Fault = 3,
}

impl State {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(State::Disabled),
            1 => Ok(State::Enabled),
            2 => Ok(State::Estop),
            3 => Ok(State::Fault),
            other => Err(Error::BadState(other)),
        }
    }

    /// Whether a node may drive its outputs in this state.
    pub fn outputs_live(self) -> bool {
        matches!(self, State::Enabled)
    }
}

/// Message kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsgType {
    /// Station to node: "send me schema chunk N".
    SchemaReq = 1,
    /// Node to station: one chunk of the schema.
    Schema = 2,
    /// Station to node: full control state.
    Control = 3,
    /// Node to station: full telemetry state.
    Telem = 4,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            msg::SCHEMA_REQ => Ok(MsgType::SchemaReq),
            msg::SCHEMA => Ok(MsgType::Schema),
            msg::CONTROL => Ok(MsgType::Control),
            msg::TELEM => Ok(MsgType::Telem),
            other => Err(Error::BadMsgType(other)),
        }
    }
}

/// The 12-byte frame header that precedes every message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub msg: MsgType,
    pub node_id: u16,
    /// Identifies the schema these values are laid out against. A frame whose
    /// hash does not match the cached schema is discarded, never guessed at.
    pub schema_hash: u32,
    /// Wrapping. Diagnostics only: it never delays or reorders application.
    pub seq: u16,
}

impl Header {
    pub fn new(msg: MsgType, node_id: u16, schema_hash: u32, seq: u16) -> Self {
        Self {
            msg,
            node_id,
            schema_hash,
            seq,
        }
    }

    pub fn encode(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < HEADER_LEN {
            return Err(Error::BufferTooSmall);
        }
        buf[0..2].copy_from_slice(&MAGIC.to_le_bytes());
        buf[2] = PROTO_VERSION;
        buf[3] = self.msg as u8;
        buf[4..6].copy_from_slice(&self.node_id.to_le_bytes());
        buf[6..10].copy_from_slice(&self.schema_hash.to_le_bytes());
        buf[10..12].copy_from_slice(&self.seq.to_le_bytes());
        Ok(HEADER_LEN)
    }

    /// Decode a header, returning it alongside the remaining body bytes.
    pub fn decode(buf: &[u8]) -> Result<(Self, &[u8])> {
        if buf.len() < HEADER_LEN {
            return Err(Error::Truncated);
        }
        if u16::from_le_bytes([buf[0], buf[1]]) != MAGIC {
            return Err(Error::BadMagic);
        }
        if buf[2] != PROTO_VERSION {
            return Err(Error::BadVersion(buf[2]));
        }
        let hdr = Header {
            msg: MsgType::from_u8(buf[3])?,
            node_id: u16::from_le_bytes([buf[4], buf[5]]),
            schema_hash: u32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]),
            seq: u16::from_le_bytes([buf[10], buf[11]]),
        };
        Ok((hdr, &buf[HEADER_LEN..]))
    }
}

/// Little-endian cursor over a byte slice. Keeps the codec readable and bounds-checked.
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub(crate) fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub(crate) fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
}

/// Little-endian append-only writer.
pub(crate) struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub(crate) fn len(&self) -> usize {
        self.pos
    }
    pub(crate) fn bytes(&mut self, src: &[u8]) -> Result<()> {
        if self.buf.len() - self.pos < src.len() {
            return Err(Error::BufferTooSmall);
        }
        self.buf[self.pos..self.pos + src.len()].copy_from_slice(src);
        self.pos += src.len();
        Ok(())
    }
    pub(crate) fn u8(&mut self, v: u8) -> Result<()> {
        self.bytes(&[v])
    }
    pub(crate) fn u16(&mut self, v: u16) -> Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub(crate) fn u32(&mut self, v: u32) -> Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub(crate) fn i32(&mut self, v: i32) -> Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub(crate) fn f32(&mut self, v: f32) -> Result<()> {
        self.bytes(&v.to_bits().to_le_bytes())
    }
}
