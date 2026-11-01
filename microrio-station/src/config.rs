//! Saved configuration: station profile, per-channel display settings, joystick
//! mappings. TOML, human readable, copyable between machines.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub nodes: Vec<NodeConfig>,
    #[serde(default)]
    pub joystick: JoystickConfig,
    #[serde(default)]
    pub logging: LogConfig,
    /// Station faults if the UI has not rendered within this many milliseconds.
    #[serde(default = "default_ui_stale_ms")]
    pub ui_stale_ms: u64,
    /// Which time zone every displayed and logged timestamp is rendered in.
    /// `"system"` follows the machine; anything else is an IANA name such as
    /// `"America/Los_Angeles"`. The wire stays unix seconds either way.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// The chart on the Operator page. Its own chart, not one of the Charts
    /// tab's, so changing one never changes the other.
    #[serde(default)]
    pub operator_chart: Option<ChartConfig>,
    /// The Charts tab.
    #[serde(default)]
    pub charts: Vec<ChartConfig>,
    /// Telemetry on one node driving a control channel on another.
    #[serde(default)]
    pub crosslinks: Vec<CrossLink>,
    /// Where this station relays to, and listens on, for stations that are
    /// watching rather than driving. Same number on every station that
    /// should see each other.
    #[serde(default = "default_relay_port")]
    pub relay_port: u16,
    /// Forward telemetry for the nodes this station drives, so other
    /// stations can watch them. Costs one broadcast packet per frame.
    #[serde(default = "yes")]
    pub relay: bool,
}

/// Where stations relay telemetry to each other. Nothing else on a network
/// uses it, and nothing carried on it is a command.
pub const DEFAULT_RELAY_PORT: u16 = 5010;

fn default_relay_port() -> u16 {
    DEFAULT_RELAY_PORT
}

fn default_ui_stale_ms() -> u64 {
    microrio_proto::DEFAULT_UI_STALE_MS as u64
}

fn default_timezone() -> String {
    "system".into()
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            nodes: vec![NodeConfig::default()],
            joystick: JoystickConfig::default(),
            logging: LogConfig::default(),
            ui_stale_ms: default_ui_stale_ms(),
            timezone: default_timezone(),
            operator_chart: None,
            charts: Vec::new(),
            crosslinks: Vec::new(),
            relay_port: default_relay_port(),
            relay: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub id: u16,
    pub label: String,
    pub ip: Ipv4Addr,
    #[serde(default = "default_node_port")]
    pub port: u16,
    #[serde(default = "default_rate")]
    pub rate_hz: u16,
    /// Whether the station opens a link to this node at all. This is not the
    /// enable switch an operator uses — that is per node and lives at runtime,
    /// because a saved file must never decide what is live on startup.
    #[serde(default = "yes", alias = "enabled")]
    pub link_active: bool,
    /// Per-channel display settings for telemetry, keyed by channel name.
    #[serde(default)]
    pub channels: HashMap<String, ChannelConfig>,
    /// Per-channel limits for numeric controls, keyed by channel name.
    #[serde(default)]
    pub controls: HashMap<String, ControlConfig>,
    /// Channels shown in this node's header row, in order. At most
    /// [`MAX_HEADER_ITEMS`] counting pinned groups, so the row still reads at
    /// half-screen width.
    #[serde(default)]
    pub pins: Vec<ChanRef>,
    /// Named groups of channels on the Operator page, in display order.
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Node-to-station watchdog: no telemetry for this long and the header
    /// says comms are lost. An alarm for the operator, nothing more.
    #[serde(default = "default_comms_watchdog_ms")]
    pub watchdog_ms: u64,
}

fn default_comms_watchdog_ms() -> u64 {
    250
}

/// Control channels that exist for the Clock tab rather than the operator.
pub const CLOCK_CONTROLS: [&str; 2] = ["set_unix", "sync_time"];

fn default_node_port() -> u16 {
    5005
}
fn default_rate() -> u16 {
    microrio_proto::DEFAULT_RATE_HZ
}
fn yes() -> bool {
    true
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            id: 1,
            label: "node-1".into(),
            ip: Ipv4Addr::new(192, 168, 0, 50),
            port: default_node_port(),
            rate_hz: default_rate(),
            link_active: true,
            channels: HashMap::new(),
            controls: HashMap::new(),
            pins: Vec::new(),
            groups: Vec::new(),
            watchdog_ms: default_comms_watchdog_ms(),
        }
    }
}

impl NodeConfig {
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(self.ip), self.port)
    }
}

