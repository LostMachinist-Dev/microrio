//! Binary session log, plus CSV export.
//!
//! Telemetry *and* control are recorded, because debugging a control problem
//! without the control history is guesswork. Writing is append-only and the
//! network path never blocks on disk.
//!
//! Single rate makes the log rectangular: every channel has a real sample at
//! every tick, so export is a straight dump rather than a resampling problem.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use microrio_proto::frames::Values;
use microrio_proto::schema::{ChannelDef, Layout, Schema};
use microrio_proto::{ChanType, Dir};

/// Version 2 added the record number to each sample. Version 1 logs still export.
const MAGIC: &[u8; 6] = b"MRLOG\x02";
const MAGIC_V1: &[u8; 6] = b"MRLOG\x01";
const KIND_SCHEMA: u8 = 1;
const KIND_SAMPLE: u8 = 2;
const KIND_META: u8 = 3;

pub enum Record {
    /// Written at session start, and again whenever a node's schema changes.
    Schema { node_id: u16, schema: Schema },
    Sample {
        /// When the sample was taken, on the station's clock. A resent record
        /// arrives late but carries its original time.
        t_us: u64,
        node_id: u16,
        dir: Dir,
        /// The node's record number for telemetry; 0 for control.
        record: u32,
        payload: Vec<u8>,
    },
    /// Ties the log's station-relative timestamps to the wall clock, so an
    /// export can print the time of day a row was taken. Written once, at
    /// the start. Logs recorded before this existed simply lack it, and
    /// export falls back to elapsed time alone.
    Meta { started_unix_us: i64 },
    /// Not written to the file: asks the logging thread to flush what it is
    /// holding and say when it has, so an export reads a complete file.
    Flush(std::sync::mpsc::SyncSender<()>),
}

pub struct Logger {
    out: BufWriter<File>,
    path: PathBuf,
}

impl Logger {
    pub fn create(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let name = format!("{}.mrlog", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let path = dir.join(name);
        let mut out = BufWriter::new(File::create(&path)?);
        out.write_all(MAGIC)?;
        Ok(Self { out, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&mut self, rec: &Record) -> anyhow::Result<()> {
        let mut body = Vec::new();
        let kind = match rec {
            Record::Schema { node_id, schema } => {
                body.extend_from_slice(&node_id.to_le_bytes());
                body.extend_from_slice(&(schema.channels.len() as u16).to_le_bytes());
                for c in &schema.channels {
                    body.push(c.dir as u8);
                    body.push(c.ctype as u8);
                    body.extend_from_slice(&c.slot.to_le_bytes());
                    body.push(c.name.len() as u8);
                    body.extend_from_slice(c.name.as_bytes());
                }
                KIND_SCHEMA
            }
            Record::Sample {
                t_us,
                node_id,
                dir,
                record,
                payload,
            } => {
                body.extend_from_slice(&t_us.to_le_bytes());
                body.extend_from_slice(&node_id.to_le_bytes());
                body.push(*dir as u8);
                body.extend_from_slice(&record.to_le_bytes());
                body.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                body.extend_from_slice(payload);
                KIND_SAMPLE
            }
            Record::Meta { started_unix_us } => {
                body.extend_from_slice(&started_unix_us.to_le_bytes());
                KIND_META
            }
            Record::Flush(ack) => {
                self.out.flush()?;
                let _ = ack.send(());
                return Ok(());
            }
        };
        self.out.write_all(&[kind])?;
        self.out.write_all(&(body.len() as u32).to_le_bytes())?;
        self.out.write_all(&body)?;
        Ok(())
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        self.out.flush()?;
        Ok(())
    }
}

// ------------------------------------------------------------------ export

struct NodeLog {
    channels: Vec<ChannelDef>,
    telem: Layout,
    ctrl: Layout,
}

/// How much of a log to export, and how to stamp it.
pub struct ExportOpts {
    /// Zone for the `time_local` column. `None` omits the column, which is
    /// what the headless `export` subcommand does when nobody said a zone.
    pub zone: Option<crate::clock::Zone>,
    /// Station-relative microseconds. `None` means from the start / to the end.
    pub from_us: Option<u64>,
    pub to_us: Option<u64>,
}

impl Default for ExportOpts {
    fn default() -> Self {
        Self {
            zone: None,
            from_us: None,
            to_us: None,
        }
    }
}

/// Export whole-session CSVs into a directory, one `node<id>.csv` per node.
pub fn export_csv(log: &Path, out_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let dir = out_dir.to_path_buf();
    export(log, &ExportOpts::default(), move |id| {
        dir.join(format!("node{id}.csv"))
    })
}

/// Export a marked window to a path the operator chose.
///
/// The chosen path is used verbatim for the first node in the log; a second
/// node lands beside it as `<name>-node<id>.csv`. One board, which is the
/// normal case, therefore produces exactly the file that was asked for.
pub fn export_window(log: &Path, out: &Path, opts: &ExportOpts) -> anyhow::Result<Vec<PathBuf>> {
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let first = std::cell::Cell::new(true);
    let out = out.to_path_buf();
    export(log, opts, move |id| {
        if first.replace(false) {
            out.clone()
        } else {
            let stem = out
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "export".into());
            let ext = out
                .extension()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "csv".into());
            out.with_file_name(format!("{stem}-node{id}.{ext}"))
        }
    })
}

/// How old the last control values may be before a telemetry row stops
/// claiming them.
///
/// Control is carried forward onto the next telemetry row, because that is
/// the state the row was taken under. But a station that stopped commanding —
/// because it dropped to watching, or its link went — must not have its last
/// command painted across the rest of the session. A second is fifty frames
/// at the usual rate: long enough to bridge any gap, short enough that a
/// stopped stream shows as blank.
const CONTROL_STALE_US: u64 = 1_000_000;

/// How far a resent record can arrive behind its own timestamp and still be
/// put back in order. Resending finishes within a few frames; two seconds is a
/// wide margin that keeps export memory bounded on an all-day log.
const REORDER_WINDOW_US: u64 = 2_000_000;

/// Export state shared by every node's rows.
struct Sink<'a> {
    opts: &'a ExportOpts,
    started_unix_us: Option<i64>,
    nodes: HashMap<u16, NodeLog>,
    writers: HashMap<u16, BufWriter<File>>,
    /// The last control values seen for a node, and when. Carried onto
    /// telemetry rows, but only while they are still current.
    last_ctrl: HashMap<u16, (u64, Values)>,
    /// Samples not yet written, per node, in time order. Ties keep arrival order.
    pending: HashMap<u16, std::collections::BTreeMap<(u64, u64), (Dir, Vec<u8>)>>,
    newest_t: HashMap<u16, u64>,
    arrival: u64,
}

