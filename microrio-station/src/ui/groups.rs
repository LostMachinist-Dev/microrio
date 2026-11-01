//! Header pins, user-defined groups, and value labels.
//!
//! All of this is presentation. Groups, pins and label tables live in the
//! profile and change nothing on the wire or in the log: an i32 shown as
//! "teleop" is still sent and recorded as 1.

use egui::{Color32, RichText};
use microrio_proto::schema::{ChannelDef, Schema};
use microrio_proto::{ChanType, Dir};

use super::{rows, App};
use crate::config::{self, label_for, label_text, ChanRef, ControlConfig, NodeConfig, ValueLabel};
use crate::station::{Intent, NodeView, Station};
use crate::theme;

/// Changes to a node's groups, collected while they are drawn and applied
/// afterwards: the profile cannot be borrowed for writing while the groups
/// are being drawn out of a copy of it.
enum Op {
    Place(usize, DragChan, Option<usize>),
    Remove(usize, ChanRef),
    Delete(usize),
    Rename(usize, String),
    Collapse(usize, bool),
    Pin(usize, bool),
    MoveGroup(usize, usize),
}

/// What is being dragged: a channel, and the group it came out of, if any.
/// Dragging out of a group moves; dragging out of a grid adds.
#[derive(Clone)]
pub(super) struct DragChan {
    pub node_id: u16,
    pub chan: ChanRef,
    pub from_group: Option<usize>,
}

/// One channel's live reading, formatted for a compact cell.
pub(super) struct Reading {
    pub title: String,
    pub text: String,
    pub colour: Color32,
    /// Outside its warning band. Shown, never acted on.
    pub warn: bool,
}

pub(super) fn label_colour(l: &ValueLabel) -> Option<Color32> {
    l.rgb().map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

/// An i32 under its label table, as coloured text. Unmapped values show the
/// number rather than nothing.
pub(super) fn labelled(table: &[ValueLabel], v: i32) -> RichText {
    let colour = label_for(table, v)
        .and_then(label_colour)
        .unwrap_or(theme::TEXT);
    theme::value(label_text(table, v)).color(colour)
}

fn def<'a>(schema: &'a Schema, chan: &ChanRef) -> Option<&'a ChannelDef> {
    schema
        .channels
        .iter()
        .find(|c| c.dir == chan.dir && c.name == chan.channel)
}

