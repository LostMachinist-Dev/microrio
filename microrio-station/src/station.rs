//! Shared station state and the per-node network loops.
//!
//! The UI never sits in the control path. It publishes a heartbeat and reads
//! snapshots; the network loops own the timing. That separation exists so a
//! hung UI cannot stop the FAULT frame going out — which would otherwise be the
//! one packet in the system that never sends.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use microrio_proto::frames::{ControlFrame, TelemFrame, Values};
use microrio_proto::records::Tracker;
use microrio_proto::schema::{encode_schema_req, Schema, SchemaAssembler, SchemaChunk};
use microrio_proto::{Dir, Header, MsgType, State, HEADER_LEN};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::config::{CrossLink, NodeConfig, Profile};
use crate::history::History;
use crate::logger::Record;

/// What to say for a node that has produced no schema yet.
///
/// "Waiting" on its own is true and useless: it reads as a fault in the
/// station, and sends an operator hunting through settings. All three real
/// reasons are at the other end of the wire, so say whether anything has come
/// back at all, and name them.
pub fn no_schema_note(cfg: &NodeConfig, view: &NodeView) -> String {
    if view.schema_rx > 0 {
        return format!(
            "Reading this node's channel list: {} chunks so far.",
            view.schema_rx
        );
    }
    // A second of asking is long enough to tell the difference between a
    // handshake in progress and nothing being there.
    if view.schema_reqs > 50 {
        format!(
            "No answer from {}:{} — {} requests sent, nothing back. The board is \
             switched off, at a different address, or running a sketch that is not a \
             MicroRIO node.",
            cfg.ip, cfg.port, view.schema_reqs
        )
    } else {
        format!("Asking {}:{} what channels it has…", cfg.ip, cfg.port)
    }
}

/// Live view of one node, for the UI.
#[derive(Debug, Clone, Default)]
pub struct NodeView {
    pub connected: bool,
    pub schema: Option<Schema>,
    pub telemetry: Option<Values>,
    pub node_state: Option<State>,
    pub node_flags: u8,
    pub rtt_us: Option<u32>,
    pub rx_frames: u64,
    pub tx_frames: u64,
    pub last_rx_ago_ms: u64,
    /// Wall clock at the last telemetry frame. `last_rx_ago_ms` is only
    /// refreshed by this node's own loop, so a crosslink reading another
    /// node needs a timestamp it can compare against now by itself.
    pub last_rx_unix_ms: u64,
    /// Set when a crosslink driving this node has lost its source.
    pub link_fault: Option<String>,
    pub schema_chunks: u16,
    /// Schema requests sent while there is still no schema, and chunks that
    /// came back. Zero answers after hundreds of asks is the difference
    /// between "negotiating" and "nothing is there", which is worth saying
    /// out loud rather than leaving an operator to watch a blank page.
    pub schema_reqs: u64,
    pub schema_rx: u64,
    /// Percentage of telemetry records that never made it into the log, after
    /// resending. This is the true loss figure: a frame the network dropped
    /// but the node resent costs nothing here.
    pub record_loss_pct: Option<f64>,
    /// Records recovered by resending, recently. Shows backfill is working.
    pub records_recovered: u64,
    /// The telemetry rate the node says it is actually sending at.
    pub node_rate_hz: Option<u16>,
    /// Another station is talking to this node. See [`Contest`].
    pub contest: Contest,
    pub error: Option<String>,
}

/// Who is driving a node, as far as this station can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Contest {
    /// Nothing says otherwise.
    #[default]
    Alone,
    /// Another station is also talking to this node, and this one owns it.
    /// Its telemetry is being split between the two.
    Shared,
    /// This station does not own the node: its control is being refused.
    Refused,
}

/// Works out whether a node is being fought over.
///
/// Two signals, because one of them works with any firmware. A node running
/// the ownership rules says so with a flag. An older one does not, but it
/// echoes the clock of whichever station it last heard — so a run of echoes
/// that are not ours means someone else is talking to it.
#[derive(Debug, Default)]
pub struct ContestWatch {
    /// Frames judged, recently.
    seen: u32,
    /// How many of them carried someone else's clock.
    foreign: u32,
    /// Control frames sent since the node last accepted one of ours.
    unheard: u32,
    /// What the last frame said.
    verdict: Contest,
}

/// Frames to judge before saying anything. At 50 Hz this is under a second —
/// long enough to rule out the handshake, when a node has not yet heard from
/// us and echoes zero or a previous session.
const ENOUGH_FRAMES: u32 = 40;

/// How much of the telemetry must carry someone else's clock. A node split
/// between two stations sends each of them roughly half.
const FOREIGN_PCT: u32 = 30;

/// Control frames in a row that the node never adopts. Our clock coming back
/// is the only proof it is listening to us rather than to someone else.
const UNHEARD: u32 = 20;

impl ContestWatch {
    /// Take one telemetry frame. `ours` is whether its echo is our clock.
    pub fn frame(&mut self, flags: u8, ours: bool) -> Contest {
        self.seen += 1;
        if ours {
            self.unheard = 0;
        } else {
            self.foreign += 1;
        }
        // Keep the figure about the recent past, so a station that has gone
        // away stops being reported.
        if self.seen >= 128 {
            self.seen /= 2;
            self.foreign /= 2;
        }

        // A node that knows about ownership says so outright, and which side
        // of it we are on falls out of whose clock the frame carries. Our own
        // clock means we are the owner being told someone is knocking; anyone
        // else's — or none — means this frame is the refusal itself.
        if flags & microrio_proto::flag::STATION_CONFLICT != 0 {
            self.verdict = if ours {
                Contest::Shared
            } else {
                Contest::Refused
            };
            return self.verdict;
        }
        self.verdict = self.guess();
        self.verdict
    }