/// Calibration and presentation live here, not in firmware: changing a scale
/// factor is a config edit rather than a reflash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelConfig {
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub units: String,
    #[serde(default = "one")]
    pub slope: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub plot: bool,
    /// Value-to-label table for an i32 channel. Profile only: the wire and the
    /// log still carry the integer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<ValueLabel>,
    /// Warn, never fault, when the scaled value goes below this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_below: Option<f64>,
    /// Warn, never fault, when the scaled value goes above this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_above: Option<f64>,
    /// The control channel this telemetry reports back. The control row then
    /// confirms the echo, or warns when it disagrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo_of: Option<String>,
    /// Keep this channel out of the grids and groups. An echo starts hidden,
    /// since its control already shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Show a bool the other way up: the lamp lights when the channel reads
    /// false. For a signal that is normally true, where false is the thing
    /// worth seeing. The wire and the log are untouched.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert_display: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn one() -> f64 {
    1.0
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            units: String::new(),
            slope: 1.0,
            offset: 0.0,
            plot: false,
            labels: Vec::new(),
            warn_below: None,
            warn_above: None,
            echo_of: None,
            hidden: None,
            invert_display: false,
        }
    }
}

impl ChannelConfig {
    pub fn scale(&self, raw: f64) -> f64 {
        raw * self.slope + self.offset
    }

    /// Whether a bool's lamp should be lit, given what the node sent.
    ///
    /// Normally that is the value itself. An inverted channel lights on
    /// false, so that on a panel of lamps anything lit means look at me,
    /// whichever way round the signal happens to be defined.
    pub fn lamp(&self, raw: bool) -> bool {
        raw != self.invert_display
    }

    /// Whether a scaled value is outside its warning band. Warning only:
    /// nothing in the station acts on it.
    pub fn warns(&self, scaled: f64) -> bool {
        self.warn_below.is_some_and(|lo| scaled < lo)
            || self.warn_above.is_some_and(|hi| scaled > hi)
    }

    /// What the operator calls this channel.
    pub fn title<'a>(&'a self, name: &'a str) -> &'a str {
        if self.display_name.trim().is_empty() {
            name
        } else {
            &self.display_name
        }
    }
}

/// One row of a value-to-label table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueLabel {
    pub value: i32,
    pub label: String,
    /// `#rrggbb`, or absent for the ordinary text colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
}

impl ValueLabel {
    /// The colour as RGB, if one is set and it parses.
    pub fn rgb(&self) -> Option<[u8; 3]> {
        let hex = self.colour.as_deref()?.trim().trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
    }
}

/// The label for a value, or `None` when the table does not cover it. An
/// unmapped value is shown as its number, never as a blank.
pub fn label_for(table: &[ValueLabel], value: i32) -> Option<&ValueLabel> {
    table.iter().find(|l| l.value == value)
}

/// Text to show for a value under a table: the label if there is one, else
/// the number itself.
pub fn label_text(table: &[ValueLabel], value: i32) -> String {
    match label_for(table, value) {
        Some(l) if !l.label.is_empty() => l.label.clone(),
        _ => value.to_string(),
    }
}

// ------------------------------------------------------- header and groups

/// Pinned items per node header, pins and pinned groups together.
pub const MAX_HEADER_ITEMS: usize = 5;

/// A channel on one node, by direction and name. Names are only unique within
/// a direction, so the direction is part of the identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChanRef {
    #[serde(with = "dir_serde")]
    pub dir: microrio_proto::Dir,
    pub channel: String,
}

impl ChanRef {
    pub fn telemetry(channel: impl Into<String>) -> Self {
        Self {
            dir: microrio_proto::Dir::Telemetry,
            channel: channel.into(),
        }
    }
    pub fn control(channel: impl Into<String>) -> Self {
        Self {
            dir: microrio_proto::Dir::Control,
            channel: channel.into(),
        }
    }
}

mod dir_serde {
    use microrio_proto::Dir;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Dir, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match d {
            Dir::Telemetry => "telemetry",
            Dir::Control => "control",
        })
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Dir, D::Error> {
        match String::deserialize(d)?.as_str() {
            "telemetry" => Ok(Dir::Telemetry),
            "control" => Ok(Dir::Control),
            other => Err(serde::de::Error::custom(format!(
                "direction must be telemetry or control, not {other:?}"
            ))),
        }
    }
}

/// A named set of channels shown together, such as everything about one valve.
///
/// Telemetry may sit in as many groups as it is useful in. A control channel
/// belongs to one group at most, so there is exactly one place to set it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub members: Vec<ChanRef>,
    #[serde(default)]
    pub collapsed: bool,
    /// Shown in the node header as one item.
    #[serde(default)]
    pub pinned: bool,
}