/// Read a channel for a pin or a group row. `None` when this node does not
/// declare it — a profile written against older firmware, say.
pub(super) fn reading(
    cfg: &NodeConfig,
    schema: &Schema,
    view: &NodeView,
    station: &Station,
    chan: &ChanRef,
) -> Option<Reading> {
    let c = def(schema, chan)?;
    let i = c.slot as usize;
    let mut colour = theme::TEXT;
    let mut warn = false;

    let (title, text) = match chan.dir {
        Dir::Telemetry => {
            let ccfg = cfg.channels.get(&c.name).cloned().unwrap_or_default();
            let title = ccfg.title(&c.name).to_string();
            let Some(values) = &view.telemetry else {
                return Some(Reading {
                    title,
                    text: "—".into(),
                    colour: theme::MUTED,
                    warn: false,
                });
            };
            let units = if ccfg.units.is_empty() {
                String::new()
            } else {
                format!(" {}", ccfg.units)
            };
            let text = match c.ctype {
                ChanType::Bool => {
                    // Inverted channels read the other way up here too, or a
                    // pinned lamp would disagree with the grid it came from.
                    let on = ccfg.lamp(values.bools.get(i).copied().unwrap_or(false));
                    colour = if on { theme::OK } else { theme::MUTED };
                    if on {
                        "ON".into()
                    } else {
                        "off".into()
                    }
                }
                ChanType::I32 => {
                    let raw = values.i32s.get(i).copied().unwrap_or(0);
                    let scaled = ccfg.scale(raw as f64);
                    warn = ccfg.warns(scaled);
                    match label_for(&ccfg.labels, raw) {
                        Some(l) if !l.label.is_empty() => {
                            colour = label_colour(l).unwrap_or(theme::TEXT);
                            l.label.clone()
                        }
                        _ => format!("{}{units}", super::fmt_scaled(scaled, &ccfg)),
                    }
                }
                ChanType::F32 => {
                    let raw = values.f32s.get(i).copied().unwrap_or(0.0) as f64;
                    let scaled = ccfg.scale(raw);
                    warn = ccfg.warns(scaled);
                    format!("{scaled:.3}{units}")
                }
                ChanType::Str => values.strs.get(i).cloned().unwrap_or_default(),
                ChanType::Cmd => "—".into(),
            };
            (title, text)
        }
        Dir::Control => {
            let ccfg = cfg
                .controls
                .get(&c.name)
                .cloned()
                .unwrap_or_else(|| ControlConfig::default_for(c.ctype));
            let title = ccfg.title(&c.name).to_string();
            let units = if ccfg.units.is_empty() {
                String::new()
            } else {
                format!(" {}", ccfg.units)
            };
            // What is actually being sent, joystick included.
            let (now, _) = station.resolve(cfg.id, c, &ccfg);
            let text = match (c.ctype, now.as_ref()) {
                (ChanType::Bool, Some(Intent::Bool(on))) => {
                    colour = if *on { theme::OK } else { theme::MUTED };
                    if *on {
                        "ON".into()
                    } else {
                        "off".into()
                    }
                }
                (ChanType::Bool, _) => {
                    colour = theme::MUTED;
                    "off".into()
                }
                (ChanType::I32, intent) => {
                    let v = match intent {
                        Some(Intent::I32(v)) => *v,
                        _ => 0,
                    };
                    match label_for(&ccfg.labels, v) {
                        Some(l) if !l.label.is_empty() => {
                            colour = label_colour(l).unwrap_or(theme::TEXT);
                            l.label.clone()
                        }
                        _ => format!("{v}{units}"),
                    }
                }
                (ChanType::F32, intent) => {
                    let v = match intent {
                        Some(Intent::F32(v)) => *v,
                        _ => 0.0,
                    };
                    format!("{v:.3}{units}")
                }
                (ChanType::Cmd, intent) => match intent {
                    Some(Intent::Cmd(n)) => format!("#{n}"),
                    _ => "#0".into(),
                },
                _ => "—".into(),
            };
            (title, text)
        }
    };
    Some(Reading {
        title,
        text,
        colour,
        warn,
    })
}

/// A header chip: a short title over a value, tinted when it warns.
pub(super) fn chip(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui), warn: bool) {
    egui::Frame::new()
        .fill(if warn {
            Color32::from_rgb(70, 45, 10)
        } else {
            theme::HEADER_BG
        })
        .stroke(egui::Stroke::new(
            1.0,
            if warn { theme::WARN } else { theme::BORDER },
        ))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .corner_radius(3.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(title.to_uppercase()).small().color(if warn {
                    theme::WARN
                } else {
                    theme::MUTED
                }));
                body(ui);
            });
        });
}

impl App {
    /// The pinned channels and groups for one node, as a row of chips.
    pub(super) fn header_pins(&self, ui: &mut egui::Ui, cfg: &NodeConfig, view: &NodeView) {
        let Some(schema) = &view.schema else {
            return;
        };

        for pin in &cfg.pins {
            match reading(cfg, schema, view, &self.station, pin) {
                Some(r) => chip(
                    ui,
                    &r.title,
                    |ui| {
                        ui.label(theme::value(&r.text).color(if r.warn {
                            theme::WARN
                        } else {
                            r.colour
                        }));
                    },
                    r.warn,
                ),
                None => chip(
                    ui,
                    &pin.channel,
                    |ui| {
                        ui.label(theme::value_muted("not on this node"));
                    },
                    false,
                ),
            }
        }
        for g in cfg.groups.iter().filter(|g| g.pinned) {
            let rs: Vec<Reading> = g
                .members
                .iter()
                .filter_map(|m| reading(cfg, schema, view, &self.station, m))
                .collect();
            let warn = rs.iter().any(|r| r.warn);
            chip(
                ui,
                &g.name,
                |ui| {
                    if rs.is_empty() {
                        ui.label(theme::value_muted("empty"));
                    }
                    // Four is what fits beside the rest at half width.
                    for (k, r) in rs.iter().take(4).enumerate() {
                        if k > 0 {
                            ui.label(theme::value_muted("·"));
                        }
                        ui.label(theme::value(&r.text).color(if r.warn {
                            theme::WARN
                        } else {
                            r.colour
                        }))
                        .on_hover_text(&r.title);
                    }
                    if rs.len() > 4 {
                        ui.label(theme::value_muted(format!("+{}", rs.len() - 4)));
                    }
                },
                warn,
            );
        }
    }

