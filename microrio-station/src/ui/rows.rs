//! One channel's row, shared by the grids and the groups.
//!
//! A channel looks and behaves the same wherever it is shown: settings on the
//! left, the live value on the right beside its raw or wire form. A group is
//! where an operator works, so it carries the whole row rather than a
//! summary, and its buttons are bigger.

use std::time::{Duration, Instant};

use egui::{Color32, RichText};
use microrio_proto::schema::{ChannelDef, Schema};
use microrio_proto::{ChanType, Dir};

use super::groups::labelled;
use super::{fmt_scaled, plottable, warned, App};
use crate::config::{self, Axis, AxisKind, ControlConfig, NodeConfig};
use crate::station::{Intent, NodeView, Source};
use crate::theme;

/// How long a control may disagree with its echo before it is flagged. Long
/// enough for a round trip and a board's loop; short enough to notice.
const ECHO_GRACE: Duration = Duration::from_millis(500);

/// Column count shared by every row, so telemetry and control rows line up
/// in a group: the live value is always the second-to-last column.
pub(super) const ROW_COLUMNS: usize = 8;

/// Grid headers for each direction, in row order.
pub(super) const TELEM_HEADERS: [&str; ROW_COLUMNS] = [
    "chan", "units", "slope", "offset", "plot", "", "value", "raw",
];
pub(super) const CTRL_HEADERS: [&str; ROW_COLUMNS] = [
    "chan", "step", "min", "max", "offset", "expo", "value", "wire",
];

/// Size of the things an operator clicks, by where they are.
#[derive(Clone, Copy)]
pub(super) struct Style {
    /// Groups label their settings inline, since they have no header row.
    pub inline: bool,
    pub button: egui::Vec2,
    pub text: f32,
    pub lamp: f32,
}

impl Style {
    pub fn grid() -> Self {
        Self {
            inline: false,
            button: egui::vec2(18.0, 18.0),
            text: 13.0,
            lamp: 14.0,
        }
    }
    pub fn group() -> Self {
        Self {
            inline: true,
            button: egui::vec2(34.0, 28.0),
            text: 17.0,
            lamp: 22.0,
        }
    }
}

/// A numeric cell. Typed into, never dragged: these sit inches from the
/// value being driven, and a limit nudged by a stray drag is a real hazard.
fn drag<'a>(v: &'a mut f64, speed: f64, prefix: &str, style: Style) -> egui::DragValue<'a> {
    let d = super::num(v, speed);
    if style.inline {
        d.prefix(format!("{prefix} "))
    } else {
        d
    }
}

impl App {
    /// The cells of a telemetry row after its name.
    pub(super) fn telemetry_cells(
        &mut self,
        ui: &mut egui::Ui,
        cfg: &NodeConfig,
        view: &NodeView,
        c: &ChannelDef,
        on_chart: bool,
        style: Style,
    ) {
        let mut ccfg = cfg.channels.get(&c.name).cloned().unwrap_or_default();
        let before = ccfg.clone();
        let numeric = matches!(c.ctype, ChanType::I32 | ChanType::F32);

        // Units are a label, not a box: there is one thing in this row an
        // operator sets by eye and it is not the name of a unit. It is named
        // in Settings, with the display name, where it is checked once.
        ui.label(theme::value_muted(&ccfg.units));
        // Calibration stays live, because it is judged against a reading.
        if numeric {
            ui.add(drag(&mut ccfg.slope, 0.01, "×", style))
                .on_hover_text("slope: scaled = raw × slope + offset");
            ui.add(drag(&mut ccfg.offset, 0.01, "+", style))
                .on_hover_text("offset");
        } else {
            ui.label("");
            ui.label("");
        }
        if plottable(c.ctype) {
            let mut on = on_chart;
            let label = if style.inline { "plot" } else { "" };
            if ui
                .checkbox(&mut on, label)
                .on_hover_text("Plot this channel on the Operator chart")
                .changed()
            {
                self.toggle_operator_plot(cfg.id, &c.name);
            }
        } else {
            ui.label("");
        }
        ui.label("");

        let i = c.slot as usize;
        let Some(values) = &view.telemetry else {
            ui.label(theme::value_muted("—"));
            ui.label(theme::value_muted("—"));
            self.store_channel(cfg.id, &c.name, &before, ccfg);
            return;
        };
        match c.ctype {
            ChanType::Bool => {
                let raw = values.bools.get(i).copied().unwrap_or(false);
                // A channel that is normally true — `rtc_ok`, `healthy` — is
                // worth showing the other way up, so a lit lamp always means
                // "look at me" rather than "all is well".
                let lit = ccfg.lamp(raw);
                ui.horizontal(|ui| {
                    theme::lamp(ui, lit, style.lamp);
                    ui.label(
                        RichText::new(if lit { "ON" } else { "off" })
                            .monospace()
                            .size(style.text)
                            .color(if lit { theme::TEXT } else { theme::MUTED }),
                    );
                });
                ui.label(theme::value_muted(if raw { "true" } else { "false" }))
                    .on_hover_text(if ccfg.invert_display {
                        "What the node actually sent. The lamp on this channel is inverted."
                    } else {
                        "What the node actually sent."
                    });
            }
            ChanType::I32 => {
                let raw = values.i32s.get(i).copied().unwrap_or(0);
                let scaled = ccfg.scale(raw as f64);
                if config::label_for(&ccfg.labels, raw).is_some() {
                    // A status, not a measurement: show the word.
                    ui.label(labelled(&ccfg.labels, raw).size(style.text));
                } else {
                    ui.label(warned(
                        theme::value(fmt_scaled(scaled, &ccfg)).size(style.text),
                        ccfg.warns(scaled),
                    ));
                }
                ui.label(theme::value_muted(format!("{raw}")));
            }
            ChanType::F32 => {
                let raw = values.f32s.get(i).copied().unwrap_or(0.0) as f64;
                let scaled = ccfg.scale(raw);
                ui.label(warned(
                    theme::value(format!("{scaled:.4}")).size(style.text),
                    ccfg.warns(scaled),
                ));
                ui.label(theme::value_muted(format!("{raw:.4}")));
            }
            ChanType::Str => {
                ui.label(
                    theme::value(values.strs.get(i).cloned().unwrap_or_default()).size(style.text),
                );
                ui.label("");
            }
            ChanType::Cmd => {
                ui.label("");
                ui.label("");
            }
        }
        self.store_channel(cfg.id, &c.name, &before, ccfg);
    }

