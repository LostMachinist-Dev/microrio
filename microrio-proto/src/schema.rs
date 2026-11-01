//! Schema: what channels a node has, negotiated once at connect.
//!
//! The firmware declares named channels. On connect the node sends them as one
//! or more chunks; the station builds a name-to-slot map and from then on both
//! sides speak in slots. Names never appear in a control or telemetry frame.
//!
//! Chunking is not optional: an Uno advertising a 256-byte frame cannot fit
//! forty channel names in one datagram.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{
    ChanType, Dir, Error, Header, MsgType, Reader, Result, Writer, FNV_OFFSET, FNV_PRIME,
    HEADER_LEN, MAX_NAME_LEN, MIN_MAX_FRAME, STR_LEN,
};

/// One declared channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDef {
    pub dir: Dir,
    pub ctype: ChanType,
    /// Index within this `(dir, ctype)` group. Payload order follows it.
    pub slot: u16,
    pub name: String,
}

/// Node-level facts, carried in schema chunk 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    pub name: String,
    pub rate_hz: u16,
    /// Largest datagram this node will send or accept. The station splits to fit.
    pub max_frame: u16,
    pub total_channels: u16,
    /// How many telemetry records the node can resend, newest included. 1
    /// means no backfill — a board with no room to spare streams its frames
    /// and keeps nothing.
    pub history: u8,
}

/// A node's full channel set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub info: NodeInfo,
    pub channels: Vec<ChannelDef>,
}

/// How many channels of each type flow one way, and how wide the payload is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Layout {
    pub n_bool: usize,
    pub n_i32: usize,
    pub n_f32: usize,
    pub n_str: usize,
    pub n_cmd: usize,
}

impl Layout {
    /// Bitpacked bools occupy whole bytes.
    pub fn bool_bytes(&self) -> usize {
        self.n_bool.div_ceil(8)
    }

    /// Total value-payload width, excluding the frame's own time and state fields.
    pub fn payload_len(&self) -> usize {
        self.bool_bytes() + self.n_i32 * 4 + self.n_f32 * 4 + self.n_str * STR_LEN + self.n_cmd * 2
    }
}

impl Schema {
    /// Count channels flowing one direction.
    pub fn layout(&self, dir: Dir) -> Layout {
        let mut l = Layout::default();
        for c in self.channels.iter().filter(|c| c.dir == dir) {
            match c.ctype {
                ChanType::Bool => l.n_bool += 1,
                ChanType::I32 => l.n_i32 += 1,
                ChanType::F32 => l.n_f32 += 1,
                ChanType::Str => l.n_str += 1,
                ChanType::Cmd => l.n_cmd += 1,
            }
        }
        l
    }

    /// Look a channel up by name.
    pub fn find(&self, name: &str) -> Option<&ChannelDef> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// FNV-1a over direction, type, slot and name of every channel in order.
    ///
    /// Node id, rate and max frame are deliberately excluded: two identical
    /// boards with different ids have the same schema, and rate is runtime
    /// tunable without invalidating anything.
    pub fn hash(&self) -> u32 {
        let mut h = FNV_OFFSET;
        let mut feed = |b: u8| {
            h ^= b as u32;
            h = h.wrapping_mul(FNV_PRIME);
        };
        for c in &self.channels {
            feed(c.dir as u8);
            feed(c.ctype as u8);
            let s = c.slot.to_le_bytes();
            feed(s[0]);
            feed(s[1]);
            for b in c.name.as_bytes() {
                feed(*b);
            }
        }
        h
    }

    /// Reject anything the rest of the codec assumes away.
    pub fn validate(&self) -> Result<()> {
        if self.info.max_frame < MIN_MAX_FRAME as u16 {
            return Err(Error::FrameSizeTooSmall(self.info.max_frame));
        }
        if self.info.name.len() > MAX_NAME_LEN {
            return Err(Error::BadName);
        }
        for c in &self.channels {
            if c.name.is_empty() || c.name.len() > MAX_NAME_LEN {
                return Err(Error::BadName);
            }
            // A cmd is a counter the station increments; it only makes sense inbound.
            if c.ctype == ChanType::Cmd && c.dir != Dir::Control {
                return Err(Error::CmdMustBeControl);
            }
        }
        Ok(())
    }