    /// The node's groups, above the grids on the Operator page.
    pub(super) fn groups_panel(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Schema,
        view: &NodeView,
        cfg: &NodeConfig,
    ) {
        let driven = self.driven(cfg.id);
        let on_chart = self.operator_plotted(cfg.id);

        ui.horizontal_wrapped(|ui| {
            ui.label(theme::header("Groups"));
            if ui
                .small_button("+ group")
                .on_hover_text("A named set of channels, such as everything about one valve")
                .clicked()
            {
                self.edit_node(cfg.id, |n| {
                    let name = format!("Group {}", n.groups.len() + 1);
                    n.groups.push(config::Group::new(name));
                });
                self.edit_groups = true;
            }
            if !cfg.groups.is_empty() {
                ui.toggle_value(&mut self.edit_groups, "edit")
                    .on_hover_text("Rename, pin, add or remove channels, delete groups");
            }
            if cfg.groups.is_empty() {
                ui.label(theme::value_muted(
                    "none yet — channels can be dragged here from the grids by their name",
                ));
            }
        });

        let mut ops: Vec<Op> = Vec::new();
        let editing = self.edit_groups;
        let n_groups = cfg.groups.len();

        // Side by side on a wide window: a group is about as wide as a
        // control row needs and no wider, so one per line wastes most of the
        // screen and pushes the chart off the bottom.
        let columns = if ui.available_width() >= super::WIDE_PX && cfg.groups.len() > 1 {
            2
        } else {
            1
        };
        let ctx = GroupCtx {
            editing,
            n_groups,
            driven: &driven,
            on_chart: &on_chart,
        };
        if columns == 1 {
            for (gi, g) in cfg.groups.iter().enumerate() {
                self.draw_group(ui, schema, view, cfg, gi, g, &ctx, &mut ops);
            }
        } else {
            // Down the first column, then the second, so reordering with the
            // arrows still reads in the order the groups are listed.
            let half = cfg.groups.len().div_ceil(2);
            let mut panes: Vec<Vec<(usize, &config::Group)>> = vec![Vec::new(), Vec::new()];
            for (gi, g) in cfg.groups.iter().enumerate() {
                panes[usize::from(gi >= half)].push((gi, g));
            }
            ui.columns(2, |cols| {
                for (col, pane) in cols.iter_mut().zip(panes) {
                    for (gi, g) in pane {
                        self.draw_group(col, schema, view, cfg, gi, g, &ctx, &mut ops);
                    }
                }
            });
        }

        if ops.is_empty() {
            return;
        }
        let mut refused = false;
        self.edit_node(cfg.id, |n| {
            for op in ops {
                match op {
                    Op::Place(g, d, at) => {
                        // Out of a group into another is a move; within one
                        // it is a reorder, which place_in_group handles.
                        if let Some(from) = d.from_group {
                            if from != g {
                                n.remove_from_group(from, &d.chan);
                            }
                        }
                        n.place_in_group(g, d.chan, at);
                    }
                    Op::Remove(g, c) => n.remove_from_group(g, &c),
                    Op::Delete(g) => {
                        if g < n.groups.len() {
                            n.groups.remove(g);
                        }
                    }
                    Op::Rename(g, name) => {
                        if let Some(x) = n.groups.get_mut(g) {
                            x.name = name;
                        }
                    }
                    Op::Collapse(g, c) => {
                        if let Some(x) = n.groups.get_mut(g) {
                            x.collapsed = c;
                        }
                    }
                    Op::Pin(g, on) => refused |= !n.set_group_pinned(g, on),
                    Op::MoveGroup(a, b) => {
                        if a < n.groups.len() && b < n.groups.len() {
                            n.groups.swap(a, b);
                        }
                    }
                }
            }
        });
        if refused {
            self.status = format!(
                "The header holds {} items at most. Unpin something first.",
                config::MAX_HEADER_ITEMS
            );
        }
    }
}