    /// The cells of a control row after its name.
    pub(super) fn control_cells(
        &mut self,
        ui: &mut egui::Ui,
        cfg: &NodeConfig,
        schema: &Schema,
        view: &NodeView,
        c: &ChannelDef,
        linked: bool,
        style: Style,
    ) {
        let mut ccfg = cfg
            .controls
            .get(&c.name)
            .cloned()
            .unwrap_or_else(|| ControlConfig::default_for(c.ctype));
        let before = ccfg.clone();
        let numeric = matches!(c.ctype, ChanType::I32 | ChanType::F32);
        let (now, source) = self.station.resolve(cfg.id, c, &ccfg);
        let joystick = !matches!(source, Source::Manual);

        if numeric {
            ui.add(drag(&mut ccfg.step, 0.01, "step", style));
            ui.add(drag(&mut ccfg.min, 0.1, "min", style));
            ui.add(drag(&mut ccfg.max, 0.1, "max", style));
            ui.add(drag(&mut ccfg.offset, 0.01, "+", style))
                .on_hover_text("offset, added before the limits");
            if joystick {
                ui.add(
                    super::num(&mut ccfg.expo, 0.01)
                        .range(0.0..=1.0)
                        .prefix("expo "),
                )
                .on_hover_text(
                    "Softens the start of the stick's travel and still reaches full \
                     scale. 0 is linear.",
                );
            } else {
                ui.label("");
            }
        } else {
            for _ in 0..5 {
                ui.label("");
            }
        }

        // The value, and beside it what actually goes out.
        let v = ui
            .horizontal(|ui| {
                let v = self.control_value(ui, cfg, c, &ccfg, now, &source, linked, style);
                self.echo_mark(ui, cfg, schema, view, c, &ccfg, now);
                v
            })
            .inner;
        match (c.ctype, v) {
            (ChanType::I32 | ChanType::F32, Some(v)) => {
                ui.label(theme::value_muted(format!("{:.3}", ccfg.on_the_wire(v))));
            }
            _ => {
                ui.label("");
            }
        }

        if ccfg != before {
            self.edit_node(cfg.id, |n| {
                n.controls.insert(c.name.clone(), ccfg);
            });
        }
    }