impl Group {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            members: Vec::new(),
            collapsed: false,
            pinned: false,
        }
    }
}

impl NodeConfig {
    /// Put a channel into a group at `at` (clamped), or at the end.
    ///
    /// Within one group this is a reorder. A control channel already in a
    /// different group is moved out of it; telemetry is copied, since showing
    /// one reading in two places is harmless and often wanted.
    pub fn place_in_group(&mut self, group: usize, chan: ChanRef, at: Option<usize>) {
        if group >= self.groups.len() {
            return;
        }
        let mut at = at;
        // Out of the target first, keeping the insertion point stable when
        // the channel was above it.
        if let Some(pos) = self.groups[group].members.iter().position(|m| *m == chan) {
            self.groups[group].members.remove(pos);
            if let Some(a) = at.as_mut() {
                if pos < *a {
                    *a -= 1;
                }
            }
        }
        if chan.dir == microrio_proto::Dir::Control {
            for (i, g) in self.groups.iter_mut().enumerate() {
                if i != group {
                    g.members.retain(|m| *m != chan);
                }
            }
        }
        let members = &mut self.groups[group].members;
        let at = at.unwrap_or(members.len()).min(members.len());
        members.insert(at, chan);
    }

    pub fn remove_from_group(&mut self, group: usize, chan: &ChanRef) {
        if let Some(g) = self.groups.get_mut(group) {
            g.members.retain(|m| m != chan);
        }
    }

    /// Whether a channel is kept out of the grids and groups. An explicit
    /// setting wins; otherwise the clock's plumbing and any echo start hidden.
    pub fn is_hidden(&self, chan: &ChanRef) -> bool {
        match chan.dir {
            microrio_proto::Dir::Telemetry => match self.channels.get(&chan.channel) {
                Some(c) => c.hidden.unwrap_or(c.echo_of.is_some()),
                None => false,
            },
            microrio_proto::Dir::Control => self
                .controls
                .get(&chan.channel)
                .and_then(|c| c.hidden)
                .unwrap_or(CLOCK_CONTROLS.contains(&chan.channel.as_str())),
        }
    }

    /// The telemetry channel that echoes a control, if one is set.
    pub fn echo_for(&self, control: &str) -> Option<&str> {
        self.channels
            .iter()
            .find(|(_, c)| c.echo_of.as_deref() == Some(control))
            .map(|(name, _)| name.as_str())
    }

    /// Whether a channel appears in any group.
    pub fn grouped(&self, chan: &ChanRef) -> bool {
        self.groups.iter().any(|g| g.members.contains(chan))
    }

    /// How many header slots are in use.
    pub fn header_items(&self) -> usize {
        self.pins.len() + self.groups.iter().filter(|g| g.pinned).count()
    }

    /// Pin or unpin a channel. Pinning past the cap is refused, and says so.
    pub fn set_pinned(&mut self, chan: &ChanRef, on: bool) -> bool {
        let pinned = self.pins.contains(chan);
        if on == pinned {
            return true;
        }
        if on {
            if self.header_items() >= MAX_HEADER_ITEMS {
                return false;
            }
            self.pins.push(chan.clone());
        } else {
            self.pins.retain(|p| p != chan);
        }
        true
    }

    /// Pin or unpin a group, under the same cap as channels.
    pub fn set_group_pinned(&mut self, group: usize, on: bool) -> bool {
        let Some(g) = self.groups.get(group) else {
            return false;
        };
        if g.pinned == on {
            return true;
        }
        if on && self.header_items() >= MAX_HEADER_ITEMS {
            return false;
        }
        self.groups[group].pinned = on;
        true
    }
}

/// Limits for a numeric output, carried over from the 2026 control grid:
/// a value with a step, clamped between a minimum and a maximum, nudged by
/// increment buttons either side of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlConfig {
    #[serde(default = "default_step")]
    pub step: f64,
    #[serde(default = "default_min")]
    pub min: f64,
    #[serde(default = "default_max")]
    pub max: f64,
    /// Added to the value before it goes on the wire, so a channel can be
    /// trimmed without touching the firmware.
    #[serde(default)]
    pub offset: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub units: String,
    /// For an i32 control: the values it may take, by name. The operator
    /// picks from a dropdown instead of typing a number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<ValueLabel>,
    /// Joystick shaping, 0..1. Softens the start of the travel and still
    /// reaches full scale. Only a joystick input passes through it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expo: f64,
    /// Keep this channel out of the grids and groups. The clock's plumbing
    /// channels start hidden; anything can be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn default_step() -> f64 {
    0.05
}
fn default_min() -> f64 {
    -1.0
}
fn default_max() -> f64 {
    1.0
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            step: default_step(),
            min: default_min(),
            max: default_max(),
            offset: 0.0,
            display_name: String::new(),
            units: String::new(),
            labels: Vec::new(),
            expo: 0.0,
            hidden: None,
        }
    }
}

