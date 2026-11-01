//! Settings: how each channel is presented. Names, units, warning thresholds,
//! value labels and header pins. None of it reaches the board.

use egui::RichText;
use microrio_proto::{ChanType, Dir};

use super::groups::label_colour;
use super::{hint, section, App};
use crate::config::{self, ChanRef, ControlConfig, ValueLabel};
use crate::theme;

impl App {
    /// One section per node, because the channel names only mean anything
    /// alongside the board that declares them: two nodes can both have `a0`.
    pub(super) fn settings_channels(&mut self, ui: &mut egui::Ui) {
        let nodes: Vec<config::NodeConfig> = self.station.profile.lock().unwrap().nodes.clone();
        for cfg in nodes {
            self.settings_channels_for(ui, &cfg);
        }
    }

    fn settings_channels_for(&mut self, ui: &mut egui::Ui, cfg: &config::NodeConfig) {
        let view = self
            .station
            .views
            .lock()
            .unwrap()
            .get(&cfg.id)
            .cloned()
            .unwrap_or_default();
        let schema = view.schema.clone();

        section(
            ui,
            &format!("channel display  ·  {} #{}", cfg.label, cfg.id),
            |ui| {
                hint(
                    ui,
                    "How this node's channels are shown. Show takes a \
                 channel out of the grids and groups without forgetting its settings. Pinned channels \
                 sit in the node's header row. A threshold only colours the reading — it \
                 never faults or disables anything. Labels turn an i32 into words: a \
                 telemetry i32 shows the label, a control i32 becomes a dropdown. The board \
                 and the log still see the plain number. The `now` column is this node's \
                 live value, so a channel can be checked here whether or not it is shown \
                 on the Operator page.",
                );
                let Some(schema) = schema else {
                    ui.label(theme::value_muted(crate::station::no_schema_note(cfg, &view)));
                    return;
                };
                ui.label(theme::value_muted(format!(
                    "header: {} of {} slots in use",
                    cfg.header_items(),
                    config::MAX_HEADER_ITEMS
                )));

                let mut refused = false;
                let controls: Vec<String> = schema
                    .channels
                    .iter()
                    .filter(|c| {
                        c.dir == Dir::Control
                            && matches!(c.ctype, ChanType::Bool | ChanType::I32 | ChanType::F32)
                    })
                    .map(|c| c.name.clone())
                    .collect();
                egui::Grid::new(("chan-display", cfg.id))
                    .num_columns(13)
                    .striped(true)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        for h in [
                            "",
                            "channel",
                            "now",
                            "display name",
                            "units",
                            "warn below",
                            "warn above",
                            "show",
                            "pin",
                            "invert",
                            "echo of",
                            "labels",
                        ] {
                            ui.label(theme::header(h));
                        }
                        ui.end_row();

                        for c in &schema.channels {
                            if c.ctype == ChanType::Str && c.dir == Dir::Control {
                                continue; // nothing on the station sets these yet
                            }
                            let r = ChanRef {
                                dir: c.dir,
                                channel: c.name.clone(),
                            };
                            let numeric = matches!(c.ctype, ChanType::I32 | ChanType::F32);
                            ui.label(theme::value_muted(if c.dir == Dir::Telemetry {
                                "t"
                            } else {
                                "c"
                            }))
                            .on_hover_text(
                                if c.dir == Dir::Telemetry {
                                    "telemetry: node -> station"
                                } else {
                                    "control: station -> node"
                                },
                            );
                            ui.label(RichText::new(&c.name).monospace().color(theme::TEXT));

                            // What is coming through, right beside the
                            // settings for it. This is where someone comes to
                            // check a channel, including one hidden from the
                            // Operator page, so it has to be visible here.
                            match super::groups::reading(cfg, &schema, &view, &self.station, &r) {
                                Some(live) => {
                                    ui.label(theme::value(&live.text).color(if live.warn {
                                        theme::WARN
                                    } else {
                                        live.colour
                                    }))
                                }
                                None => ui.label(theme::value_muted("—")),
                            };

                            let mut p = self.station.profile.lock().unwrap();
                            let Some(n) = p.nodes.iter_mut().find(|n| n.id == cfg.id) else {
                                ui.end_row();
                                continue;
                            };
                            let mut pinned = n.pins.contains(&r);
                            let pin_changed;
                            let mut shown = !n.is_hidden(&r);

                            // Edit a copy and store it only if it changed, so that
                            // looking at this page does not write a default entry
                            // for every channel into the profile.
                            match c.dir {
                                Dir::Telemetry => {
                                    let mut cc =
                                        n.channels.get(&c.name).cloned().unwrap_or_default();
                                    let before = cc.clone();
                                    names(ui, &c.name, &mut cc.display_name, &mut cc.units);
                                    if numeric {
                                        optional(ui, ("lo", cfg.id, &c.name), &mut cc.warn_below);
                                        optional(ui, ("hi", cfg.id, &c.name), &mut cc.warn_above);
                                    } else {
                                        ui.label(theme::value_muted("—"));
                                        ui.label(theme::value_muted("—"));
                                    }
                                    if ui
                                        .checkbox(&mut shown, "")
                                        .on_hover_text("Show in the grids and groups")
                                        .changed()
                                    {
                                        cc.hidden = Some(!shown);
                                    }
                                    pin_changed = ui.checkbox(&mut pinned, "").changed();
                                    if c.ctype == ChanType::Bool {
                                        ui.checkbox(&mut cc.invert_display, "").on_hover_text(
                                            "Light the lamp when this reads false. For a \
                                             signal that is normally true — rtc_ok, healthy \
                                             — so that a lit lamp always means look at me.",
                                        );
                                    } else {
                                        ui.label(theme::value_muted("—"));
                                    }
                                    echo_picker(ui, ("echo", cfg.id, &c.name), &mut cc, &controls);
                                    if c.ctype == ChanType::I32 {
                                        label_table(
                                            ui,
                                            ("labels", cfg.id, 0u8, &c.name),
                                            &mut cc.labels,
                                        );
                                    } else {
                                        ui.label(theme::value_muted("—"));
                                    }
                                    if cc != before {
                                        n.channels.insert(c.name.clone(), cc);
                                    }
                                }
                                Dir::Control => {
                                    let mut cc = n
                                        .controls
                                        .get(&c.name)
                                        .cloned()
                                        .unwrap_or_else(|| ControlConfig::default_for(c.ctype));
                                    let before = cc.clone();
                                    names(ui, &c.name, &mut cc.display_name, &mut cc.units);
                                    ui.label(theme::value_muted("—"));
                                    ui.label(theme::value_muted("—"));
                                    if ui
                                        .checkbox(&mut shown, "")
                                        .on_hover_text("Show in the grids and groups")
                                        .changed()
                                    {
                                        cc.hidden = Some(!shown);
                                    }
                                    pin_changed = ui.checkbox(&mut pinned, "").changed();
                                    ui.label(theme::value_muted("—"));
                                    ui.label(theme::value_muted("—"));
                                    if c.ctype == ChanType::I32 {
                                        label_table(
                                            ui,
                                            ("labels", cfg.id, 1u8, &c.name),
                                            &mut cc.labels,
                                        );
                                    } else {
                                        ui.label(theme::value_muted("—"));
                                    }
                                    if cc != before {
                                        n.controls.insert(c.name.clone(), cc);
                                    }
                                }
                            }
                            if pin_changed {
                                refused |= !n.set_pinned(&r, pinned);
                            }
                            ui.end_row();
                        }
                    });
                if refused {
                    self.status = format!(
                    "The header holds {} items at most, pinned groups included. Unpin something first.",
                    config::MAX_HEADER_ITEMS
                );
                }
            },
        );
    }
}