/// What every group on a page shares, so one group's drawing needs four
/// arguments rather than eight.
struct GroupCtx<'a> {
    editing: bool,
    n_groups: usize,
    /// Control channels a cross-node link is driving.
    driven: &'a [String],
    /// Telemetry channels already on the Operator chart.
    on_chart: &'a [String],
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn draw_group(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Schema,
        view: &NodeView,
        cfg: &NodeConfig,
        gi: usize,
        g: &config::Group,
        ctx: &GroupCtx<'_>,
        ops: &mut Vec<Op>,
    ) {
        let (editing, n_groups) = (ctx.editing, ctx.n_groups);
        let (driven, on_chart) = (ctx.driven, ctx.on_chart);
        let frame = egui::Frame::new()
            .fill(theme::PANEL)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let arrow = if g.collapsed { ">" } else { "v" };
                    if ui.small_button(arrow).clicked() {
                        ops.push(Op::Collapse(gi, !g.collapsed));
                    }
                    if editing {
                        let mut name = g.name.clone();
                        if ui
                            .add_sized(
                                egui::vec2(150.0, 18.0),
                                egui::TextEdit::singleline(&mut name),
                            )
                            .changed()
                        {
                            ops.push(Op::Rename(gi, name));
                        }
                    } else {
                        ui.label(RichText::new(&g.name).strong().color(theme::HEADER_TEXT));
                    }
                    let mut pinned = g.pinned;
                    if ui
                        .toggle_value(&mut pinned, "pin")
                        .on_hover_text("Show this group in the node's header row")
                        .changed()
                    {
                        ops.push(Op::Pin(gi, pinned));
                    }
                    if editing {
                        if ui
                            .add_enabled(gi > 0, egui::Button::new("^").small())
                            .clicked()
                        {
                            ops.push(Op::MoveGroup(gi, gi - 1));
                        }
                        if ui
                            .add_enabled(gi + 1 < n_groups, egui::Button::new("v").small())
                            .clicked()
                        {
                            ops.push(Op::MoveGroup(gi, gi + 1));
                        }
                        // Add without dragging, for anyone who would rather pick.
                        egui::ComboBox::from_id_salt(("grp-add", cfg.id, gi))
                            .selected_text("add channel")
                            .width(130.0)
                            .show_ui(ui, |ui| {
                                for c in &schema.channels {
                                    let r = ChanRef {
                                        dir: c.dir,
                                        channel: c.name.clone(),
                                    };
                                    if g.members.contains(&r) {
                                        continue;
                                    }
                                    let tag = if c.dir == Dir::Telemetry { "t" } else { "c" };
                                    if ui
                                        .selectable_label(false, format!("{tag} · {}", c.name))
                                        .clicked()
                                    {
                                        ops.push(Op::Place(
                                            gi,
                                            DragChan {
                                                node_id: cfg.id,
                                                chan: r,
                                                from_group: None,
                                            },
                                            None,
                                        ));
                                    }
                                }
                            });
                        if ui.small_button("delete").clicked() {
                            ops.push(Op::Delete(gi));
                        }
                    }
                    if g.collapsed {
                        // A collapsed group still says what it is doing.
                        let rs: Vec<Reading> = g
                            .members
                            .iter()
                            .filter_map(|m| reading(cfg, schema, view, &self.station, m))
                            .collect();
                        for r in rs.iter().take(6) {
                            ui.label(theme::value(&r.text).color(if r.warn {
                                theme::WARN
                            } else {
                                r.colour
                            }))
                            .on_hover_text(&r.title);
                        }
                    }
                });

                if g.collapsed {
                    return;
                }
                if g.members.is_empty() {
                    ui.label(theme::value_muted("drag channels here"));
                }
                egui::Grid::new(("grp", cfg.id, gi))
                    .num_columns(rows::ROW_COLUMNS + 2)
                    .spacing([8.0, 5.0])
                    .show(ui, |ui| {
                        for (mi, m) in g.members.iter().enumerate() {
                            if cfg.is_hidden(m) {
                                continue;
                            }
                            let payload = DragChan {
                                node_id: cfg.id,
                                chan: m.clone(),
                                from_group: Some(gi),
                            };
                            let handle = ui
                                .dnd_drag_source(
                                    egui::Id::unique(("grp-drag", cfg.id, gi, mi)),
                                    payload,
                                    |ui| {
                                        ui.label(
                                            RichText::new("::").size(16.0).color(theme::MUTED),
                                        );
                                    },
                                )
                                .response
                                .on_hover_text("Drag to reorder, or into another group");

                            let Some(c) = def(schema, m) else {
                                ui.label(theme::value_muted(&m.channel));
                                ui.label(theme::value_muted("not on this node"));
                                if editing && ui.small_button("×").clicked() {
                                    ops.push(Op::Remove(gi, m.clone()));
                                }
                                ui.end_row();
                                continue;
                            };
                            let linked =
                                m.dir == Dir::Control && driven.iter().any(|d| *d == c.name);
                            let title = match m.dir {
                                Dir::Telemetry => cfg
                                    .channels
                                    .get(&c.name)
                                    .map(|x| x.title(&c.name).to_string()),
                                Dir::Control => cfg
                                    .controls
                                    .get(&c.name)
                                    .map(|x| x.title(&c.name).to_string()),
                            }
                            .unwrap_or_else(|| c.name.clone());
                            let name_resp = ui.label(
                                RichText::new(&title)
                                    .size(15.0)
                                    .color(rows::name_colour(m.dir, linked)),
                            );

                            match m.dir {
                                Dir::Telemetry => self.telemetry_cells(
                                    ui,
                                    cfg,
                                    view,
                                    c,
                                    on_chart.contains(&c.name),
                                    rows::Style::group(),
                                ),
                                Dir::Control => self.control_cells(
                                    ui,
                                    cfg,
                                    schema,
                                    view,
                                    c,
                                    linked,
                                    rows::Style::group(),
                                ),
                            }

                            if editing {
                                if ui
                                    .small_button("×")
                                    .on_hover_text("Remove from group")
                                    .clicked()
                                {
                                    ops.push(Op::Remove(gi, m.clone()));
                                }
                            } else {
                                ui.label("");
                            }

                            // Dropping onto a row puts the channel before it.
                            let top = handle.rect.top().min(name_resp.rect.top());
                            let bottom = handle.rect.bottom().max(name_resp.rect.bottom());
                            let row = ui.interact(
                                egui::Rect::from_x_y_ranges(
                                    handle.rect.left()..=ui.max_rect().right(),
                                    top..=bottom,
                                ),
                                egui::Id::unique(("grp-row", cfg.id, gi, mi)),
                                egui::Sense::hover(),
                            );
                            if let Some(p) = row.dnd_hover_payload::<DragChan>() {
                                if p.node_id == cfg.id {
                                    ui.painter().hline(
                                        row.rect.x_range(),
                                        row.rect.top() - 2.0,
                                        egui::Stroke::new(2.0, theme::LED_ON),
                                    );
                                }
                            }
                            if let Some(p) = row.dnd_release_payload::<DragChan>() {
                                if p.node_id == cfg.id {
                                    ops.push(Op::Place(gi, (*p).clone(), Some(mi)));
                                }
                            }
                            ui.end_row();
                        }
                    });
            });

        // Dropping anywhere else on the group adds to the end.
        let resp = frame.response;
        if let Some(p) = resp.dnd_hover_payload::<DragChan>() {
            if p.node_id == cfg.id {
                ui.painter().rect_stroke(
                    resp.rect,
                    2.0,
                    egui::Stroke::new(1.5, theme::LED_ON),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if let Some(p) = resp.dnd_release_payload::<DragChan>() {
            let already = ops.iter().any(|o| matches!(o, Op::Place(..)));
            if p.node_id == cfg.id && !already {
                ops.push(Op::Place(gi, (*p).clone(), None));
            }
        }
        ui.add_space(4.0);
    }
}

impl App {
    /// Change one node's profile entry in place.
    pub(super) fn edit_node(&self, id: u16, f: impl FnOnce(&mut NodeConfig)) {
        let mut p = self.station.profile.lock().unwrap();
        if let Some(n) = p.nodes.iter_mut().find(|n| n.id == id) {
            f(n);
        }
    }
}