    /// Note that a control frame went out without being acknowledged by the
    /// node adopting our clock. Refusal looks exactly like this.
    pub fn sent(&mut self) {
        self.unheard = self.unheard.saturating_add(1);
    }

    /// Firmware too old to declare ownership: go by whose clock comes back.
    ///
    /// A run of echoes that are not ours means the node is answering someone
    /// else. If our control frames are going unheard as well, that someone
    /// else is driving it and we are not.
    fn guess(&self) -> Contest {
        // A proportion rather than a run: a node answering two stations
        // alternates between them, so consecutive foreign echoes never build
        // up even though half the telemetry is going elsewhere.
        if self.seen < ENOUGH_FRAMES || self.foreign * 100 < FOREIGN_PCT * self.seen {
            return Contest::Alone;
        }
        if self.unheard >= UNHEARD {
            Contest::Refused
        } else {
            Contest::Shared
        }
    }
}

pub struct Station {
    pub profile: Mutex<Profile>,
    /// Milliseconds since the epoch at the last UI frame. The network loops
    /// read this; if it goes stale the machine stops.
    pub ui_beat: AtomicU64,
    pub estop: AtomicBool,
    /// Enable is per node: one board can be driving while another is parked.
    /// It is runtime state, never loaded from a file — opening a profile must
    /// not be able to decide what is live.
    pub node_enabled: Mutex<HashMap<u16, bool>>,
    pub views: Mutex<HashMap<u16, NodeView>>,
    /// Control values the operator has set, by node then channel name.
    pub intents: Mutex<HashMap<u16, HashMap<String, Intent>>>,
    /// Live joystick inputs mapped onto control channels, by node then
    /// channel. Rewritten whole on every joystick poll, so an unplugged pad's
    /// inputs disappear and the channel falls back to its typed value.
    pub joy: Mutex<HashMap<u16, HashMap<String, JoyInput>>>,
    /// Channels the operator is holding at a fixed value, overriding the
    /// joystick mapped to them. Runtime only.
    pub holds: Mutex<std::collections::HashSet<(u16, String)>>,
    /// Nodes this station is only watching. It marks its frames view-only, so
    /// the node sends telemetry back but takes no value from them.
    ///
    /// Set automatically when a node turns out to belong to another station,
    /// and by `--view`. Runtime only: which station is driving is a fact
    /// about this moment, not something a saved file should decide.
    pub viewing: Mutex<HashMap<u16, bool>>,
    /// Every node starts watched, because `--view` said so.
    pub view_by_default: AtomicBool,
    pub log_path: Mutex<Option<String>>,
    pub started: Instant,
    /// Where the current profile came from, and where Save writes back to.
    pub profile_path: Mutex<Option<PathBuf>>,
    /// Bumped whenever the node list changes. The supervisor watches it and
    /// restarts the links, so loading a profile takes effect immediately
    /// instead of needing the app restarted.
    pub profile_epoch: AtomicU64,
    /// Rolling per-channel history for the plot. Bounded; the full-rate record
    /// is the log on disk.
    pub history: Mutex<History>,
    /// Unix microseconds at `started`. Lets a station-relative timestamp be
    /// turned back into a wall-clock time for charts and CSV export.
    pub started_unix_us: i64,
    /// The operator's marked recording window, if one is open.
    pub log_session: Mutex<Option<LogSession>>,
    /// Handle on the logging thread, so an export can ask it to flush first
    /// and read a file that is not missing its last two hundred rows.
    pub log_tx: Mutex<Option<mpsc::UnboundedSender<Record>>>,
    /// How long the longest un-dismissed UI stall was, in milliseconds, or 0.
    ///
    /// A stall faults every node while it lasts. What this records is that it
    /// happened, so the operator is told rather than finding the machine live
    /// again on its own.
    pub stall_ms: AtomicU64,
}

/// An explicitly started recording. The background log runs the whole time the
/// app is open; this only marks the slice the operator cares about.
#[derive(Debug, Clone, Copy)]
pub struct LogSession {
    /// Station-relative microseconds, matching the log's own timestamps.
    pub start_t_us: u64,
}

/// One joystick input, routed to a control channel and cleaned up by its
/// mapping. The channel still has to shape and range it.
#[derive(Debug, Clone, PartialEq)]
pub struct JoyInput {
    /// The axis or button it came from, for display.
    pub source: String,
    /// `-1..1` for a stick, `0..1` for a trigger or button.
    pub deflection: f32,
    pub unipolar: bool,
}