fn names(ui: &mut egui::Ui, channel: &str, display: &mut String, units: &mut String) {
    ui.add_sized(
        egui::vec2(140.0, 18.0),
        egui::TextEdit::singleline(display).hint_text(channel),
    );
    ui.add_sized(egui::vec2(60.0, 18.0), egui::TextEdit::singleline(units));
}

/// Which control this telemetry channel reports back, if any.
fn echo_picker(
    ui: &mut egui::Ui,
    id: impl egui::AsIdSalt,
    cc: &mut config::ChannelConfig,
    controls: &[String],
) {
    let shown = cc.echo_of.clone().unwrap_or_else(|| "—".into());
    egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(shown).monospace())
        .width(110.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(cc.echo_of.is_none(), "—").clicked() {
                cc.echo_of = None;
            }
            for c in controls {
                if ui
                    .selectable_label(cc.echo_of.as_deref() == Some(c), c)
                    .clicked()
                {
                    cc.echo_of = Some(c.clone());
                }
            }
        })
        .response
        .on_hover_text(
            "The control this channel reports back. Its control row then shows ok \
             when they agree and warns when they do not. An echo is hidden from the \
             grids by default, since its control already shows it.",
        );
}

/// A threshold that may be off: a checkbox, and a number when it is on.
fn optional(ui: &mut egui::Ui, id: impl egui::AsIdSalt, v: &mut Option<f64>) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            let mut on = v.is_some();
            if ui.checkbox(&mut on, "").changed() {
                *v = if on { Some(0.0) } else { None };
            }
            if let Some(x) = v.as_mut() {
                ui.add(super::num(x, 0.05));
            }
        });
    });
}

/// The value-to-label editor, folded away until asked for.
fn label_table(ui: &mut egui::Ui, id: impl egui::AsIdSalt, table: &mut Vec<ValueLabel>) {
    ui.push_id(id, |ui| {
        let summary = if table.is_empty() {
            "none".to_string()
        } else {
            table
                .iter()
                .take(3)
                .map(|l| format!("{}={}", l.value, l.label))
                .collect::<Vec<_>>()
                .join(" ")
                + if table.len() > 3 { " …" } else { "" }
        };
        egui::CollapsingHeader::new(RichText::new(summary).monospace().small())
            .id_salt("tbl")
            .show(ui, |ui| {
                let mut remove = None;
                egui::Grid::new("rows").spacing([6.0, 3.0]).show(ui, |ui| {
                    for (i, l) in table.iter_mut().enumerate() {
                        ui.add(super::num(&mut l.value, 0.0));
                        ui.add_sized(
                            egui::vec2(110.0, 18.0),
                            egui::TextEdit::singleline(&mut l.label),
                        );
                        let mut coloured = l.colour.is_some();
                        if ui
                            .checkbox(&mut coloured, "")
                            .on_hover_text("Give this label its own colour")
                            .changed()
                        {
                            l.colour = coloured.then(|| "#55ff55".to_string());
                        }
                        if coloured {
                            let mut rgb = label_colour(l)
                                .map(|c| [c.r(), c.g(), c.b()])
                                .unwrap_or([0x55, 0xff, 0x55]);
                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                l.colour =
                                    Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
                            }
                        } else {
                            ui.label("");
                        }
                        if ui.small_button("×").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
                if let Some(i) = remove {
                    table.remove(i);
                }
                if ui.small_button("+ label").clicked() {
                    let next = table
                        .iter()
                        .map(|l| l.value)
                        .max()
                        .map(|v| v + 1)
                        .unwrap_or(0);
                    table.push(ValueLabel {
                        value: next,
                        label: String::new(),
                        colour: None,
                    });
                }
            });
    });
}