impl Sink<'_> {
    fn push(&mut self, node_id: u16, t_us: u64, dir: Dir, payload: Vec<u8>) -> anyhow::Result<()> {
        self.arrival += 1;
        self.pending
            .entry(node_id)
            .or_default()
            .insert((t_us, self.arrival), (dir, payload));
        let newest = self.newest_t.entry(node_id).or_insert(0);
        *newest = (*newest).max(t_us);
        let cutoff = newest.saturating_sub(REORDER_WINDOW_US);
        self.drain(node_id, Some(cutoff))
    }

    /// Write out a node's pending rows older than `before`, or all of them.
    fn drain(&mut self, node_id: u16, before: Option<u64>) -> anyhow::Result<()> {
        loop {
            let Some(q) = self.pending.get_mut(&node_id) else {
                return Ok(());
            };
            let Some((&(t_us, _), _)) = q.first_key_value() else {
                return Ok(());
            };
            if before.is_some_and(|b| t_us >= b) {
                return Ok(());
            }
            let (_, (dir, payload)) = q.pop_first().unwrap();
            self.row(node_id, t_us, dir, &payload)?;
        }
    }

    fn row(&mut self, node_id: u16, t_us: u64, dir: Dir, payload: &[u8]) -> anyhow::Result<()> {
        let Some(node) = self.nodes.get(&node_id) else {
            return Ok(()); // samples before the schema block: nothing to name them with
        };
        let layout = if dir == Dir::Telemetry {
            &node.telem
        } else {
            &node.ctrl
        };
        let Ok(values) = Values::decode_payload(layout, payload) else {
            return Ok(());
        };

        // Control values are carried forward onto the next telemetry row, so
        // one outside the window still sets the state the first row inside it
        // was taken under.
        if dir == Dir::Control {
            self.last_ctrl.insert(node_id, (t_us, values));
            return Ok(());
        }

        if self.opts.from_us.is_some_and(|a| t_us < a) || self.opts.to_us.is_some_and(|b| t_us > b)
        {
            return Ok(());
        }

        let Some(w) = self.writers.get_mut(&node_id) else {
            return Ok(());
        };
        write!(w, "{t_us}")?;
        if let (Some(zone), Some(epoch)) = (self.opts.zone.as_ref(), self.started_unix_us) {
            write!(w, ",{}", zone.stamp_us(epoch + t_us as i64))?;
        }
        emit(w, &values, &node.channels, Dir::Telemetry)?;
        match self
            .last_ctrl
            .get(&node_id)
            .filter(|(at, _)| t_us.saturating_sub(*at) <= CONTROL_STALE_US)
        {
            Some((_, c)) => emit(w, c, &node.channels, Dir::Control)?,
            None => {
                for _ in node.channels.iter().filter(|c| c.dir == Dir::Control) {
                    write!(w, ",")?;
                }
            }
        }
        writeln!(w)?;
        Ok(())
    }
}