/// Where a control's value is coming from right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Typed or clicked in the station.
    Manual,
    /// A joystick input, named.
    Joystick(String),
    /// A joystick is mapped but the operator is holding a fixed value.
    Held(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Intent {
    Bool(bool),
    I32(i32),
    F32(f32),
    /// Counter, incremented by the UI; the node fires on change.
    Cmd(u16),
}

impl Station {
    pub fn new(profile: Profile, path: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            profile: Mutex::new(profile),
            ui_beat: AtomicU64::new(now_ms()),
            estop: AtomicBool::new(false),
            node_enabled: Mutex::new(HashMap::new()),
            views: Mutex::new(HashMap::new()),
            intents: Mutex::new(HashMap::new()),
            joy: Mutex::new(HashMap::new()),
            holds: Mutex::new(Default::default()),
            viewing: Mutex::new(HashMap::new()),
            view_by_default: AtomicBool::new(false),
            log_path: Mutex::new(None),
            started: Instant::now(),
            history: Mutex::new(History::default()),
            profile_path: Mutex::new(path),
            profile_epoch: AtomicU64::new(1),
            started_unix_us: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_micros() as i64)
                .unwrap_or(0),
            log_session: Mutex::new(None),
            log_tx: Mutex::new(None),
            stall_ms: AtomicU64::new(0),
        })
    }

    /// Replace the whole profile and restart the links.
    ///
    /// Views, intents and plot history all belong to the old node set, so they
    /// go with it rather than bleeding into the new one.
    pub fn adopt_profile(&self, profile: Profile, path: Option<PathBuf>) {
        *self.profile.lock().unwrap() = profile;
        *self.profile_path.lock().unwrap() = path;
        self.views.lock().unwrap().clear();
        self.intents.lock().unwrap().clear();
        self.history.lock().unwrap().clear_all();
        // Loading someone else's profile must never leave the machine live.
        self.node_enabled.lock().unwrap().clear();
        self.profile_epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Tell the supervisor the node list changed.
    pub fn restart_links(&self) {
        self.views.lock().unwrap().clear();
        self.history.lock().unwrap().clear_all();
        // The node list may have changed under us; a node that is gone must
        // not leave an enable flag behind for a future node reusing its id.
        self.node_enabled.lock().unwrap().clear();
        self.profile_epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Called once per rendered frame. Also the place a stall is noticed.
    ///
    /// A stalled UI faults every node, and a fault must not clear itself: the
    /// window coming back says the window came back, not that the operator is
    /// watching again. So a stall that happened while something was live
    /// disables everything and is reported until dismissed. Without this, a
    /// file dialog opened on the UI thread — which blocks rendering for as
    /// long as it is up — would fault the machine and then quietly re-enable
    /// it on close. The dialogs now run on their own threads for that reason,
    /// but the rule stands for anything else that stops the window drawing.
    pub fn beat(&self) {
        let now = now_ms();
        let previous = self.ui_beat.swap(now, Ordering::Relaxed);
        let gap = now.saturating_sub(previous);
        let limit = self.profile.lock().map(|p| p.ui_stale_ms).unwrap_or(250);
        if gap > limit && self.enabled_count() > 0 {
            self.disable_all();
            self.stall_ms.fetch_max(gap, Ordering::Relaxed);
        }
    }

    /// The stall the operator has not acknowledged yet, if any.
    pub fn pending_stall_ms(&self) -> Option<u64> {
        match self.stall_ms.load(Ordering::Relaxed) {
            0 => None,
            ms => Some(ms),
        }
    }

    pub fn clear_stall(&self) {
        self.stall_ms.store(0, Ordering::Relaxed);
    }

    pub fn ui_stale_ms(&self) -> u64 {
        now_ms().saturating_sub(self.ui_beat.load(Ordering::Relaxed))
    }

    /// The state commanded to one node this tick.
    ///
    /// Four independent ways to end up neutral: the operator hits the global
    /// estop, the UI stops rendering, this node is not enabled, or (on the
    /// node itself) control frames stop arriving. Enable is per node so that
    /// parking one board does not park the rest; estop is global because an
    /// emergency is never about one board.
    pub fn effective_state(&self, node_id: u16) -> State {
        if self.estop.load(Ordering::Relaxed) {
            return State::Estop;
        }
        let limit = self.profile.lock().map(|p| p.ui_stale_ms).unwrap_or(250);
        if self.ui_stale_ms() > limit {
            return State::Fault;
        }
        if self.is_node_enabled(node_id) {
            State::Enabled
        } else {
            State::Disabled
        }
    }

    /// One word for the whole station, for the badge at the top of the window.
    ///
    /// Reports the worst thing that is true, so a station with one node
    /// faulted never reads as simply ENABLED.
    pub fn aggregate_state(&self) -> State {
        if self.estop.load(Ordering::Relaxed) {
            return State::Estop;
        }
        let limit = self.profile.lock().map(|p| p.ui_stale_ms).unwrap_or(250);
        if self.ui_stale_ms() > limit {
            return State::Fault;
        }
        if self
            .views
            .lock()
            .map(|v| v.values().any(|n| n.link_fault.is_some()))
            .unwrap_or(false)
        {
            return State::Fault;
        }
        if self.enabled_count() > 0 {
            State::Enabled
        } else {
            State::Disabled
        }
    }

    /// Whether this station is only watching that node.
    pub fn is_viewing(&self, node_id: u16) -> bool {
        self.viewing
            .lock()
            .map(|m| {
                m.get(&node_id)
                    .copied()
                    .unwrap_or_else(|| self.view_by_default.load(Ordering::Relaxed))
            })
            .unwrap_or(false)
    }

    /// Watch a node, or ask to drive it again. Taking control is a request:
    /// if the node still belongs to someone else it refuses, and the station
    /// drops back to watching on its own.
    pub fn set_viewing(&self, node_id: u16, on: bool) {
        if let Ok(mut m) = self.viewing.lock() {
            m.insert(node_id, on);
        }
        if on {
            // Watching is not driving. Whatever was enabled is not any more.
            self.set_node_enabled(node_id, false);
        }
    }

    pub fn is_node_enabled(&self, node_id: u16) -> bool {
        if self.estop.load(Ordering::Relaxed) {
            return false;
        }
        self.node_enabled
            .lock()
            .map(|m| m.get(&node_id).copied().unwrap_or(false))
            .unwrap_or(false)
    }

    pub fn enabled_count(&self) -> usize {
        if self.estop.load(Ordering::Relaxed) {
            return 0;
        }
        self.node_enabled
            .lock()
            .map(|m| m.values().filter(|v| **v).count())
            .unwrap_or(0)
    }

    /// Enabling is refused while the estop is latched; disabling always works.
    pub fn set_node_enabled(&self, node_id: u16, on: bool) {
        if on && self.estop.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut m) = self.node_enabled.lock() {
            m.insert(node_id, on);
        }
    }

    pub fn disable_all(&self) {
        if let Ok(mut m) = self.node_enabled.lock() {
            for v in m.values_mut() {
                *v = false;
            }
        }
    }

    pub fn set_estop(&self) {
        self.estop.store(true, Ordering::Relaxed);
        self.disable_all();
    }

    /// Estop latches; clearing it is always an explicit operator action, and
    /// never re-enables anything on its own.
    pub fn clear_estop(&self) {
        self.estop.store(false, Ordering::Relaxed);
        self.disable_all();
    }

    pub fn set_intent(&self, node_id: u16, channel: &str, intent: Intent) {
        let mut map = self.intents.lock().unwrap();
        map.entry(node_id)
            .or_default()
            .insert(channel.to_string(), intent);
    }

    /// Station-relative microseconds, the same clock the log is stamped with.
    pub fn t_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }

    /// Block until the logging thread has written everything it is holding.
    ///
    /// Returns false if there is no logger, or it did not answer in time.
    /// Called from a worker thread, never the UI thread: the UI going quiet
    /// for even a moment is a fault by design.
    pub fn flush_log(&self, timeout: Duration) -> bool {
        let tx = { self.log_tx.lock().ok().and_then(|g| g.clone()) };
        let Some(tx) = tx else { return false };
        let (ack, done) = std::sync::mpsc::sync_channel(1);
        if tx.send(Record::Flush(ack)).is_err() {
            return false;
        }
        done.recv_timeout(timeout).is_ok()
    }

    /// The value a control channel stands at, before offset and clamping,
    /// and where it came from.
    ///
    /// One answer for both the frame builder and the grid, so the grid can
    /// never show a value other than the one being sent. A held channel uses
    /// the typed value; otherwise a mapped joystick wins over it.
    pub fn resolve(
        &self,
        node_id: u16,
        c: &microrio_proto::schema::ChannelDef,
        ccfg: &crate::config::ControlConfig,
    ) -> (Option<Intent>, Source) {
        let manual = self
            .intents
            .lock()
            .unwrap()
            .get(&node_id)
            .and_then(|m| m.get(&c.name))
            .copied();
        let joy = self
            .joy
            .lock()
            .unwrap()
            .get(&node_id)
            .and_then(|m| m.get(&c.name))
            .cloned();
        let Some(joy) = joy else {
            return (manual, Source::Manual);
        };
        if self
            .holds
            .lock()
            .unwrap()
            .contains(&(node_id, c.name.clone()))
        {
            return (manual, Source::Held(joy.source));
        }
        let d = joy.deflection as f64;
        let v = match c.ctype {
            microrio_proto::ChanType::Bool => Some(Intent::Bool(d >= 0.5)),
            microrio_proto::ChanType::I32 => {
                Some(Intent::I32(ccfg.from_stick(d, joy.unipolar).round() as i32))
            }
            microrio_proto::ChanType::F32 => {
                Some(Intent::F32(ccfg.from_stick(d, joy.unipolar) as f32))
            }
            // Nothing sensible maps a stick onto a string or a counter.
            _ => return (manual, Source::Manual),
        };
        (v, Source::Joystick(joy.source))
    }

    /// Hold a joystick-driven channel at a fixed value, or let it go. Taking
    /// hold starts from where the stick was, so the output does not jump.
    pub fn set_hold(
        &self,
        node_id: u16,
        c: &microrio_proto::schema::ChannelDef,
        ccfg: &crate::config::ControlConfig,
        on: bool,
    ) {
        if on {
            if let (Some(v), _) = self.resolve(node_id, c, ccfg) {
                self.set_intent(node_id, &c.name, v);
            }
            self.holds.lock().unwrap().insert((node_id, c.name.clone()));
        } else {
            self.holds
                .lock()
                .unwrap()
                .remove(&(node_id, c.name.clone()));
        }
    }

    pub fn bump_cmd(&self, node_id: u16, channel: &str) {
        let mut map = self.intents.lock().unwrap();
        let e = map.entry(node_id).or_default();
        let next = match e.get(channel) {
            Some(Intent::Cmd(v)) => v.wrapping_add(1),
            _ => 1,
        };
        e.insert(channel.to_string(), Intent::Cmd(next));
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Owns the per-node links and rebuilds them whenever the profile changes.
///
/// Without this, loading a profile would update the settings page and nothing
/// else until the app was restarted.
pub async fn supervisor(station: Arc<Station>, log_tx: mpsc::UnboundedSender<Record>) {
    let mut running_epoch = 0u64;
    let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // One listener for the whole station: relayed frames carry the node id,
    // so there is nothing per-node about receiving them.
    {
        let st = station.clone();
        let tx = log_tx.clone();
        tokio::spawn(async move { watch_relay(st, tx).await });
    }

    loop {
        let epoch = station.profile_epoch.load(Ordering::Relaxed);
        if epoch != running_epoch {
            for h in handles.drain(..) {
                h.abort();
            }
            let nodes: Vec<NodeConfig> = station
                .profile
                .lock()
                .unwrap()
                .nodes
                .iter()
                .filter(|n| n.link_active)
                .cloned()
                .collect();
            for cfg in nodes {
                let st = station.clone();
                let tx = log_tx.clone();
                handles.push(tokio::spawn(async move { node_task(st, cfg, tx).await }));
            }
            running_epoch = epoch;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Take in what another station is relaying, for the nodes being watched.
///
/// A watched node has no link of its own — this station never speaks to the
/// board — so everything the window shows for it arrives here: the schema
/// another station is forwarding, and each telemetry frame exactly as it came
/// off the wire. It is logged the same way too, so a watcher's recording is a
/// real recording.
async fn watch_relay(station: Arc<Station>, log_tx: mpsc::UnboundedSender<Record>) {
    let port = station
        .profile
        .lock()
        .map(|p| p.relay_port)
        .unwrap_or(crate::relay::DEFAULT_PORT);
    let mut listener = match crate::relay::Listener::bind(port) {
        Ok(l) => l,
        Err(e) => {
            // Watching is one feature of several; a station that cannot
            // listen still drives perfectly well.
            eprintln!("not listening for relayed telemetry: {e}");
            return;
        }
    };
    let mut asm: HashMap<u16, SchemaAssembler> = HashMap::new();
    let mut trackers: HashMap<u16, Tracker> = HashMap::new();
    let mut logged: std::collections::HashSet<u16> = Default::default();

    loop {
        match listener.next().await {
            crate::relay::Relayed::Schema(node_id, body) => {
                if !station.is_viewing(node_id) {
                    continue;
                }
                let Ok(chunk) = SchemaChunk::decode(&body) else {
                    continue;
                };
                let have = station.views.lock().ok().and_then(|v| {
                    v.get(&node_id)
                        .and_then(|n| n.schema.as_ref().map(|s| s.hash()))
                });
                if let Some(schema) = asm.entry(node_id).or_default().push(chunk) {
                    if have == Some(schema.hash()) {
                        continue;
                    }
                    trackers.insert(node_id, Tracker::new(schema.info.history));
                    if logged.insert(node_id) {
                        let _ = log_tx.send(Record::Schema {
                            node_id,
                            schema: schema.clone(),
                        });
                    }
                    let mut v = station.views.lock().unwrap();
                    let view = v.entry(node_id).or_default();
                    view.schema_chunks = 1;
                    view.schema = Some(schema);
                    view.error = None;
                }
            }
            crate::relay::Relayed::Telemetry(node_id, hash, body) => {
                if !station.is_viewing(node_id) {
                    continue;
                }
                let Some(schema) = station
                    .views
                    .lock()
                    .ok()
                    .and_then(|v| v.get(&node_id).and_then(|n| n.schema.clone()))
                else {
                    continue;
                };
                if hash != schema.hash() {
                    // The board was reflashed; wait for the schema that goes
                    // with these frames rather than reading them wrongly.
                    asm.entry(node_id).or_default().reset();
                    continue;
                }
                let layout = schema.layout(Dir::Telemetry);
                let Ok(frame) = TelemFrame::decode(&layout, &body) else {
                    continue;
                };

                let now_us = station.started.elapsed().as_micros() as u64;
                let newest_node_us = frame.newest().node_time_us;
                let tracker = trackers
                    .entry(node_id)
                    .or_insert_with(|| Tracker::new(schema.info.history));
                for &i in &tracker.offer(&frame).new {
                    let rec = &frame.records[i];
                    let age = newest_node_us.wrapping_sub(rec.node_time_us) as u64;
                    let Ok(payload) = rec.values.encode_payload(&layout) else {
                        continue;
                    };
                    let _ = log_tx.send(Record::Sample {
                        t_us: now_us.saturating_sub(age),
                        node_id,
                        dir: Dir::Telemetry,
                        record: rec.record,
                        payload,
                    });
                }
                if tracker.received + tracker.lost > 2000 {
                    tracker.decay();
                }

                record_history(&station, node_id, &schema, &frame.newest().values);
                let mut v = station.views.lock().unwrap();
                let view = v.entry(node_id).or_default();
                view.record_loss_pct = tracker.delivered_pct().map(|d| 100.0 - d);
                view.records_recovered = tracker.recovered;
                view.node_rate_hz = Some(frame.rate_hz);
                view.node_state = Some(frame.state);
                view.node_flags = frame.flags;
                view.telemetry = frame.records.into_iter().next().map(|r| r.values);
                view.rx_frames += 1;
                view.last_rx_unix_ms = now_ms();
                view.last_rx_ago_ms = 0;
                view.connected = true;
                view.contest = Contest::Alone;
                view.error = None;
            }
        }
    }
}

/// Run one node's link until the task is dropped.
pub async fn node_task(
    station: Arc<Station>,
    cfg: NodeConfig,
    log_tx: mpsc::UnboundedSender<Record>,
) {
    let mut backoff = Duration::from_millis(250);
    loop {
        match run_node(&station, &cfg, &log_tx).await {
            Ok(()) => backoff = Duration::from_millis(250),
            Err(e) => {
                let mut v = station.views.lock().unwrap();
                let view = v.entry(cfg.id).or_default();
                view.connected = false;
                view.error = Some(e.to_string());
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(3));
    }
}

async fn run_node(
    station: &Arc<Station>,
    cfg: &NodeConfig,
    log_tx: &mpsc::UnboundedSender<Record>,
) -> anyhow::Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    let target: SocketAddr = cfg.addr();

    let mut schema: Option<Schema> = None;
    let mut asm = SchemaAssembler::new();
    let mut want_chunk: u16 = 0;
    let mut seq: u16 = 0;
    let mut buf = vec![0u8; 2048];
    let mut logged_schema = false;

    // The rate is re-read from the profile each tick rather than captured
    // once, so changing it in the UI takes effect without tearing the link
    // down and re-negotiating the schema.
    let mut rate_hz = cfg.rate_hz.max(1);
    let mut ticker = tokio::time::interval(Duration::from_micros(1_000_000 / rate_hz as u64));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut last_rx = Instant::now();
    let mut rx_frames = 0u64;
    let mut tx_frames = 0u64;
    let mut schema_reqs = 0u64;
    let mut schema_rx = 0u64;

    let mut link_fault: Option<String> = None;
    let mut contest = ContestWatch::default();

    // Forwarding for the stations that are only watching. A watcher never
    // asks the board for anything, so this is the only way it sees.
    let (relay_on, relay_port) = station
        .profile
        .lock()
        .map(|p| (p.relay, p.relay_port))
        .unwrap_or((true, crate::relay::DEFAULT_PORT));
    let mut relay = match relay_on {
        true => crate::relay::Relay::bind(relay_port, cfg.ip).await.ok(),
        false => None,
    };
    // Which numbered records have arrived. Rebuilt with each schema, since the
    // schema carries how many the node can resend.
    let mut tracker = Tracker::new(1);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match &schema {
                    None => {
                        // Handshake. Ask for the chunk we are missing.
                        let mut pkt = vec![0u8; HEADER_LEN];
                        Header::new(MsgType::SchemaReq, cfg.id, 0, seq).encode(&mut pkt)?;
                        seq = seq.wrapping_add(1);
                        let mut body = [0u8; 2];
                        let n = encode_schema_req(want_chunk, &mut body)?;
                        pkt.extend_from_slice(&body[..n]);
                        if sock.send_to(&pkt, target).await.is_ok() {
                            schema_reqs += 1;
                            if let Ok(mut v) = station.views.lock() {
                                v.entry(cfg.id).or_default().schema_reqs = schema_reqs;
                            }
                        }
                    }
                    Some(s) => {
                        let layout = s.layout(Dir::Control);
                        let viewing = station.is_viewing(cfg.id);
                        // Watching means watching. The board hears nothing
                        // from this station — except an estop, which it only
                        // has to listen to, never answer.
                        if viewing && !station.estop.load(Ordering::Relaxed) {
                            let mut v = station.views.lock().unwrap();
                            let view = v.entry(cfg.id).or_default();
                            view.tx_frames = tx_frames;
                            view.link_fault = None;
                            // Relayed frames are what keep a watched node
                            // alive on screen. If the station driving it
                            // closes, they stop, and this says so rather
                            // than leaving the last reading up for ever.
                            view.last_rx_ago_ms = now_ms().saturating_sub(view.last_rx_unix_ms);
                            view.connected = view.schema.is_some() && view.last_rx_ago_ms < 1500;
                            continue;
                        }
                        let outcome = if viewing {
                            // Nothing of ours goes out while watching, so the
                            // frame carries a zeroed payload the node will
                            // ignore anyway.
                            ControlOutcome {
                                values: Values::zeroed(&layout),
                                source_lost: None,
                            }
                        } else {
                            build_values(station, cfg.id, s)
                        };
                        // A crosslink that has lost its source is a fault for
                        // the node it was driving, not a silent hold.
                        let state = match (&outcome.source_lost, viewing) {
                            // A watcher may stop the machine and nothing else,
                            // so the only state it ever sends is its estop.
                            (_, true) => {
                                if station.estop.load(Ordering::Relaxed) {
                                    State::Estop
                                } else {
                                    State::Disabled
                                }
                            }
                            (Some(_), _) => State::Fault,
                            (None, _) => station.effective_state(cfg.id),
                        };
                        let frame = ControlFrame {
                            host_time_us: micros_u32(station),
                            state,
                            flags: if viewing {
                                microrio_proto::ctrl_flag::VIEW_ONLY
                            } else {
                                0
                            },
                            // One rate per node: the station sends at it and
                            // asks the node to send telemetry at it, so the
                            // log is rectangular at exactly this rate.
                            rate_hz,
                            ack_record: tracker.ack(),
                            values: outcome.values,
                        };
                        let mut pkt = vec![0u8; HEADER_LEN];
                        Header::new(MsgType::Control, cfg.id, s.hash(), seq).encode(&mut pkt)?;
                        seq = seq.wrapping_add(1);
                        let mut body = vec![0u8; ControlFrame::body_len(&layout)];
                        let n = frame.encode(&layout, &mut body)?;
                        pkt.extend_from_slice(&body[..n]);

                        link_fault = outcome.source_lost;

                        if sock.send_to(&pkt, target).await.is_ok() {
                            tx_frames += 1;
                            contest.sent();
                            // A watcher records what the machine did, not the
                            // nothing it sent: logging its own empty control
                            // frames would put a column of zeros beside
                            // somebody else's real commands.
                            if !viewing {
                                let _ = log_tx.send(Record::Sample {
                                    t_us: station.started.elapsed().as_micros() as u64,
                                    node_id: cfg.id,
                                    dir: Dir::Control,
                                    record: 0,
                                    payload: ControlFrame::payload(&body[..n]).to_vec(),
                                });
                            }
                        }
                    }
                }

                {
                    let mut v = station.views.lock().unwrap();
                    let view = v.entry(cfg.id).or_default();
                    view.tx_frames = tx_frames;
                    view.last_rx_ago_ms = last_rx.elapsed().as_millis() as u64;
                    view.connected = schema.is_some() && view.last_rx_ago_ms < 500;
                    view.link_fault = link_fault.clone();
                }

                // Pick up a rate change from the UI. Rebuilding the interval
                // rather than the whole link means the schema stays negotiated.
                let wanted = station
                    .profile
                    .lock()
                    .ok()
                    .and_then(|p| p.nodes.iter().find(|n| n.id == cfg.id).map(|n| n.rate_hz))
                    .unwrap_or(rate_hz)
                    .max(1);
                if wanted != rate_hz {
                    rate_hz = wanted;
                    ticker =
                        tokio::time::interval(Duration::from_micros(1_000_000 / rate_hz as u64));
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                }
            }

            res = sock.recv_from(&mut buf) => {
                let Ok((n, from)) = res else { continue };
                if from.ip() != target.ip() { continue; }

                let Ok((hdr, body)) = Header::decode(&buf[..n]) else { continue };
                if hdr.node_id != cfg.id { continue; }

                match hdr.msg {
                    MsgType::Schema => {
                        let Ok(chunk) = SchemaChunk::decode(body) else { continue };
                        let chunks = chunk.chunk_count;
                        schema_rx += 1;
                        if let Ok(mut v) = station.views.lock() {
                            v.entry(cfg.id).or_default().schema_rx = schema_rx;
                        }
                        if let Some(s) = asm.push(chunk) {
                            if !logged_schema {
                                let _ = log_tx.send(Record::Schema {
                                    node_id: cfg.id,
                                    schema: s.clone(),
                                });
                                logged_schema = true;
                            }
                            let mut v = station.views.lock().unwrap();
                            let view = v.entry(cfg.id).or_default();
                            view.schema = Some(s.clone());
                            view.schema_chunks = chunks;
                            view.error = None;
                            tracker = Tracker::new(s.info.history);
                            schema = Some(s);
                        } else {
                            want_chunk = asm.missing().first().copied().unwrap_or(0);
                        }
                        last_rx = Instant::now();
                    }
                    MsgType::Telem => {
                        let Some(s) = &schema else { continue };
                        // A changed hash means the board was reflashed: start over.
                        if hdr.schema_hash != s.hash() {
                            schema = None;
                            asm.reset();
                            want_chunk = 0;
                            logged_schema = false;
                            // Channel names and slots have changed underneath
                            // us; the old trace no longer means anything.
                            station.history.lock().unwrap().clear_node(cfg.id);
                            continue;
                        }
                        let layout = s.layout(Dir::Telemetry);
                        let Ok(frame) = TelemFrame::decode(&layout, body) else { continue };

                        rx_frames += 1;
                        last_rx = Instant::now();
                        let rx_unix_ms = now_ms();

                        // Log every record that is new to us. A resent one goes
                        // in at the time it was taken, not the time it arrived:
                        // its age on the node's clock is subtracted from now.
                        if !station.is_viewing(cfg.id) {
                            if let Some(r) = relay.as_mut() {
                                r.frame(&buf[..n]).await;
                                r.schema(cfg.id, s).await;
                            }
                        }

                        let now_us = station.started.elapsed().as_micros() as u64;
                        let newest_node_us = frame.newest().node_time_us;
                        let accepted = tracker.offer(&frame);
                        for &i in &accepted.new {
                            let rec = &frame.records[i];
                            let age = newest_node_us.wrapping_sub(rec.node_time_us) as u64;
                            let Ok(payload) = rec.values.encode_payload(&layout) else {
                                continue;
                            };
                            let _ = log_tx.send(Record::Sample {
                                t_us: now_us.saturating_sub(age),
                                node_id: cfg.id,
                                dir: Dir::Telemetry,
                                record: rec.record,
                                payload,
                            });
                        }
                        if tracker.received + tracker.lost > 2000 {
                            tracker.decay();
                        }

                        let mut v = station.views.lock().unwrap();
                        let mut view = v.entry(cfg.id).or_default();
                        record_history(station, cfg.id, s, &frame.newest().values);

                        // Only trust an echo the node got from us: before our
                        // first control frame lands it may still be echoing a
                        // previous station's clock.
                        let ours = frame.rtt_us_checked(micros_u32(station), 1_000_000);
                        if let Some(d) = ours {
                            view.rtt_us = Some(d);
                        }
                        view.contest = contest.frame(frame.flags, ours.is_some());
                        // Refused: this node belongs to another station. Drop
                        // to watching rather than carrying on shouting at it,
                        // which is both useless and a second source of noise
                        // on the link for whoever is actually driving.
                        if view.contest == Contest::Refused && !station.is_viewing(cfg.id) {
                            drop(v);
                            station.set_viewing(cfg.id, true);
                            v = station.views.lock().unwrap();
                            view = v.entry(cfg.id).or_default();
                            view.contest = Contest::Refused;
                        }

                        view.record_loss_pct = tracker.delivered_pct().map(|d| 100.0 - d);
                        view.records_recovered = tracker.recovered;
                        view.node_rate_hz = Some(frame.rate_hz);
                        view.node_state = Some(frame.state);
                        view.node_flags = frame.flags;
                        view.telemetry = frame.records.into_iter().next().map(|r| r.values);
                        view.rx_frames = rx_frames;
                        view.last_rx_unix_ms = rx_unix_ms;
                        view.connected = true;
                        view.error = None;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Push this frame's numeric channels into the plot history.
///
/// Bools go in as 0/1 so a digital line can share an axis with an analogue one,
/// which is how the original scope drew them. Strings have nothing to plot.
fn record_history(station: &Arc<Station>, node_id: u16, schema: &Schema, values: &Values) {
    let t = station.started.elapsed().as_secs_f64();
    let mut hist = station.history.lock().unwrap();
    for c in schema.channels.iter().filter(|c| c.dir == Dir::Telemetry) {
        let i = c.slot as usize;
        let v = match c.ctype {
            microrio_proto::ChanType::Bool => {
                values.bools.get(i).copied().unwrap_or(false) as u8 as f64
            }
            microrio_proto::ChanType::I32 => values.i32s.get(i).copied().unwrap_or(0) as f64,
            microrio_proto::ChanType::F32 => values.f32s.get(i).copied().unwrap_or(0.0) as f64,
            _ => continue,
        };
        hist.push(node_id, &c.name, t, v);
    }
}

fn micros_u32(station: &Arc<Station>) -> u32 {
    station.started.elapsed().as_micros() as u32
}

/// What one tick of control works out to.
pub struct ControlOutcome {
    pub values: Values,
    /// A crosslink meant to be driving this node, whose source has gone quiet.
    pub source_lost: Option<String>,
}

/// A crosslink source older than this is not a source any more.
///
/// Ten frames at the default rate. Long enough to ride out a dropped packet,
/// short enough that a dead board is noticed before it matters.
const CROSSLINK_STALE_MS: u64 = 200;

/// Turn the UI's, joystick's and crosslinks' intents into a full control payload.
///
/// Every frame carries complete state, so anything the operator has not touched
/// goes out at its default rather than being omitted.
///
/// Numeric channels pass through their `ControlConfig` on the way out: offset
/// applied, then clamped. That transform belongs here rather than in the UI,
/// because the joystick sets intents too and both sources must leave the
/// station through the same gate.
pub fn build_values(station: &Arc<Station>, node_id: u16, schema: &Schema) -> ControlOutcome {
    let layout = schema.layout(Dir::Control);
    let mut values = Values::zeroed(&layout);

    // Snapshot what this node needs and let the profile lock go. The crosslink
    // pass below takes the views lock, and holding two of these at once is how
    // a stuck UI thread turns into a stopped control loop.
    let (controls, crosslinks) = {
        let Ok(p) = station.profile.lock() else {
            return ControlOutcome {
                values,
                source_lost: None,
            };
        };
        let controls = p
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .map(|n| n.controls.clone())
            .unwrap_or_default();
        let crosslinks: Vec<CrossLink> = p
            .crosslinks
            .iter()
            .filter(|l| l.active && l.to_node == node_id)
            .cloned()
            .collect();
        (controls, crosslinks)
    };

    for c in schema.channels.iter().filter(|c| c.dir == Dir::Control) {
        // The same transform, and the same defaults, the control grid shows
        // in its `wire` column. If these two ever disagree the grid is lying
        // about what is being sent, which is how this was wrong before.
        let ccfg = controls
            .get(&c.name)
            .cloned()
            .unwrap_or_else(|| crate::config::ControlConfig::default_for(c.ctype));
        let (Some(intent), _) = station.resolve(node_id, c, &ccfg) else {
            continue;
        };
        let i = c.slot as usize;
        match intent {
            Intent::Bool(v) => {
                if let Some(s) = values.bools.get_mut(i) {
                    *s = v;
                }
            }
            Intent::I32(v) => {
                if let Some(s) = values.i32s.get_mut(i) {
                    *s = ccfg.on_the_wire(v as f64).round() as i32;
                }
            }
            Intent::F32(v) => {
                if let Some(s) = values.f32s.get_mut(i) {
                    *s = ccfg.on_the_wire(v as f64) as f32;
                }
            }
            Intent::Cmd(v) => {
                if let Some(s) = values.cmds.get_mut(i) {
                    *s = v;
                }
            }
        }
    }

    // Crosslinks run last, so a channel wired to another board's telemetry
    // wins over whatever the operator last left in the box for it.
    let mut source_lost = None;
    if !crosslinks.is_empty() {
        let now = now_ms();
        let views = station.views.lock().unwrap();
        for l in &crosslinks {
            let Some(dest) = schema
                .channels
                .iter()
                .find(|c| c.dir == Dir::Control && c.name == l.to_channel)
            else {
                continue; // the destination node does not declare that channel
            };
            match crosslink_source(&views, l, now) {
                Some(raw) => {
                    let v = l.apply(raw);
                    let i = dest.slot as usize;
                    match dest.ctype {
                        microrio_proto::ChanType::Bool => {
                            if let Some(s) = values.bools.get_mut(i) {
                                *s = v != 0.0;
                            }
                        }
                        microrio_proto::ChanType::I32 => {
                            if let Some(s) = values.i32s.get_mut(i) {
                                *s = v.round() as i32;
                            }
                        }
                        microrio_proto::ChanType::F32 => {
                            if let Some(s) = values.f32s.get_mut(i) {
                                *s = v as f32;
                            }
                        }
                        _ => {}
                    }
                }
                None => {
                    // Holding the last value would mean a dead sensor keeps
                    // commanding an output, which is the failure this whole
                    // design exists to avoid. Fault instead.
                    source_lost = Some(format!(
                        "{} on node {} (drives {})",
                        l.from_channel, l.from_node, l.to_channel
                    ));
                }
            }
        }
    }

    ControlOutcome {
        values,
        source_lost,
    }
}

/// The current value of a crosslink's source channel, or `None` if there is
/// not a fresh one. For the UI, which has no clock of its own to pass in.
pub fn crosslink_value(views: &HashMap<u16, NodeView>, link: &CrossLink) -> Option<f64> {
    crosslink_source(views, link, now_ms())
}

/// The current value of a crosslink's source channel, or `None` if there is
/// not a fresh one.
fn crosslink_source(views: &HashMap<u16, NodeView>, link: &CrossLink, now_ms_: u64) -> Option<f64> {
    let view = views.get(&link.from_node)?;
    if now_ms_.saturating_sub(view.last_rx_unix_ms) > CROSSLINK_STALE_MS {
        return None;
    }
    let schema = view.schema.as_ref()?;
    let values = view.telemetry.as_ref()?;
    let c = schema
        .channels
        .iter()
        .find(|c| c.dir == Dir::Telemetry && c.name == link.from_channel)?;
    let i = c.slot as usize;
    match c.ctype {
        microrio_proto::ChanType::Bool => values.bools.get(i).map(|b| *b as u8 as f64),
        microrio_proto::ChanType::I32 => values.i32s.get(i).map(|v| *v as f64),
        microrio_proto::ChanType::F32 => values.f32s.get(i).map(|v| *v as f64),
        _ => None,
    }
}