impl ControlConfig {
    /// Defaults per type, and they are enforced on the wire, so they must
    /// not clip anything legitimate.
    ///
    /// An i32 therefore defaults to the whole i32 range rather than a tidy
    /// round number: `set_unix` carries a unix timestamp of about 1.8e9, and
    /// a default ceiling of ten thousand would quietly turn clock sync into
    /// sending the year 1970. Bounds on an integer channel are something the
    /// operator opts into. A float keeps -1..1, which is what a normalised
    /// output is and what the grid has always enforced on typed values.
    pub fn default_for(ctype: microrio_proto::ChanType) -> Self {
        match ctype {
            microrio_proto::ChanType::I32 => Self {
                step: 1.0,
                min: i32::MIN as f64,
                max: i32::MAX as f64,
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    pub fn clamp(&self, v: f64) -> f64 {
        v.clamp(self.min.min(self.max), self.max.max(self.min))
    }

    pub fn title<'a>(&'a self, name: &'a str) -> &'a str {
        if self.display_name.trim().is_empty() {
            name
        } else {
            &self.display_name
        }
    }

    /// A joystick deflection to a channel value: expo, then spread over
    /// min..max. A stick's centre lands mid-range; a trigger at rest lands on
    /// min. Offset and clamping come after, in [`on_the_wire`](Self::on_the_wire),
    /// as they do for a typed value.
    pub fn from_stick(&self, deflection: f64, unipolar: bool) -> f64 {
        let e = self.expo.clamp(0.0, 1.0);
        // Soft near rest, unchanged at full scale: f(0)=0, f(1)=1, f'(0)=1-e.
        let shape = |x: f64| (1.0 - e) * x + e * x * x * x;
        let t = if unipolar {
            shape(deflection.clamp(0.0, 1.0))
        } else {
            let d = deflection.clamp(-1.0, 1.0);
            (d.signum() * shape(d.abs()) + 1.0) / 2.0
        };
        self.min + t * (self.max - self.min)
    }

    /// The value actually sent: offset applied, then clamped.
    pub fn on_the_wire(&self, v: f64) -> f64 {
        self.clamp(v + self.offset)
    }
}

// ------------------------------------------------------------------ charts

/// What one axis of a chart is showing.
///
/// Axes are symmetric on purpose: either one can be time or a channel, so an
/// X-Y plot is the same machinery as a scope trace rather than a special
/// case. Channels name a node as well as a channel, so a chart can draw two
/// boards against each other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Axis {
    #[serde(default)]
    pub kind: AxisKind,
    /// Only meaningful when `kind` is `Channel`.
    #[serde(default)]
    pub node_id: u16,
    #[serde(default)]
    pub channel: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AxisKind {
    /// Seconds before now; the newest sample sits at 0 and the trace scrolls left.
    #[default]
    Elapsed,
    /// Wall-clock time in the profile's zone.
    Clock,
    /// A telemetry channel, giving an X-Y plot rather than a time series.
    Channel,
}

impl Axis {
    pub fn elapsed() -> Self {
        Self {
            kind: AxisKind::Elapsed,
            node_id: 0,
            channel: String::new(),
        }
    }

    pub fn channel(node_id: u16, channel: impl Into<String>) -> Self {
        Self {
            kind: AxisKind::Channel,
            node_id,
            channel: channel.into(),
        }
    }

    pub fn is_time(&self) -> bool {
        !matches!(self.kind, AxisKind::Channel)
    }

    /// What the axis is called on screen and in the legend.
    pub fn label(&self) -> String {
        match self.kind {
            AxisKind::Elapsed => "seconds ago".into(),
            AxisKind::Clock => "time".into(),
            AxisKind::Channel => format!("n{}·{}", self.node_id, self.channel),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartConfig {
    #[serde(default)]
    pub title: String,
    #[serde(default = "Axis::elapsed")]
    pub x: Axis,
    #[serde(default)]
    pub y: Vec<Axis>,
    /// How much history the chart draws, in seconds.
    #[serde(default = "default_window_s")]
    pub window_s: f64,
    #[serde(default = "yes")]
    pub autoscale: bool,
    #[serde(default = "neg_one_f64")]
    pub y_min: f64,
    #[serde(default = "one")]
    pub y_max: f64,
    /// Draw Y downwards: larger values lower. Depth, descent rate, anything
    /// measured from a datum downwards reads the right way up this way.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert_y: bool,
}

/// Five minutes: the span a session is usually watched over.
fn default_window_s() -> f64 {
    300.0
}
fn neg_one_f64() -> f64 {
    -1.0
}

impl Default for ChartConfig {
    fn default() -> Self {
        Self {
            title: "chart".into(),
            x: Axis::elapsed(),
            y: Vec::new(),
            window_s: default_window_s(),
            autoscale: true,
            y_min: -1.0,
            y_max: 1.0,
            invert_y: false,
        }
    }
}

// -------------------------------------------------------------- crosslinks

/// One node's telemetry driving another node's control channel.
///
/// The value is scaled and clamped on the way across, so a sensor reading in
/// counts can drive an output in whatever units that output wants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossLink {
    #[serde(default = "yes")]
    pub active: bool,
    pub from_node: u16,
    pub from_channel: String,
    pub to_node: u16,
    pub to_channel: String,
    #[serde(default = "one")]
    pub slope: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default = "neg_one_f64")]
    pub min: f64,
    #[serde(default = "one")]
    pub max: f64,
}

impl Default for CrossLink {
    fn default() -> Self {
        Self {
            active: true,
            from_node: 1,
            from_channel: String::new(),
            to_node: 1,
            to_channel: String::new(),
            slope: 1.0,
            offset: 0.0,
            min: -1.0,
            max: 1.0,
        }
    }
}

impl CrossLink {
    pub fn apply(&self, raw: f64) -> f64 {
        (raw * self.slope + self.offset).clamp(self.min.min(self.max), self.max.max(self.min))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JoystickConfig {
    /// Per-device settings, keyed by a stable device name.
    #[serde(default)]
    pub devices: HashMap<String, DeviceConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Friendly names for axes and buttons, keyed by the gilrs code.
    #[serde(default)]
    pub names: HashMap<String, String>,
    #[serde(default)]
    pub mappings: Vec<Mapping>,
}

/// One axis or button driving one control channel on one node.
///
/// A mapping only routes and cleans up the input: invert, trim, deadband. What
/// the channel does with it — expo, range, offset, limits — belongs to the
/// channel, in the control grid, so a value from the stick and a value typed
/// in go out through exactly the same steps. Older profiles carried min, max
/// and expo here; they are ignored on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    /// gilrs axis or button code, as shown in the joystick panel.
    pub source: String,
    pub node_id: u16,
    pub channel: String,
    #[serde(default)]
    pub trim: f32,
    #[serde(default)]
    pub deadband: f32,
    #[serde(default)]
    pub invert: bool,
}

impl Mapping {
    /// Raw input to a cleaned deflection.
    ///
    /// A stick is `-1..1` about its centre. An analogue trigger, or a button,
    /// is `0..1` from rest — `unipolar` — and inverting one means fully
    /// pressed reads as rest, not as negative.
    pub fn deflection(&self, raw: f32, unipolar: bool) -> f32 {
        if unipolar {
            let mut v = if self.invert { 1.0 - raw } else { raw };
            v += self.trim;
            if v < self.deadband {
                return 0.0;
            }
            if self.deadband < 1.0 {
                v = (v - self.deadband) / (1.0 - self.deadband);
            }
            return v.clamp(0.0, 1.0);
        }

        let mut v = if self.invert { -raw } else { raw };
        v += self.trim;
        if v.abs() < self.deadband {
            v = 0.0;
        } else if self.deadband < 1.0 {
            // Re-expand so the first live position past the deadband is not a jump.
            v = v.signum() * ((v.abs() - self.deadband) / (1.0 - self.deadband));
        }
        v.clamp(-1.0, 1.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub dir: Option<PathBuf>,
    /// Where the last CSV was saved, so the save dialog opens there again.
    #[serde(default)]
    pub last_export_dir: Option<PathBuf>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: None,
            last_export_dir: None,
        }
    }
}

impl LogConfig {
    pub fn resolved_dir(&self) -> PathBuf {
        self.dir.clone().unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("microrio")
                .join("logs")
        })
    }
}

/// Where the profile lives when the user did not say.
pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("microrio")
        .join("station.toml")
}

pub fn load(path: &Path) -> anyhow::Result<Profile> {
    if !path.exists() {
        let p = Profile::default();
        save(path, &p)?;
        return Ok(p);
    }
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

pub fn save(path: &Path, profile: &Profile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml::to_string_pretty(profile)?)?;
    Ok(())
}