    /// Split into chunks that each fit `max_frame`, and encode them.
    ///
    /// Chunk 0 additionally carries [`NodeInfo`].
    ///
    /// A single entry always fits: [`validate`](Self::validate) caps names at
    /// [`MAX_NAME_LEN`] and floors `max_frame` at [`MIN_MAX_FRAME`], so the
    /// worst case preamble plus one entry stays well inside the budget.
    pub fn encode_chunks(&self, node_id: u16, seq_start: u16) -> Result<Vec<Vec<u8>>> {
        self.validate()?;
        let budget = self.info.max_frame as usize;

        // Group channels into chunks that fit, before writing anything, so the
        // chunk count in every header is correct.
        let mut groups: Vec<Vec<&ChannelDef>> = Vec::new();
        let mut current: Vec<&ChannelDef> = Vec::new();
        let mut used = HEADER_LEN + Self::chunk_preamble_len(true, &self.info);

        for c in &self.channels {
            let entry = Self::entry_len(c);
            if !current.is_empty() && used + entry > budget {
                groups.push(core::mem::take(&mut current));
                used = HEADER_LEN + Self::chunk_preamble_len(false, &self.info) + entry;
            } else {
                used += entry;
            }
            current.push(c);
        }
        groups.push(current);

        let chunk_count = groups.len() as u16;
        let mut out = Vec::with_capacity(groups.len());

        for (i, group) in groups.iter().enumerate() {
            let mut buf = alloc::vec![0u8; budget];
            let hdr = Header::new(
                MsgType::Schema,
                node_id,
                self.hash(),
                seq_start.wrapping_add(i as u16),
            );
            let n = hdr.encode(&mut buf)?;
            let mut w = Writer::new(&mut buf[n..]);

            w.u16(i as u16)?;
            w.u16(chunk_count)?;
            w.u16(group.len() as u16)?;

            if i == 0 {
                w.u16(self.info.rate_hz)?;
                w.u16(self.info.max_frame)?;
                w.u16(self.info.total_channels)?;
                w.u8(self.info.history)?;
                w.u8(self.info.name.len() as u8)?;
                w.bytes(self.info.name.as_bytes())?;
            }

            for c in group {
                w.u8(c.dir as u8)?;
                w.u8(c.ctype as u8)?;
                w.u16(c.slot)?;
                w.u8(c.name.len() as u8)?;
                w.bytes(c.name.as_bytes())?;
            }

            let total = n + w.len();
            buf.truncate(total);
            out.push(buf);
        }
        Ok(out)
    }

    fn entry_len(c: &ChannelDef) -> usize {
        1 + 1 + 2 + 1 + c.name.len()
    }

    fn chunk_preamble_len(is_first: bool, info: &NodeInfo) -> usize {
        let base = 2 + 2 + 2; // chunk_index, chunk_count, entry_count
        if is_first {
            base + 2 + 2 + 2 + 1 + 1 + info.name.len()
        } else {
            base
        }
    }
}

/// One decoded schema chunk, before chunks are stitched together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaChunk {
    pub chunk_index: u16,
    pub chunk_count: u16,
    /// Present on chunk 0 only.
    pub info: Option<NodeInfo>,
    pub channels: Vec<ChannelDef>,
}

impl SchemaChunk {
    /// Decode a chunk body, i.e. everything after the 12-byte header.
    pub fn decode(body: &[u8]) -> Result<Self> {
        let mut r = Reader::new(body);
        let chunk_index = r.u16()?;
        let chunk_count = r.u16()?;
        let entry_count = r.u16()?;

        let info = if chunk_index == 0 {
            let rate_hz = r.u16()?;
            let max_frame = r.u16()?;
            let total_channels = r.u16()?;
            let history = r.u8()?;
            let name_len = r.u8()? as usize;
            if name_len > MAX_NAME_LEN {
                return Err(Error::BadName);
            }
            let name = core::str::from_utf8(r.take(name_len)?).map_err(|_| Error::BadName)?;
            Some(NodeInfo {
                name: String::from(name),
                rate_hz,
                max_frame,
                total_channels,
                history,
            })
        } else {
            None
        };

        let mut channels = Vec::with_capacity(entry_count as usize);
        for _ in 0..entry_count {
            let dir = Dir::from_u8(r.u8()?)?;
            let ctype = ChanType::from_u8(r.u8()?)?;
            let slot = r.u16()?;
            let name_len = r.u8()? as usize;
            if name_len == 0 || name_len > MAX_NAME_LEN {
                return Err(Error::BadName);
            }
            let name = core::str::from_utf8(r.take(name_len)?).map_err(|_| Error::BadName)?;
            channels.push(ChannelDef {
                dir,
                ctype,
                slot,
                name: String::from(name),
            });
        }

        Ok(SchemaChunk {
            chunk_index,
            chunk_count,
            info,
            channels,
        })
    }
}

/// Collects chunks until the set is complete.
///
/// Chunks arrive over UDP, so they may be duplicated, reordered, or dropped.
#[derive(Debug, Default)]
pub struct SchemaAssembler {
    expected: Option<u16>,
    info: Option<NodeInfo>,
    parts: Vec<Option<Vec<ChannelDef>>>,
}

impl SchemaAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk. Returns the finished [`Schema`] once every chunk has arrived.
    pub fn push(&mut self, chunk: SchemaChunk) -> Option<Schema> {
        // A changed chunk count means the node restarted mid-handshake.
        if self.expected != Some(chunk.chunk_count) {
            self.expected = Some(chunk.chunk_count);
            self.parts = alloc::vec![None; chunk.chunk_count as usize];
            self.info = None;
        }
        if let Some(info) = chunk.info {
            self.info = Some(info);
        }
        if let Some(slot) = self.parts.get_mut(chunk.chunk_index as usize) {
            *slot = Some(chunk.channels);
        }

        if self.parts.iter().all(Option::is_some) {
            let info = self.info.clone()?;
            let mut channels = Vec::new();
            for p in &self.parts {
                channels.extend(p.as_ref()?.iter().cloned());
            }
            return Some(Schema { info, channels });
        }
        None
    }

    /// Chunk indices still outstanding, for targeted re-requests.
    pub fn missing(&self) -> Vec<u16> {
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_none())
            .map(|(i, _)| i as u16)
            .collect()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Encode a `SCHEMA_REQ` body: which chunk the station wants.
pub fn encode_schema_req(chunk_index: u16, buf: &mut [u8]) -> Result<usize> {
    let mut w = Writer::new(buf);
    w.u16(chunk_index)?;
    Ok(w.len())
}

/// Decode a `SCHEMA_REQ` body.
pub fn decode_schema_req(body: &[u8]) -> Result<u16> {
    Reader::new(body).u16()
}