    /// The editor for one control's value, or — while a joystick drives it —
    /// a readout of what the stick is producing, with a hold switch.
    ///
    /// Returns the value as it stands before offset, for the wire column.
    #[allow(clippy::too_many_arguments)]
    fn control_value(
        &mut self,
        ui: &mut egui::Ui,
        cfg: &NodeConfig,
        c: &ChannelDef,
        ccfg: &ControlConfig,
        now: Option<Intent>,
        source: &Source,
        linked: bool,
        style: Style,
    ) -> Option<f64> {
        let big = |t: &str| RichText::new(t).size(style.text);
        let numeric_now = match now {
            Some(Intent::F32(x)) => Some(x as f64),
            Some(Intent::I32(x)) => Some(x as f64),
            _ => None,
        };

        if let Source::Joystick(src) = source {
            match c.ctype {
                ChanType::Bool => {
                    theme::lamp(ui, matches!(now, Some(Intent::Bool(true))), style.lamp);
                }
                ChanType::I32 if !ccfg.labels.is_empty() => {
                    ui.label(
                        labelled(&ccfg.labels, numeric_now.unwrap_or(0.0) as i32).size(style.text),
                    );
                }
                _ => {
                    ui.label(
                        theme::value(format!("{:.3}", numeric_now.unwrap_or(0.0))).size(style.text),
                    );
                }
            }
            self.hold_switch(ui, cfg, c, ccfg, src, false);
            return numeric_now;
        }

        let out = match c.ctype {
            ChanType::Bool => {
                let on = matches!(now, Some(Intent::Bool(true)));
                if theme::lamp(ui, on, style.lamp).clicked() && !linked {
                    self.station.set_intent(cfg.id, &c.name, Intent::Bool(!on));
                }
                None
            }
            ChanType::Cmd => {
                let n = match now {
                    Some(Intent::Cmd(x)) => x,
                    _ => 0,
                };
                if ui
                    .add_enabled(
                        !linked,
                        egui::Button::new(big("Fire")).min_size(style.button),
                    )
                    .clicked()
                {
                    self.station.bump_cmd(cfg.id, &c.name);
                    self.status = format!("{} fired", c.name);
                }
                ui.label(theme::value_muted(format!("#{n}")));
                None
            }
            ChanType::Str => {
                ui.label(theme::value_muted("(string)"));
                None
            }
            ChanType::I32 | ChanType::F32 => {
                let mut v = numeric_now.unwrap_or(0.0);

                // A labelled i32 is a choice, not a number to type.
                if c.ctype == ChanType::I32 && !ccfg.labels.is_empty() {
                    let cur = v.round() as i32;
                    let mut pick = None;
                    ui.add_enabled_ui(!linked, |ui| {
                        egui::ComboBox::from_id_salt((
                            "ctrl-labels",
                            cfg.id,
                            &c.name,
                            style.inline,
                        ))
                        .selected_text(labelled(&ccfg.labels, cur).size(style.text))
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            for l in &ccfg.labels {
                                let text = labelled(&ccfg.labels, l.value);
                                if ui.selectable_label(l.value == cur, text).clicked() {
                                    pick = Some(l.value);
                                }
                            }
                        });
                    });
                    if let Some(nv) = pick {
                        self.station.set_intent(cfg.id, &c.name, Intent::I32(nv));
                        v = nv as f64;
                    }
                } else {
                    // [ - value + ] with clamping, as the 2026 grid did.
                    let mut set = None;
                    ui.add_enabled_ui(!linked, |ui| {
                        if ui
                            .add(egui::Button::new(big("−")).min_size(style.button))
                            .clicked()
                        {
                            set = Some(ccfg.clamp(v - ccfg.step));
                        }
                        let speed = (ccfg.step / 4.0).max(0.001);
                        let mut dv = super::num(&mut v, speed)
                            .range(ccfg.min.min(ccfg.max)..=ccfg.max.max(ccfg.min));
                        if style.inline {
                            dv = dv.custom_formatter(|x, _| format!("{x:.3}"));
                        }
                        let r = ui.add_sized(
                            egui::vec2(if style.inline { 90.0 } else { 60.0 }, style.button.y),
                            dv,
                        );
                        if r.changed() {
                            set = Some(ccfg.clamp(v));
                        }
                        if ui
                            .add(egui::Button::new(big("+")).min_size(style.button))
                            .clicked()
                        {
                            set = Some(ccfg.clamp(v + ccfg.step));
                        }
                    });
                    if let Some(nv) = set {
                        v = nv;
                        let intent = if c.ctype == ChanType::I32 {
                            Intent::I32(nv.round() as i32)
                        } else {
                            Intent::F32(nv as f32)
                        };
                        self.station.set_intent(cfg.id, &c.name, intent);
                    }
                }
                Some(v)
            }
        };
        if let Source::Held(src) = source {
            self.hold_switch(ui, cfg, c, ccfg, src, true);
        }
        out
    }

    fn hold_switch(
        &mut self,
        ui: &mut egui::Ui,
        cfg: &NodeConfig,
        c: &ChannelDef,
        ccfg: &ControlConfig,
        src: &str,
        held: bool,
    ) {
        ui.label(theme::value_muted(format!("🎮 {src}")));
        let mut h = held;
        if ui
            .toggle_value(&mut h, "hold")
            .on_hover_text(
                "Override the joystick with a fixed value. It starts from where the \
                 stick is, and is yours to set until you let go of hold.",
            )
            .changed()
        {
            self.station.set_hold(cfg.id, c, ccfg, h);
        }
    }

    /// A tick when a control's echo agrees with what is being sent; a warning
    /// when it has disagreed for longer than [`ECHO_GRACE`].
    #[allow(clippy::too_many_arguments)]
    fn echo_mark(
        &mut self,
        ui: &mut egui::Ui,
        cfg: &NodeConfig,
        schema: &Schema,
        view: &NodeView,
        c: &ChannelDef,
        ccfg: &ControlConfig,
        now: Option<Intent>,
    ) {
        let Some(echo_name) = cfg.echo_for(&c.name) else {
            return;
        };
        let Some(e) = schema
            .channels
            .iter()
            .find(|x| x.dir == Dir::Telemetry && x.name == echo_name)
        else {
            return;
        };
        let Some(values) = &view.telemetry else {
            return;
        };
        let i = e.slot as usize;
        let echoed = match e.ctype {
            ChanType::Bool => values.bools.get(i).map(|b| *b as u8 as f64),
            ChanType::I32 => values.i32s.get(i).map(|v| *v as f64),
            ChanType::F32 => values.f32s.get(i).map(|v| *v as f64),
            _ => None,
        };
        // Compare with what the board was sent, not with the box: the offset
        // and limits are applied on the way out.
        let sent = match now {
            Some(Intent::Bool(b)) => Some(b as u8 as f64),
            Some(Intent::I32(x)) => Some(ccfg.on_the_wire(x as f64).round()),
            Some(Intent::F32(x)) => Some(ccfg.on_the_wire(x as f64)),
            None => Some(0.0),
            _ => None,
        };
        let (Some(echoed), Some(sent)) = (echoed, sent) else {
            return;
        };
        let key = (cfg.id, c.name.clone());
        let tol = 1e-4 * sent.abs().max(1.0);
        if (echoed - sent).abs() <= tol {
            self.echo_mismatch.remove(&key);
            ui.label(RichText::new("ok").color(theme::MUTED))
                .on_hover_text(format!("{echo_name} confirms it"));
            return;
        }
        let since = *self.echo_mismatch.entry(key).or_insert_with(Instant::now);
        if since.elapsed() < ECHO_GRACE {
            ui.label(RichText::new("…").color(theme::MUTED))
                .on_hover_text(format!("waiting for {echo_name} to confirm"));
        } else {
            let shown = match e.ctype {
                ChanType::I32 => {
                    let labels = &cfg
                        .channels
                        .get(echo_name)
                        .map(|x| x.labels.clone())
                        .unwrap_or_default();
                    config::label_text(labels, echoed as i32)
                }
                _ => format!("{echoed}"),
            };
            ui.label(RichText::new(format!("!= {shown}")).color(theme::WARN))
                .on_hover_text(format!(
                    "{echo_name} reports {shown}, which is not what is being sent, and \
                     has not been for {:.1} s. The board may be refusing it, or not \
                     have received it.",
                    since.elapsed().as_secs_f32()
                ));
        }
    }

    /// Add a channel to the Operator chart, or take it off.
    pub(super) fn toggle_operator_plot(&mut self, node_id: u16, name: &str) {
        let mut p = self.station.profile.lock().unwrap();
        if let Some(chart) = p.operator_chart.as_mut() {
            let axis = Axis::channel(node_id, name);
            if let Some(pos) = chart.y.iter().position(|a| *a == axis) {
                chart.y.remove(pos);
                self.status = format!("{name} removed from the chart.");
            } else {
                chart.y.push(axis);
                self.status = format!("{name} added to the chart.");
            }
        }
    }

    /// Channels on the Operator chart for one node.
    pub(super) fn operator_plotted(&self, node_id: u16) -> Vec<String> {
        self.station
            .profile
            .lock()
            .unwrap()
            .operator_chart
            .as_ref()
            .map(|c| {
                c.y.iter()
                    .filter(|a| a.kind == AxisKind::Channel && a.node_id == node_id)
                    .map(|a| a.channel.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn store_channel(
        &self,
        node_id: u16,
        name: &str,
        before: &config::ChannelConfig,
        after: config::ChannelConfig,
    ) {
        if *before != after {
            self.edit_node(node_id, |n| {
                n.channels.insert(name.to_string(), after);
            });
        }
    }
}

/// The colour a channel's name is drawn in.
pub(super) fn name_colour(dir: Dir, linked: bool) -> Color32 {
    match (dir, linked) {
        (_, true) => theme::WARN,
        (Dir::Control, _) => theme::HEADER_TEXT,
        (Dir::Telemetry, _) => theme::TEXT,
    }
}