/// One row per telemetry record, in the order the records were taken, carrying
/// the control values that were current at that moment.
fn export(
    log: &Path,
    opts: &ExportOpts,
    mut name_for: impl FnMut(u16) -> PathBuf,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut f = BufReader::new(File::open(log)?);
    let mut magic = [0u8; 6];
    f.read_exact(&mut magic)?;
    let has_record = match &magic {
        m if m == MAGIC => true,
        m if m == MAGIC_V1 => false,
        _ => anyhow::bail!("{} is not a MicroRIO log", log.display()),
    };

    let mut sink = Sink {
        opts,
        // Only known once the meta record is read; a log without one exports
        // elapsed time only rather than inventing a wall clock.
        started_unix_us: None,
        nodes: HashMap::new(),
        writers: HashMap::new(),
        last_ctrl: HashMap::new(),
        pending: HashMap::new(),
        newest_t: HashMap::new(),
        arrival: 0,
    };
    let mut written = Vec::new();

    loop {
        let mut kind = [0u8; 1];
        if f.read_exact(&mut kind).is_err() {
            break; // clean end, or a truncated tail from a hard stop
        }
        let mut len = [0u8; 4];
        if f.read_exact(&mut len).is_err() {
            break;
        }
        let mut body = vec![0u8; u32::from_le_bytes(len) as usize];
        if f.read_exact(&mut body).is_err() {
            break;
        }

        match kind[0] {
            KIND_META => {
                if body.len() >= 8 {
                    sink.started_unix_us = Some(i64::from_le_bytes(body[0..8].try_into().unwrap()));
                }
            }
            KIND_SCHEMA => {
                let node_id = u16::from_le_bytes([body[0], body[1]]);
                // Rows taken under the old schema belong to the old file.
                sink.drain(node_id, None)?;

                let n = u16::from_le_bytes([body[2], body[3]]) as usize;
                let mut p = 4;
                let mut channels = Vec::with_capacity(n);
                for _ in 0..n {
                    let dir = Dir::from_u8(body[p])?;
                    let ctype = ChanType::from_u8(body[p + 1])?;
                    let slot = u16::from_le_bytes([body[p + 2], body[p + 3]]);
                    let nl = body[p + 4] as usize;
                    let name = String::from_utf8_lossy(&body[p + 5..p + 5 + nl]).into_owned();
                    p += 5 + nl;
                    channels.push(ChannelDef {
                        dir,
                        ctype,
                        slot,
                        name,
                    });
                }
                let schema = Schema {
                    info: microrio_proto::schema::NodeInfo {
                        name: String::new(),
                        rate_hz: 0,
                        max_frame: microrio_proto::DEFAULT_MAX_FRAME as u16,
                        total_channels: n as u16,
                        history: 1,
                    },
                    channels: channels.clone(),
                };
                let telem = schema.layout(Dir::Telemetry);
                let ctrl = schema.layout(Dir::Control);

                let path = name_for(node_id);
                let mut w = BufWriter::new(File::create(&path)?);
                let stamped = opts.zone.is_some() && sink.started_unix_us.is_some();
                write!(w, "time_us")?;
                if stamped {
                    write!(w, ",time_local")?;
                }
                for c in channels.iter().filter(|c| c.dir == Dir::Telemetry) {
                    write!(w, ",{}", c.name)?;
                }
                for c in channels.iter().filter(|c| c.dir == Dir::Control) {
                    write!(w, ",ctrl_{}", c.name)?;
                }
                writeln!(w)?;
                sink.writers.insert(node_id, w);
                written.push(path);

                sink.nodes.insert(
                    node_id,
                    NodeLog {
                        channels,
                        telem,
                        ctrl,
                    },
                );
            }
            KIND_SAMPLE => {
                let t_us = u64::from_le_bytes(body[0..8].try_into().unwrap());
                let node_id = u16::from_le_bytes([body[8], body[9]]);
                let dir = Dir::from_u8(body[10])?;
                let mut p = 11;
                if has_record {
                    p += 4; // the record number; ordering is by time
                }
                let plen = u16::from_le_bytes([body[p], body[p + 1]]) as usize;
                let payload = body[p + 2..p + 2 + plen].to_vec();
                sink.push(node_id, t_us, dir, payload)?;
            }
            _ => {}
        }
    }

    let ids: Vec<u16> = sink.pending.keys().copied().collect();
    for id in ids {
        sink.drain(id, None)?;
    }
    for (_, mut w) in sink.writers {
        w.flush()?;
    }
    Ok(written)
}

fn emit(
    w: &mut BufWriter<File>,
    v: &Values,
    channels: &[ChannelDef],
    dir: Dir,
) -> anyhow::Result<()> {
    for c in channels.iter().filter(|c| c.dir == dir) {
        let i = c.slot as usize;
        match c.ctype {
            ChanType::Bool => write!(w, ",{}", v.bools.get(i).copied().unwrap_or(false) as u8)?,
            ChanType::I32 => write!(w, ",{}", v.i32s.get(i).copied().unwrap_or(0))?,
            ChanType::F32 => write!(w, ",{}", v.f32s.get(i).copied().unwrap_or(0.0))?,
            ChanType::Cmd => write!(w, ",{}", v.cmds.get(i).copied().unwrap_or(0))?,
            ChanType::Str => {
                let s = v.strs.get(i).cloned().unwrap_or_default();
                if s.contains(',') || s.contains('"') {
                    write!(w, ",\"{}\"", s.replace('"', "\"\""))?;
                } else {
                    write!(w, ",{s}")?;
                }
            }
        }
    }
    Ok(())
}
