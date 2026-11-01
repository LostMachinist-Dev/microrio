//! The driver station window.
//!
//! Layout and styling follow the 2026 instrument GUI. The Operator page is
//! where the time goes, so it carries telemetry, control and a chart at once;
//! everything else is a tab you visit and leave.
//!
//! The window is built to be usable at two sizes, because those are the two
//! that get used: filling a screen, and snapped to half of one. Wide enough
//! and the grids sit side by side; narrower and they stack rather than
//! overlapping.
//!
//! The UI is a subscriber. It publishes a heartbeat and reads snapshots; it
//! never sits in the control path, so a hung window cannot stop a FAULT frame
//! going out.

use std::sync::{Arc, Mutex};

use egui::{Color32, RichText};
use egui_plot::{Legend, Line, Plot, PlotBounds, PlotPoints, Points, VLine};
use microrio_proto::schema::Schema;
use microrio_proto::{ChanType, Dir, State};

use crate::clock::{Zone, COMMON_ZONES};
use crate::config::{self, Axis, AxisKind, ChannelConfig, ChartConfig, CrossLink};
use crate::history;
use crate::joystick::JoystickState;
use crate::station::{no_schema_note, Intent, Station};
use crate::theme;

mod channels;
mod groups;
mod rows;

use groups::DragChan;

/// Below this window width the two grids stack instead of sitting side by
/// side. A half-screen window on a 1920-wide display is 960, which must
/// stack; a full screen is 1920, which must not.
const WIDE_PX: f32 = 1180.0;

pub struct App {
    station: Arc<Station>,
    joy: Arc<Mutex<JoystickState>>,
    tab: Tab,
    selected_node: u16,
    status: String,
    new_map_source: String,
    new_map_channel: String,
    /// Time zone as typed in Settings, committed to the profile when valid.
    tz_input: String,
    /// How tall the operator grids were last frame. The chart gets whatever
    /// is left, so a short grid gives a tall chart instead of a dead band.
    grids_h: f32,
    /// Result of the last CSV export, handed back from the worker thread.
    export: Arc<Mutex<Option<String>>>,
    exporting: bool,
    styled: bool,
    /// Groups show rename, pin and delete controls.
    edit_groups: bool,
    /// The grids list grouped channels too, rather than only the rest.
    show_all_channels: bool,
    /// Pause and clear state per chart. "op" is the Operator page's chart.
    chart_rt: std::collections::HashMap<String, ChartRt>,
    /// Buttons seen part-way pressed, so they stay listed with the axes.
    seen_travel: std::collections::HashSet<String>,
    /// What a file dialog came back with, set by its worker thread.
    file_job: Arc<Mutex<Option<FileJob>>>,
    /// A dialog is up. Only one at a time, and the buttons say so.
    dialog_open: bool,
    /// The About panel is open.
    about: bool,
    /// When each control first disagreed with its echo, by node and channel.
    echo_mismatch: std::collections::HashMap<(u16, String), std::time::Instant>,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Operator,
    Charts,
    Clock,
    Joystick,
    Logging,
    Settings,
}

impl App {
    pub fn new(station: Arc<Station>, joy: Arc<Mutex<JoystickState>>) -> Self {
        let (selected_node, tz_input) = {
            let p = station.profile.lock().unwrap();
            (
                p.nodes.first().map(|n| n.id).unwrap_or(1),
                p.timezone.clone(),
            )
        };
        Self {
            station,
            joy,
            tab: Tab::Operator,
            selected_node,
            status: String::new(),
            new_map_source: String::new(),
            new_map_channel: String::new(),
            tz_input,
            grids_h: 0.0,
            export: Arc::new(Mutex::new(None)),
            exporting: false,
            styled: false,
            edit_groups: false,
            show_all_channels: false,
            chart_rt: Default::default(),
            file_job: Arc::new(Mutex::new(None)),
            dialog_open: false,
            about: false,
            seen_travel: Default::default(),
            echo_mismatch: Default::default(),
        }
    }

    pub fn select_tab(&mut self, name: &str) {
        self.tab = match name.to_ascii_lowercase().as_str() {
            "chart" | "charts" | "plot" => Tab::Charts,
            "clock" | "time" => Tab::Clock,
            "joystick" | "joy" => Tab::Joystick,
            "log" | "logging" => Tab::Logging,
            "settings" => Tab::Settings,
            _ => Tab::Operator,
        };
    }

    fn node_cfg(&self) -> Option<config::NodeConfig> {
        self.station
            .profile
            .lock()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.id == self.selected_node)
            .cloned()
    }

    fn zone(&self) -> Zone {
        Zone::parse(&self.station.profile.lock().unwrap().timezone)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.styled {
            theme::apply(ui.ctx());
            self.styled = true;
        }
        // The heartbeat the network loops watch. If this stops, every node faults.
        self.station.beat();
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(25));

        if let Some(msg) = self.export.lock().unwrap().take() {
            self.status = msg;
            self.exporting = false;
        }
        self.take_file_job();
        // Here rather than in the Operator page: opening straight to Charts
        // with --tab charts would otherwise find none, because the page that
        // created the first one had never been drawn.
        self.ensure_charts();

        egui::Panel::top("chrome")
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(6.0))
            .show(ui, |ui| {
                self.safety_bar(ui);
                self.stall_banner(ui);
                self.profile_bar(ui);
                ui.add_space(2.0);
                self.about_panel(ui);
                self.node_bar(ui);
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Operator, "Operator");
                    ui.selectable_value(&mut self.tab, Tab::Charts, "Charts");
                    ui.selectable_value(&mut self.tab, Tab::Clock, "Clock");
                    ui.selectable_value(&mut self.tab, Tab::Joystick, "Joystick");
                    ui.selectable_value(&mut self.tab, Tab::Logging, "Logging");
                    ui.selectable_value(&mut self.tab, Tab::Settings, "Settings");
                });
            });

        if !self.status.is_empty() {
            egui::Panel::bottom("status")
                .resizable(false)
                .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(6.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("×").clicked() {
                            self.status.clear();
                        }
                        ui.label(
                            RichText::new(&self.status)
                                .color(Color32::LIGHT_BLUE)
                                .small(),
                        );
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(8.0))
            .show(ui, |ui| match self.tab {
                // The operator page manages its own scrolling: the chart is
                // pinned under the grids rather than scrolling away from them.
                Tab::Operator => self.operator_tab(ui),
                Tab::Charts => self.charts_tab(ui),
                Tab::Clock => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.clock_tab(ui));
                }
                Tab::Joystick => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.joystick_tab(ui));
                }
                Tab::Logging => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.logging_tab(ui));
                }
                Tab::Settings => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.settings_tab(ui));
                }
            });
    }
}

// ------------------------------------------------------------------- chrome

impl App {
    fn safety_bar(&mut self, ui: &mut egui::Ui) {
        let state = self.station.aggregate_state();
        let (label, colour) = match state {
            State::Enabled => ("ENABLED", Color32::from_rgb(85, 85, 85)),
            State::Disabled => ("DISABLED", Color32::from_rgb(40, 40, 40)),
            State::Estop => ("E-STOP", Color32::from_rgb(190, 40, 40)),
            State::Fault => ("FAULT", theme::WARN),
        };
        let estopped = self
            .station
            .estop
            .load(std::sync::atomic::Ordering::Relaxed);
        let node_count = self.station.profile.lock().unwrap().nodes.len();
        let live = self.station.enabled_count();

        ui.horizontal(|ui| {
            // Big, and only ever on screen. There is deliberately no hotkey:
            // a keyboard estop is one that gets pressed by accident.
            let estop = egui::Button::new(
                RichText::new("E-STOP")
                    .size(30.0)
                    .strong()
                    .color(Color32::WHITE),
            )
            .fill(Color32::from_rgb(185, 25, 25))
            .stroke(egui::Stroke::new(2.0, Color32::from_rgb(255, 120, 120)))
            .corner_radius(6.0)
            .min_size(egui::vec2(230.0, 58.0));
            if ui
                .add(estop)
                .on_hover_text(
                    "Latches. Every node is disabled and stays that way until you clear it.",
                )
                .clicked()
            {
                self.station.set_estop();
                self.status = "E-stop latched. Every node disabled.".into();
            }
            ui.add_space(10.0);

            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("  {label}  "))
                            .size(20.0)
                            .strong()
                            .color(Color32::WHITE)
                            .background_color(colour),
                    );
                    ui.add_space(8.0);
                    ui.label(theme::value_muted(format!(
                        "{live} of {node_count} enabled"
                    )))
                    .on_hover_text(
                        "Enable is per node — the buttons are on the node rows below. \
                             E-stop is the only thing that acts on everything at once.",
                    );
                });
                ui.horizontal(|ui| {
                    if estopped && ui.button("Clear E-Stop").clicked() {
                        self.station.clear_estop();
                        self.status =
                            "E-stop cleared. Everything stays disabled until you enable it.".into();
                    }
                });
            });

            // Pushed right so it never crowds the buttons on a narrow window.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(theme::value_muted(crate::version::short())).frame(false),
                    )
                    .on_hover_text("What this build is, and where everything it uses lives")
                    .clicked()
                {
                    self.about = !self.about;
                }
                ui.separator();
                ui.label(theme::value_muted(format!(
                    "ui {:>4} ms",
                    self.station.ui_stale_ms()
                )))
                .on_hover_text(
                    "How long since this window last drew. If it exceeds the limit in \
                     Settings, every node is faulted and disabled.",
                );
            });
        });
    }

    /// What this is, what it speaks, and where it keeps things.
    ///
    /// Folded away until asked for. It exists so that a build in someone
    /// else's hands can account for itself without a manual.
    fn about_panel(&mut self, ui: &mut egui::Ui) {
        if !self.about {
            return;
        }
        egui::Frame::new()
            .fill(theme::PANEL)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("MicroRIO Driver Station")
                            .size(15.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("close").clicked() {
                            self.about = false;
                        }
                    });
                });
                ui.label(
                    RichText::new(
                        "Telemetry and control for microcontrollers over plain UDP. \
                         Channels are named once at connect and carried as numbered \
                         slots after that, so the board spends its memory on your \
                         sketch rather than on text.",
                    )
                    .small()
                    .color(theme::MUTED),
                );
                ui.add_space(6.0);

                let logs = self
                    .station
                    .log_path
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| "(recording off)".into());
                let profile = self
                    .station
                    .profile_path
                    .lock()
                    .unwrap()
                    .clone()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unsaved)".into());
                egui::Grid::new("about")
                    .num_columns(2)
                    .spacing([14.0, 3.0])
                    .show(ui, |ui| {
                        for (k, v) in [
                            ("version", crate::version::VERSION.to_string()),
                            ("build", crate::version::COMMIT.to_string()),
                            ("built", crate::version::DATE.to_string()),
                            ("protocol", format!("v{}", microrio_proto::PROTO_VERSION)),
                            ("licence", env!("CARGO_PKG_LICENSE").to_string()),
                            ("source", env!("CARGO_PKG_REPOSITORY").to_string()),
                            ("profile", profile),
                            ("this session's log", logs),
                        ] {
                            ui.label(theme::value_muted(k));
                            ui.label(RichText::new(v).monospace().color(theme::TEXT));
                            ui.end_row();
                        }
                    });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "A board speaks this protocol through the MicroRIO Arduino \
                         library, which is published alongside the station.",
                    )
                    .small()
                    .italics()
                    .color(theme::MUTED),
                );
            });
        ui.add_space(3.0);
    }

    /// A stall faulted the machine; say so until it is acknowledged.
    fn stall_banner(&mut self, ui: &mut egui::Ui) {
        let Some(ms) = self.station.pending_stall_ms() else {
            return;
        };
        ui.add_space(3.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(70, 40, 10))
            .stroke(egui::Stroke::new(1.0, theme::WARN))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("WINDOW STALLED FOR {ms} ms"))
                            .strong()
                            .color(theme::WARN),
                    );
                    ui.label(
                        RichText::new(
                            "— every node was faulted and has been left disabled. \
                             A machine busy enough to stop this window drawing will do \
                             this. Re-enable \
                             deliberately.",
                        )
                        .small()
                        .color(theme::TEXT),
                    );
                    if ui.button("Dismiss").clicked() {
                        self.station.clear_stall();
                    }
                });
            });
        ui.add_space(3.0);
    }

    /// Open, save and save-as for the settings file.
    ///
    /// The profile is an ordinary file you point at, so one build can serve
    /// several machines: keep a profile per robot, per bench, per site.
    fn profile_bar(&mut self, ui: &mut egui::Ui) {
        let current = self.station.profile_path.lock().unwrap().clone();

        ui.horizontal_wrapped(|ui| {
            ui.label(theme::header("profile"));

            match &current {
                Some(p) => {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.display().to_string());
                    ui.label(theme::value(name))
                        .on_hover_text(p.display().to_string());
                }
                None => {
                    ui.label(theme::value_muted("(unsaved)"));
                }
            }

            ui.separator();

            let busy = self.dialog_open;
            if ui.add_enabled(!busy, egui::Button::new("Open…")).clicked() {
                self.ask_file(
                    || {
                        rfd::FileDialog::new()
                            .add_filter("MicroRIO profile", &["toml"])
                            .set_title("Open a station profile")
                            .pick_file()
                    },
                    FileJob::Open,
                    "Open cancelled.",
                );
            }

            // Save as opens on the current file, so saving in place is one
            // click more than a Save button and there is only one way to save.
            if ui
                .add_enabled(!busy, egui::Button::new("Save as…"))
                .clicked()
            {
                let suggested = current
                    .as_ref()
                    .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "station.toml".to_string());
                let dir = current
                    .as_ref()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                self.ask_file(
                    move || {
                        let mut dialog = rfd::FileDialog::new()
                            .add_filter("MicroRIO profile", &["toml"])
                            .set_file_name(suggested)
                            .set_title("Save station profile as");
                        if let Some(dir) = dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog.save_file()
                    },
                    FileJob::SaveProfile,
                    "Save cancelled.",
                );
            }

            if busy {
                ui.label(
                    RichText::new("file dialog open…")
                        .small()
                        .color(theme::MUTED),
                )
                .on_hover_text(
                    "It is a separate window. The station keeps running and keeps \
                     driving while it is up.",
                );
            }
        });
    }

    /// Open a file dialog on a worker thread, and act on the answer later.
    ///
    /// A native file dialog blocks whichever thread opens it. Opened from the
    /// UI thread it stops the window rendering, which the station correctly
    /// reads as a stall and answers by disabling every node — so picking a
    /// profile mid-test used to stop the machine. Off the UI thread, the
    /// heartbeat carries on and nothing is interrupted.
    fn ask_file(
        &mut self,
        pick: impl FnOnce() -> Option<std::path::PathBuf> + Send + 'static,
        done: impl FnOnce(std::path::PathBuf) -> FileJob + Send + 'static,
        cancelled: &str,
    ) {
        let slot = self.file_job.clone();
        let cancelled = cancelled.to_string();
        self.dialog_open = true;
        std::thread::spawn(move || {
            let job = match pick() {
                Some(path) => done(path),
                None => FileJob::Cancelled(cancelled),
            };
            *slot.lock().unwrap() = Some(job);
        });
    }

    /// Apply whatever the dialog came back with. Called once per frame.
    fn take_file_job(&mut self) {
        let Some(job) = self.file_job.lock().unwrap().take() else {
            return;
        };
        self.dialog_open = false;
        match job {
            FileJob::Cancelled(msg) => self.status = msg,
            FileJob::Open(path) => match config::load(&path) {
                Ok(profile) => {
                    let nodes = profile.nodes.len();
                    self.tz_input = profile.timezone.clone();
                    self.station.adopt_profile(profile, Some(path.clone()));
                    self.selected_node = self
                        .station
                        .profile
                        .lock()
                        .unwrap()
                        .nodes
                        .first()
                        .map(|n| n.id)
                        .unwrap_or(1);
                    self.chart_rt.clear();
                    self.status = format!(
                        "Opened {} — {nodes} node(s), links restarted, left disabled.",
                        path.display()
                    );
                }
                Err(e) => self.status = format!("Could not open that profile: {e}"),
            },
            FileJob::SaveProfile(path) => {
                self.save_profile_to(&path);
                *self.station.profile_path.lock().unwrap() = Some(path);
            }
            FileJob::Export {
                path,
                from_us,
                to_us,
                what,
            } => self.write_export(path, from_us, to_us, what),
        }
    }

    fn save_profile_to(&mut self, path: &std::path::Path) {
        let snapshot = self.station.profile.lock().unwrap().clone();
        match config::save(path, &snapshot) {
            Ok(()) => {
                self.status = format!(
                    "Saved to {} — nodes, scaling, control limits, charts, links and \
                     joystick mappings.",
                    path.display()
                )
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// One block per node: select it, see it, enable or disable it, and
    /// read whatever has been pinned to it.
    ///
    /// Enable lives here rather than in the safety bar because it is a
    /// per-node decision — the safety bar only carries what is global. Rows
    /// wrap rather than run off the edge, so the header still reads with the
    /// window snapped to half a screen.
    fn node_bar(&mut self, ui: &mut egui::Ui) {
        let nodes: Vec<config::NodeConfig> = self.station.profile.lock().unwrap().nodes.clone();
        if nodes.is_empty() {
            ui.label(theme::value_muted(
                "No nodes configured. Add one in Settings.",
            ));
            return;
        }
        let views = self.station.views.lock().unwrap().clone();
        let estopped = self
            .station
            .estop
            .load(std::sync::atomic::Ordering::Relaxed);
        let mut rate_edits: Vec<(u16, u16)> = Vec::new();

        for n in &nodes {
            let view = views.get(&n.id).cloned().unwrap_or_default();
            let on = self.station.is_node_enabled(n.id);
            let viewing = self.station.is_viewing(n.id);
            let selected = self.selected_node == n.id;

            egui::Frame::new()
                .fill(if selected { theme::PANEL } else { theme::BG })
                .stroke(egui::Stroke::new(
                    1.0,
                    if selected {
                        theme::BORDER_BRIGHT
                    } else {
                        theme::BORDER
                    },
                ))
                .inner_margin(egui::Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        theme::lamp(ui, view.connected, 10.0);
                        ui.selectable_value(
                            &mut self.selected_node,
                            n.id,
                            RichText::new(format!("{} #{}", n.label, n.id)).strong(),
                        );

                        // Watching: say so once, offer the way back, and
                        // stop there. A station that is not driving has
                        // nothing to enable and nothing to warn about.
                        if viewing {
                            ui.label(
                                RichText::new(" VIEW ONLY ")
                                    .strong()
                                    .color(Color32::WHITE)
                                    .background_color(Color32::from_rgb(40, 70, 120)),
                            )
                            .on_hover_text(
                                "Another station is driving this node. You can see \
                                 everything it reports and record it; you cannot set \
                                 anything. The E-stop still works.",
                            );
                            if ui
                                .button("take control")
                                .on_hover_text(
                                    "Ask for this node. It is granted once the other \
                                     station stops talking to it; until then you stay a \
                                     watcher.",
                                )
                                .clicked()
                            {
                                self.station.set_viewing(n.id, false);
                                self.status = format!(
                                    "Asking for {}. If another station still has it, this \
                                     one goes back to watching.",
                                    n.label
                                );
                            }
                        }

                        // One button that says what the node is, and changes
                        // it. Enabled reads as ordinary bright text; disabled
                        // is greyed out, so the state is visible at a glance.
                        let (text, fill, colour) = if on {
                            (" ENABLED ", Color32::from_rgb(70, 70, 70), Color32::WHITE)
                        } else {
                            (" DISABLED ", theme::BG, theme::MUTED)
                        };
                        let toggle = egui::Button::new(RichText::new(text).strong().color(colour))
                            .fill(fill)
                            .stroke(egui::Stroke::new(
                                1.0,
                                if on { Color32::WHITE } else { theme::BORDER },
                            ))
                            .min_size(egui::vec2(96.0, 22.0));
                        if ui
                            .add_enabled(!viewing && (on || !estopped), toggle)
                            .on_hover_text(if on {
                                "Enabled. Click to disable this node."
                            } else if estopped {
                                "E-stop is latched. Clear it before enabling anything."
                            } else {
                                "Disabled. Click to enable this node."
                            })
                            .clicked()
                        {
                            self.station.set_node_enabled(n.id, !on);
                            self.status =
                                format!("{} {}.", n.label, if on { "disabled" } else { "enabled" });
                        }

                        match view.node_state {
                            Some(s) => {
                                let c = match s {
                                    State::Enabled => theme::OK,
                                    State::Fault | State::Estop => theme::BAD,
                                    State::Disabled => theme::MUTED,
                                };
                                ui.label(RichText::new(format!("node {s:?}")).monospace().color(c));
                            }
                            None => {
                                ui.label(theme::value_muted("node ?"));
                            }
                        }

                        // One rate per node: telemetry, control and the log.
                        ui.separator();
                        let mut hz = n.rate_hz;
                        if ui
                            .add(num(&mut hz, 1.0).range(1..=1000).suffix(" Hz"))
                            .on_hover_text(
                                "How often this node sends telemetry, and so how many rows \
                                 a second the log gets. The station sends control at the \
                                 same rate. It changes nothing about how fast the board's \
                                 own loop runs. Takes effect immediately.",
                            )
                            .changed()
                        {
                            rate_edits.push((n.id, hz.max(1)));
                        }
                        if let Some(node_hz) = view.node_rate_hz {
                            if node_hz != n.rate_hz {
                                ui.label(
                                    RichText::new(format!("node {node_hz} Hz"))
                                        .monospace()
                                        .color(theme::WARN),
                                )
                                .on_hover_text(
                                    "The rate the node says it is actually sending at. It \
                                     differs from what was asked for: the board can only \
                                     time in whole milliseconds, so very high rates round.",
                                );
                            }
                        }

                        // Link health is always shown, pinned items or not.
                        ui.separator();
                        let rtt = match view.rtt_us {
                            Some(us) => format!("rtt {:.1} ms", us as f64 / 1000.0),
                            None => "rtt —".into(),
                        };
                        ui.label(theme::value_muted(rtt)).on_hover_text(format!(
                            "{}:{}  ·  rx {} / tx {}",
                            n.ip, n.port, view.rx_frames, view.tx_frames
                        ));
                        let (loss_text, loss_colour) = match view.record_loss_pct {
                            Some(l) if l >= 10.0 => (format!("loss {l:.1}%"), theme::BAD),
                            Some(l) if l >= 0.05 => (format!("loss {l:.1}%"), theme::WARN),
                            Some(l) => (format!("loss {l:.1}%"), theme::MUTED),
                            None => ("loss —".into(), theme::MUTED),
                        };
                        let history = view.schema.as_ref().map(|s| s.info.history).unwrap_or(1);
                        ui.label(RichText::new(loss_text).monospace().color(loss_colour))
                            .on_hover_text(if history > 1 {
                                format!(
                                    "Telemetry records that never reached the log. Frames the \
                                     network dropped but the node resent do not count: it \
                                     keeps its last {history}, and {} have been recovered \
                                     recently.",
                                    view.records_recovered
                                )
                            } else {
                                "Telemetry records that never reached the log. This board \
                                 keeps no history to resend from, so every dropped frame \
                                 is a missing row."
                                    .to_string()
                            });

                        // Being fought over is worth saying loudly: it is the
                        // one fault that looks like a flaky network and is not.
                        match view.contest {
                            // Already said, and acted on, by the badge above.
                            _ if viewing => {}
                            crate::station::Contest::Refused => {
                                ui.label(
                                    RichText::new("NOT IN CONTROL")
                                        .monospace()
                                        .strong()
                                        .color(theme::BAD),
                                )
                                .on_hover_text(
                                    "Another station owns this node and is refusing this \
                                     one's control. A node answers to one station at a \
                                     time; this station can see it but is not driving it. \
                                     Close the other station, and this one takes over \
                                     within a watchdog period.",
                                );
                            }
                            crate::station::Contest::Shared => {
                                ui.label(
                                    RichText::new("SECOND STATION")
                                        .monospace()
                                        .strong()
                                        .color(theme::WARN),
                                )
                                .on_hover_text(
                                    "Another station is talking to this node. This one is \
                                     driving it, but the node's telemetry is being split \
                                     between the two, which looks exactly like packet \
                                     loss. Close the other one.",
                                );
                            }
                            crate::station::Contest::Alone => {}
                        }

                        // Node-to-station watchdog: the operator is told, and
                        // nothing else happens.
                        if view.last_rx_unix_ms > 0 && !viewing {
                            let quiet =
                                crate::station::now_ms().saturating_sub(view.last_rx_unix_ms);
                            if quiet > n.watchdog_ms {
                                ui.label(
                                    RichText::new(format!(
                                        "NO COMMS {:.1} s",
                                        quiet as f64 / 1000.0
                                    ))
                                    .monospace()
                                    .strong()
                                    .color(theme::BAD),
                                )
                                .on_hover_text(format!(
                                    "Nothing has been heard from this node for longer than its \
                                     {} ms watchdog. The network, its power, or the board \
                                     itself. Set the time in Settings -> Nodes.",
                                    n.watchdog_ms
                                ));
                            }
                        }
                        // Station-to-node watchdog, as the node reports it.
                        if view.node_flags & microrio_proto::flag::WATCHDOG_TRIPPED != 0 {
                            ui.label(
                                RichText::new("NODE WATCHDOG")
                                    .monospace()
                                    .color(theme::WARN),
                            )
                            .on_hover_text(
                                "The node says it stopped hearing this station for longer \
                                     than its own watchdog allows, and has gone to its failstate. \
                                     It clears as soon as control frames reach it again.",
                            );
                        }
                        if let Some(reason) = &view.link_fault {
                            ui.label(
                                RichText::new("LINK SOURCE LOST")
                                    .monospace()
                                    .color(theme::BAD),
                            )
                            .on_hover_text(format!(
                                "A cross-node link is driving this board from {reason}, \
                                     and that source has stopped reporting. The node is being \
                                     held in FAULT rather than driven from a stale value."
                            ));
                        }
                        if let Some(e) = &view.error {
                            ui.label(RichText::new(e).small().color(theme::BAD));
                        }
                    });

                    if n.header_items() > 0 {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                            self.header_pins(ui, n, &view);
                        });
                    }
                });
            ui.add_space(2.0);
        }

        if !rate_edits.is_empty() {
            let mut p = self.station.profile.lock().unwrap();
            for (id, hz) in rate_edits {
                if let Some(n) = p.nodes.iter_mut().find(|n| n.id == id) {
                    n.rate_hz = hz;
                }
            }
        }
    }
}

// ----------------------------------------------------------------- operator

impl App {
    /// Telemetry, control and a chart on one page.
    ///
    /// This is where an operator spends the session, so the chart lives here
    /// rather than behind a tab. Above `WIDE_PX` the two grids sit side by
    /// side; below it they stack, because two grids of seven columns each
    /// cannot share 960 pixels without colliding.
    fn operator_tab(&mut self, ui: &mut egui::Ui) {
        let Some(cfg) = self.node_cfg() else {
            ui.label("No nodes configured. Add one in Settings.");
            return;
        };
        let view = self
            .station
            .views
            .lock()
            .unwrap()
            .get(&cfg.id)
            .cloned()
            .unwrap_or_default();

        let Some(schema) = view.schema.clone() else {
            ui.add_space(10.0);
            ui.label(no_schema_note(&cfg, &view));
            ui.label(
                RichText::new(
                    "Nothing is displayed until the node answers, because the station \
                     will not guess at a layout it has not been told.",
                )
                .italics()
                .color(theme::MUTED),
            );
            return;
        };

        // The grids take only what they need and the chart takes the rest.
        // How much they need is measured from the last frame rather than
        // guessed at a fixed fraction, which on a wide screen left a band of
        // nothing between the two.
        let total = ui.available_height();
        const MIN_CHART: f32 = 170.0;
        let grids_h = if self.grids_h > 0.0 {
            self.grids_h.min(total - MIN_CHART - 40.0).max(90.0)
        } else {
            (total * 0.5).clamp(120.0, 600.0)
        };
        let chart_h = (total - grids_h - 40.0).max(MIN_CHART);
        let wide = ui.available_width() >= WIDE_PX;

        ui.allocate_ui(egui::vec2(ui.available_width(), grids_h), |ui| {
            let out = egui::ScrollArea::vertical()
                .id_salt("opgrids")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.groups_panel(ui, &schema, &view, &cfg);
                    if !cfg.groups.is_empty() {
                        ui.horizontal(|ui| {
                            ui.checkbox(
                                &mut self.show_all_channels,
                                "grids show grouped channels too",
                            )
                            .on_hover_text(
                                "Off, the grids below hold only what no group has, so \
                                     nothing appears twice. On, they list everything — which \
                                     is where scaling and limits are edited.",
                            );
                        });
                    }
                    ui.add_space(4.0);
                    if wide {
                        ui.columns(2, |cols| {
                            self.telemetry_panel(&mut cols[0]);
                            self.control_panel(&mut cols[1]);
                        });
                    } else {
                        self.telemetry_panel(ui);
                        ui.add_space(12.0);
                        ui.separator();
                        self.control_panel(ui);
                    }
                });
            self.grids_h = out.content_size.y + 6.0;
        });

        ui.separator();
        let chart = self.station.profile.lock().unwrap().operator_chart.clone();
        if let Some(chart) = chart {
            let mut edited = chart.clone();
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::header("chart"));
                if self.chart_controls(ui, &mut edited, "op", false) {
                    self.station.profile.lock().unwrap().operator_chart = Some(edited.clone());
                }
            });
            self.draw_chart(ui, &edited, "op", chart_h - 30.0);
        }
    }

    /// Give the Operator page its chart as soon as a node has told us what
    /// it has.
    ///
    /// Also migrates two older shapes of profile. One kept the Operator
    /// chart as the first of the Charts tab's list, which tied the two
    /// together: it moves out into its own slot. An older one still carried
    /// `plot = true` on channels and no charts at all, and those channels are
    /// exactly what that operator wanted plotted.
    fn ensure_charts(&mut self) {
        let mut p = self.station.profile.lock().unwrap();
        if p.operator_chart.is_some() {
            return;
        }
        if !p.charts.is_empty() {
            let first = p.charts.remove(0);
            p.operator_chart = Some(first);
            return;
        }
        drop(p);
        let Some(cfg) = self.node_cfg() else { return };
        let schema = self
            .station
            .views
            .lock()
            .unwrap()
            .get(&cfg.id)
            .and_then(|v| v.schema.clone());
        let Some(schema) = schema else { return };
        let node_id = cfg.id;
        let armed: Vec<Axis> = schema
            .channels
            .iter()
            .filter(|c| c.dir == Dir::Telemetry && plottable(c.ctype))
            .filter(|c| cfg.channels.get(&c.name).map(|cc| cc.plot).unwrap_or(false))
            .map(|c| Axis::channel(node_id, &c.name))
            .collect();
        // Nothing was armed either, so show the first couple of numeric
        // channels. An empty chart teaches nobody anything.
        let y = if armed.is_empty() {
            schema
                .channels
                .iter()
                .filter(|c| c.dir == Dir::Telemetry && plottable(c.ctype))
                .take(2)
                .map(|c| Axis::channel(node_id, &c.name))
                .collect()
        } else {
            armed
        };
        let mut p = self.station.profile.lock().unwrap();
        if p.operator_chart.is_none() {
            p.operator_chart = Some(ChartConfig {
                title: "live".into(),
                y,
                ..Default::default()
            });
        }
    }

    /// Telemetry for every node, each under its own heading, with the
    /// cross-node links last.
    ///
    /// Telemetry is read, not driven, so there is no reason to show one board
    /// at a time: watching two boards against each other is the whole point of
    /// a multi-node station. Control stays on the selected node, because
    /// driving the wrong board is a different kind of mistake.
    fn telemetry_panel(&mut self, ui: &mut egui::Ui) {
        let nodes: Vec<config::NodeConfig> = self.station.profile.lock().unwrap().nodes.clone();
        let views = self.station.views.lock().unwrap().clone();
        let many = nodes.len() > 1;

        ui.label(theme::header("Telemetry  ·  node -> station"));
        ui.add_space(2.0);
        for n in &nodes {
            let view = views.get(&n.id).cloned().unwrap_or_default();
            if many {
                self.node_heading(ui, n, &view);
            }
            match view.schema.clone() {
                Some(schema) => self.telemetry_grid(ui, &schema, &view, n),
                None => {
                    ui.label(theme::value_muted(no_schema_note(n, &view)));
                }
            }
        }
        self.passthrough_grid(ui, &nodes, &views);
    }

    /// Control for every node, each under its own heading.
    ///
    /// Every node gets a complete control frame every tick, so there is no
    /// such thing as the station controlling one board: this is the whole
    /// output, listed per node because the channels belong to boards.
    fn control_panel(&mut self, ui: &mut egui::Ui) {
        let nodes: Vec<config::NodeConfig> = self.station.profile.lock().unwrap().nodes.clone();
        let views = self.station.views.lock().unwrap().clone();
        let many = nodes.len() > 1;

        ui.label(theme::header("Control  ·  station -> nodes"));
        ui.add_space(2.0);
        for n in &nodes {
            let view = views.get(&n.id).cloned().unwrap_or_default();
            if many {
                self.node_heading(ui, n, &view);
            }
            if self.station.is_viewing(n.id) {
                ui.label(
                    RichText::new("view only — another station is driving this node")
                        .small()
                        .color(Color32::from_rgb(120, 160, 220)),
                );
            }
            match view.schema.clone() {
                Some(schema) => self.control_grid(ui, &schema, &view, n),
                None => {
                    ui.label(theme::value_muted("waiting for this node's schema…"));
                }
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "These are what you send. Nothing fills them in but you, the joystick, \
                 or a cross-node link — a value left alone sits where you put it. The \
                 wire column is the number that actually goes out.",
            )
            .italics()
            .small()
            .color(theme::MUTED),
        );
    }

    /// A node's name over its slice of a panel. Clicking it selects the node.
    fn node_heading(
        &mut self,
        ui: &mut egui::Ui,
        n: &config::NodeConfig,
        view: &crate::station::NodeView,
    ) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            theme::lamp(ui, view.connected, 9.0);
            let r = ui.selectable_label(
                self.selected_node == n.id,
                RichText::new(format!("{} #{}", n.label, n.id))
                    .strong()
                    .color(theme::HEADER_TEXT),
            );
            if r.clicked() {
                self.selected_node = n.id;
            }
            r.on_hover_text("Select this node. Settings and the Clock tab follow it.");
        });
    }

    /// Cross-node links: one board's telemetry driving another's control.
    ///
    /// It belongs on the telemetry side because it is a reading, not a value
    /// the operator sets — and it is the one number that appears on neither
    /// node's own grid as itself.
    fn passthrough_grid(
        &mut self,
        ui: &mut egui::Ui,
        nodes: &[config::NodeConfig],
        views: &std::collections::HashMap<u16, crate::station::NodeView>,
    ) {
        let links: Vec<CrossLink> = self
            .station
            .profile
            .lock()
            .unwrap()
            .crosslinks
            .iter()
            .filter(|l| l.active)
            .cloned()
            .collect();
        if links.is_empty() {
            return;
        }
        let label_of = |id: u16| {
            nodes
                .iter()
                .find(|n| n.id == id)
                .map(|n| n.label.clone())
                .unwrap_or_else(|| format!("#{id}"))
        };

        ui.add_space(8.0);
        ui.label(theme::header("pass-through  ·  node -> node"));
        egui::Grid::new("passthrough")
            .num_columns(5)
            .striped(true)
            .spacing([8.0, 3.0])
            .show(ui, |ui| {
                for h in ["from", "reading", "->", "to", "sent"] {
                    ui.label(theme::header(h));
                }
                ui.end_row();
                for l in &links {
                    let raw = crate::station::crosslink_value(views, l);
                    ui.label(
                        RichText::new(format!("{}·{}", label_of(l.from_node), l.from_channel))
                            .monospace()
                            .color(theme::TEXT),
                    );
                    match raw {
                        Some(v) => ui.label(theme::value(format!("{v:.4}"))),
                        None => ui.label(RichText::new("no source").color(theme::BAD)),
                    };
                    ui.label(theme::value_muted("->"));
                    ui.label(
                        RichText::new(format!("{}·{}", label_of(l.to_node), l.to_channel))
                            .monospace()
                            .color(theme::HEADER_TEXT),
                    );
                    match raw {
                        Some(v) => ui.label(theme::value(format!("{:.4}", l.apply(v)))),
                        None => ui.label(RichText::new("FAULT").monospace().color(theme::BAD)),
                    };
                    ui.end_row();
                }
            });
        ui.label(
            RichText::new(
                "Scaled and limited on the way across. A source that goes quiet faults \
                 the destination node rather than leaving it on a stale value.",
            )
            .italics()
            .small()
            .color(theme::MUTED),
        );
    }

    fn telemetry_grid(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Schema,
        view: &crate::station::NodeView,
        cfg: &config::NodeConfig,
    ) {
        if view.telemetry.is_none() {
            ui.label(theme::value_muted("no telemetry yet"));
            return;
        }
        let on_chart = self.operator_plotted(cfg.id);
        egui::Grid::new(("telem", cfg.id))
            .num_columns(rows::ROW_COLUMNS)
            .striped(true)
            .spacing([6.0, 3.0])
            .show(ui, |ui| {
                for h in rows::TELEM_HEADERS {
                    ui.label(theme::header(h));
                }
                ui.end_row();

                for c in schema.channels.iter().filter(|c| c.dir == Dir::Telemetry) {
                    let r = config::ChanRef::telemetry(&c.name);
                    if cfg.is_hidden(&r) || (!self.show_all_channels && cfg.grouped(&r)) {
                        continue;
                    }
                    let title = cfg
                        .channels
                        .get(&c.name)
                        .map(|cc| cc.title(&c.name).to_string())
                        .unwrap_or_else(|| c.name.clone());
                    chan_name(ui, cfg.id, r, &title, theme::TEXT);
                    self.telemetry_cells(
                        ui,
                        cfg,
                        view,
                        c,
                        on_chart.contains(&c.name),
                        rows::Style::grid(),
                    );
                    ui.end_row();
                }
            });
    }

    fn control_grid(
        &mut self,
        ui: &mut egui::Ui,
        schema: &Schema,
        view: &crate::station::NodeView,
        cfg: &config::NodeConfig,
    ) {
        let driven = self.driven(cfg.id);
        egui::Grid::new("ctrl")
            .num_columns(rows::ROW_COLUMNS)
            .striped(true)
            .spacing([6.0, 3.0])
            .show(ui, |ui| {
                for h in rows::CTRL_HEADERS {
                    ui.label(theme::header(h));
                }
                ui.end_row();

                let viewing = self.station.is_viewing(cfg.id);
                for c in schema.channels.iter().filter(|c| c.dir == Dir::Control) {
                    let r = config::ChanRef::control(&c.name);
                    if cfg.is_hidden(&r) || (!self.show_all_channels && cfg.grouped(&r)) {
                        continue;
                    }
                    let linked = viewing || driven.iter().any(|d| *d == c.name);
                    let title = cfg
                        .controls
                        .get(&c.name)
                        .map(|cc| cc.title(&c.name).to_string())
                        .unwrap_or_else(|| c.name.clone());
                    let name = chan_name(
                        ui,
                        cfg.id,
                        r,
                        &title,
                        rows::name_colour(Dir::Control, linked),
                    );
                    if viewing {
                        name.on_hover_text(
                            "Another station is driving this node. Use take control on \
                             the node row above if you want it.",
                        );
                    } else if linked {
                        name.on_hover_text(
                            "Driven by a cross-node link, set up in Settings. \
                             What you type here is ignored for this channel.",
                        );
                    }
                    self.control_cells(ui, cfg, schema, view, c, linked, rows::Style::grid());
                    ui.end_row();
                }
            });
    }

    /// Control channels another node is driving, which are not the
    /// operator's to set.
    fn driven(&self, node_id: u16) -> Vec<String> {
        self.station
            .profile
            .lock()
            .unwrap()
            .crosslinks
            .iter()
            .filter(|l| l.active && l.to_node == node_id)
            .map(|l| l.to_channel.clone())
            .collect()
    }
}

/// A channel's name in a grid. Once there are groups to put it in, the name
/// is also the handle it is dragged by.
fn chan_name(
    ui: &mut egui::Ui,
    node_id: u16,
    chan: config::ChanRef,
    title: &str,
    colour: Color32,
) -> egui::Response {
    ui.dnd_drag_source(
        egui::Id::unique(("grid-drag", node_id, chan.dir as u8, chan.channel.clone())),
        DragChan {
            node_id,
            chan,
            from_group: None,
        },
        |ui| ui.label(RichText::new(title).color(colour)),
    )
    .response
}

/// A reading outside its warning band is drawn in the warning colour. That is
/// all a threshold does.
fn warned(text: RichText, warn: bool) -> RichText {
    if warn {
        text.color(theme::WARN)
    } else {
        text
    }
}

fn plottable(t: ChanType) -> bool {
    matches!(t, ChanType::Bool | ChanType::I32 | ChanType::F32)
}

fn fmt_scaled(v: f64, cfg: &ChannelConfig) -> String {
    if cfg.slope == 1.0 && cfg.offset == 0.0 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.3}")
    }
}

// ------------------------------------------------------------------- charts

impl App {
    /// Every telemetry channel on every node that could go on an axis,
    /// paired with a label for the dropdown.
    fn available_axes(&self) -> Vec<(String, Axis)> {
        let nodes = self.station.profile.lock().unwrap().nodes.clone();
        let views = self.station.views.lock().unwrap().clone();
        let mut out = vec![
            ("time · seconds ago".to_string(), Axis::elapsed()),
            (
                "time · clock".to_string(),
                Axis {
                    kind: AxisKind::Clock,
                    node_id: 0,
                    channel: String::new(),
                },
            ),
        ];
        for n in &nodes {
            let Some(schema) = views.get(&n.id).and_then(|v| v.schema.clone()) else {
                continue;
            };
            for c in schema
                .channels
                .iter()
                .filter(|c| c.dir == Dir::Telemetry && plottable(c.ctype))
            {
                out.push((
                    format!("{} · {}", n.label, c.name),
                    Axis::channel(n.id, &c.name),
                ));
            }
        }
        out
    }

    /// The row of pickers above a chart. Returns true if anything saved
    /// changed; pause and clear are runtime only and change nothing on disk.
    fn chart_controls(
        &mut self,
        ui: &mut egui::Ui,
        chart: &mut ChartConfig,
        key: &str,
        with_title: bool,
    ) -> bool {
        let options = self.available_axes();
        let mut changed = false;

        if with_title {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut chart.title)
                        .desired_width(110.0)
                        .hint_text("name"),
                )
                .changed();
        }

        // X and Y are picked the same way, from the same list, because they
        // hold the same kind of thing: time, or a channel on any node.
        ui.label(theme::header("x"));
        changed |= axis_picker(ui, format!("x{key}"), &mut chart.x, &options);

        ui.label(theme::header("y"));
        let mut drop_y = None;
        for (i, y) in chart.y.iter_mut().enumerate() {
            changed |= axis_picker(ui, format!("y{key}_{i}"), y, &options);
            if ui
                .small_button("×")
                .on_hover_text("Remove this trace")
                .clicked()
            {
                drop_y = Some(i);
            }
        }
        if let Some(i) = drop_y {
            chart.y.remove(i);
            changed = true;
        }
        if ui.small_button("+").on_hover_text("Add a trace").clicked() {
            let next = options
                .iter()
                .find(|(_, a)| a.kind == AxisKind::Channel && !chart.y.contains(a))
                .map(|(_, a)| a.clone())
                .unwrap_or_else(Axis::elapsed);
            chart.y.push(next);
            changed = true;
        }

        ui.separator();
        // A window only means something when one axis is time; for an X-Y
        // plot it is still how much history is drawn.
        egui::ComboBox::from_id_salt(format!("win{key}"))
            .selected_text(RichText::new(fmt_window(chart.window_s)).monospace())
            .width(70.0)
            .show_ui(ui, |ui| {
                for w in WINDOWS {
                    if ui
                        .selectable_label(chart.window_s == w, fmt_window(w))
                        .clicked()
                    {
                        chart.window_s = w;
                        changed = true;
                    }
                }
            })
            .response
            .on_hover_text("How much history the chart shows");

        changed |= ui
            .checkbox(&mut chart.invert_y, "flip Y")
            .on_hover_text("Draw Y downwards: larger values lower")
            .changed();
        changed |= ui
            .checkbox(&mut chart.autoscale, "auto Y")
            .on_hover_text("Fit the Y axis to what is on screen. Off, you type the limits.")
            .changed();
        if !chart.autoscale {
            changed |= ui.add(num(&mut chart.y_min, 0.1).prefix("min ")).changed();
            changed |= ui.add(num(&mut chart.y_max, 0.1).prefix("max ")).changed();
        }

        ui.separator();
        let newest = self.station.history.lock().unwrap().newest().unwrap_or(0.0);
        let rt = self.chart_rt.entry(key.to_string()).or_default();
        let paused = rt.paused_at.is_some();
        let panned = rt.pan_s > 0.0;
        if ui
            .add(egui::Button::new(if paused { "resume" } else { "pause" }).selected(paused))
            .on_hover_text(
                "Freeze the chart where it is so you can hover over it. Recording \
                 carries on underneath.",
            )
            .clicked()
        {
            rt.paused_at = if paused { None } else { Some(newest) };
        }
        if ui
            .button("clear")
            .on_hover_text("Start this chart afresh from now. The log is untouched.")
            .clicked()
        {
            rt.cleared_at = Some(rt.paused_at.unwrap_or(newest));
            rt.pan_s = 0.0;
        }
        if panned {
            // Dragged back in history. One button to catch up again, so the
            // chart can never be left somewhere it cannot be got out of.
            if ui
                .button(
                    RichText::new(format!(">> live  (−{})", fmt_ago(rt.pan_s))).color(theme::WARN),
                )
                .on_hover_text("Back to now")
                .clicked()
            {
                rt.pan_s = 0.0;
            }
        } else {
            ui.label(theme::value_muted("drag to look back"));
        }
        changed
    }

    /// Build one chart's traces out of history.
    ///
    /// Four shapes fall out of letting both axes be anything: time on X is the
    /// usual scope trace, time on Y is that trace rotated, a channel on each
    /// is an X-Y plot, and time on both is meaningless and drawn as nothing.
    fn chart_series(&self, chart: &ChartConfig, rt: ChartRt) -> Vec<(String, Vec<[f64; 2]>)> {
        let nodes = self.station.profile.lock().unwrap().nodes.clone();
        let scale_of = |a: &Axis| -> ChannelConfig {
            nodes
                .iter()
                .find(|n| n.id == a.node_id)
                .and_then(|n| n.channels.get(&a.channel).cloned())
                .unwrap_or_default()
        };
        let hist = self.station.history.lock().unwrap();
        // A paused chart's "now" is the moment it was paused; panning moves
        // the right-hand edge back from there.
        let now = rt.paused_at.unwrap_or_else(|| hist.newest().unwrap_or(0.0));
        let to = now - rt.pan_s;
        let from = (to - chart.window_s).max(rt.cleared_at.unwrap_or(f64::MIN));
        let epoch = self.station.started_unix_us as f64 / 1_000_000.0;
        // Station-relative seconds, rendered the way this axis wants them.
        let as_time = |kind: AxisKind, t: f64| match kind {
            AxisKind::Clock => epoch + t,
            _ => t - now,
        };
        // About two points per pixel column is all a screen can show.
        const BUCKETS: usize = 1000;

        let mut out = Vec::new();
        for y in &chart.y {
            match (chart.x.is_time(), y.is_time()) {
                (true, false) => {
                    let cc = scale_of(y);
                    let pts = history::thin(hist.range(y.node_id, &y.channel, from, to), BUCKETS)
                        .into_iter()
                        .map(|p| [as_time(chart.x.kind, p[0]), cc.scale(p[1])])
                        .collect();
                    out.push((y.label(), pts));
                }
                (false, true) => {
                    let cc = scale_of(&chart.x);
                    let pts = history::thin(
                        hist.range(chart.x.node_id, &chart.x.channel, from, to),
                        BUCKETS,
                    )
                    .into_iter()
                    .map(|p| [cc.scale(p[1]), as_time(y.kind, p[0])])
                    .collect();
                    out.push((chart.x.label(), pts));
                }
                (false, false) => {
                    let (cx, cy) = (scale_of(&chart.x), scale_of(y));
                    // Half a second of slack: enough to pair two boards
                    // running at different rates, little enough that a dead
                    // source leaves a gap instead of a straight line.
                    let pts = hist
                        .pairs_between(
                            (chart.x.node_id, &chart.x.channel),
                            (y.node_id, &y.channel),
                            from,
                            to,
                            0.5,
                        )
                        .into_iter()
                        .map(|p| [cx.scale(p[0]), cy.scale(p[1])])
                        .collect();
                    out.push((format!("{} vs {}", y.label(), chart.x.label()), pts));
                }
                (true, true) => {}
            }
        }
        out
    }

    fn draw_chart(&mut self, ui: &mut egui::Ui, chart: &ChartConfig, key: &str, height: f32) {
        if chart.y.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "No traces yet. Pick one with the + button above, or tick plot \
                     beside a channel in the telemetry grid.",
                )
                .italics()
                .color(theme::MUTED),
            );
            return;
        }
        if chart.x.is_time() && chart.y.iter().all(|y| y.is_time()) {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Time is on both axes, so there is nothing to draw. Pick a channel \
                     for one of them.",
                )
                .italics()
                .color(theme::WARN),
            );
            return;
        }

        let rt = self.chart_rt.get(key).copied().unwrap_or_default();
        let series = self.chart_series(chart, rt);
        let epoch_s = self.station.started_unix_us as f64 / 1_000_000.0;
        let now = rt
            .paused_at
            .unwrap_or_else(|| self.station.history.lock().unwrap().newest().unwrap_or(0.0));
        let xy = !chart.x.is_time();
        let zone = self.zone();
        let clock_x = chart.x.kind == AxisKind::Clock;
        let elapsed_x = chart.x.kind == AxisKind::Elapsed;
        let long = chart.window_s >= 120.0;

        let mut plot = Plot::new(format!("chart-{key}"))
            .height(height.max(80.0))
            .legend(Legend::default())
            .x_axis_label(if elapsed_x && long {
                "minutes:seconds ago".to_string()
            } else {
                chart.x.label()
            })
            .y_axis_label(if chart.y.len() == 1 {
                chart.y[0].label()
            } else {
                "scaled value".to_string()
            })
            // The readout below replaces the plot's own coordinate label.
            .show_x(false)
            .show_y(false)
            // Every kind of view-changing gesture is off, and the bounds are
            // set outright below. A chart that can be zoomed or flung by a
            // stray scroll is a chart that can be left unreadable with no way
            // back, which is exactly what happened.
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_axis_zoom_drag(false);

        if clock_x {
            // Raw unix seconds on an axis are unreadable; show the time of day
            // in the station's zone, which is what the whole zone setting is for.
            let z = zone.clone();
            plot = plot.x_axis_formatter(move |mark, _| z.fmt(mark.value as i64, "%H:%M:%S"));
        } else if elapsed_x {
            if long {
                // Label whole minutes only. Over a five-minute window every
                // gridline labelled is thirty numbers and no information.
                plot = plot.x_axis_formatter(|mark, _| {
                    let s = (-mark.value).max(0.0);
                    if (s % 60.0) > 0.5 {
                        return String::new();
                    }
                    format!("{}:00", (s / 60.0).round() as u64)
                });
            }
        }
        // Work out the view here rather than letting the plot decide, so it
        // is the same every frame and nothing can wander off screen.
        // egui_plot's axis always increases upwards, so an inverted Y is
        // drawn by negating the values and the tick labels. Everything the
        // operator reads — the hover box, the limits typed in — stays in the
        // channel's own numbers.
        let flip = if chart.invert_y { -1.0 } else { 1.0 };
        let series: Vec<(String, Vec<[f64; 2]>)> = if chart.invert_y {
            series
                .into_iter()
                .map(|(n, pts)| (n, pts.into_iter().map(|p| [p[0], -p[1]]).collect()))
                .collect()
        } else {
            series
        };
        let (y0, y1) = if chart.autoscale {
            history::fit(series.iter().flat_map(|(_, p)| p.iter().map(|q| q[1])))
        } else {
            history::span(flip * chart.y_min, flip * chart.y_max)
        };
        let (x0, x1) = if xy {
            history::fit(series.iter().flat_map(|(_, p)| p.iter().map(|q| q[0])))
        } else if clock_x {
            let right = epoch_s + now - rt.pan_s;
            (right - chart.window_s, right)
        } else {
            (-rt.pan_s - chart.window_s, -rt.pan_s)
        };

        if chart.invert_y {
            plot = plot.y_axis_formatter(|mark, _| fmt_tick(-mark.value));
        }

        let resp = plot.show(ui, |pui| {
            pui.set_plot_bounds(PlotBounds::from_min_max([x0, y0], [x1, y1]));
            for (i, (name, pts)) in series.iter().enumerate() {
                if xy {
                    // An X-Y plot is a cloud of samples, not a line through
                    // them in time order: joining them draws loops that are
                    // an artefact of sample order rather than of the data.
                    pui.points(
                        Points::new(name.clone(), PlotPoints::from(pts.clone()))
                            .color(theme::trace(i))
                            .radius(1.8),
                    );
                } else {
                    pui.line(
                        Line::new(name.clone(), PlotPoints::from(pts.clone()))
                            .color(theme::trace(i))
                            .width(1.5),
                    );
                }
            }
            let at = pui.pointer_coordinate();
            if let Some(at) = at {
                if !xy {
                    pui.vline(VLine::new("", at.x).color(theme::BORDER_BRIGHT).width(1.0));
                }
            }
            at
        });

        // Drag anywhere on the plot to look further back. The window keeps
        // its length — this slides it, it never stretches it.
        if !xy {
            let dx = resp.response.drag_delta().x as f64;
            if dx != 0.0 {
                let rt = self.chart_rt.entry(key.to_string()).or_default();
                rt.pan_s = history::panned(
                    rt.pan_s,
                    dx,
                    resp.response.rect.width() as f64,
                    chart.window_s,
                );
            }
        }

        // Hover readout: the moment under the pointer and every trace's value
        // there. On an X-Y plot "the moment" has no meaning, so it reads the
        // nearest sample instead.
        let Some(at) = resp.inner else { return };
        if !resp.response.hovered() {
            return;
        }
        resp.response
            .on_hover_cursor(if xy {
                egui::CursorIcon::Default
            } else {
                egui::CursorIcon::Grab
            })
            .on_hover_ui_at_pointer(|ui| {
                if xy {
                    ui.label(theme::value(format!(
                        "x {:.4}   y {:.4}",
                        at.x,
                        flip * at.y
                    )));
                    return;
                }
                let unix = if clock_x { at.x } else { epoch_s + now + at.x };
                let ago = if clock_x { epoch_s + now - at.x } else { -at.x };
                ui.label(theme::value(format!(
                    "{}  ·  {} ago",
                    zone.fmt_ms(unix),
                    fmt_ago(ago)
                )));
                egui::Grid::new("hover")
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (i, (name, pts)) in series.iter().enumerate() {
                            let idx = pts.partition_point(|p| p[0] < at.x);
                            let near = [idx.checked_sub(1), Some(idx)]
                                .into_iter()
                                .flatten()
                                .filter_map(|k| pts.get(k))
                                .min_by(|a, b| (a[0] - at.x).abs().total_cmp(&(b[0] - at.x).abs()));
                            ui.label(RichText::new("—").color(theme::trace(i)));
                            ui.label(RichText::new(name).monospace().color(theme::TEXT));
                            match near {
                                Some(p) => ui.label(theme::value(format!("{:.4}", flip * p[1]))),
                                None => ui.label(theme::value_muted("—")),
                            };
                            ui.end_row();
                        }
                    });
            });
    }

    fn charts_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    "These charts are separate from the one on the Operator page. Their \
                     setup is saved with the profile.",
                )
                .italics()
                .small()
                .color(theme::MUTED),
            );
            if ui.button("Add chart").clicked() {
                let mut p = self.station.profile.lock().unwrap();
                let n = p.charts.len() + 1;
                p.charts.push(ChartConfig {
                    title: format!("chart {n}"),
                    ..Default::default()
                });
            }
        });
        ui.separator();

        let charts = self.station.profile.lock().unwrap().charts.clone();
        if charts.is_empty() {
            ui.add_space(10.0);
            ui.label(theme::value_muted(
                "No charts yet. Add chart, above, makes one.",
            ));
            return;
        }

        // Each chart gets a fair share of the page, with a floor so that six
        // charts are scrollable rather than six slivers.
        let per = ((ui.available_height() / charts.len().min(2) as f32) - 70.0).max(160.0);
        let mut remove = None;
        let mut edits: Vec<(usize, ChartConfig)> = Vec::new();

        egui::ScrollArea::vertical()
            .id_salt("charts")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, chart) in charts.iter().enumerate() {
                    let key = format!("c{i}");
                    let mut edited = chart.clone();
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .stroke(egui::Stroke::new(1.0, theme::BORDER))
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if self.chart_controls(ui, &mut edited, &key, true) {
                                    edits.push((i, edited.clone()));
                                }
                                if ui.small_button("remove chart").clicked() {
                                    remove = Some(i);
                                }
                            });
                            self.draw_chart(ui, &edited, &key, per);
                        });
                    ui.add_space(8.0);
                }
            });

        let mut p = self.station.profile.lock().unwrap();
        for (i, c) in edits {
            if let Some(slot) = p.charts.get_mut(i) {
                *slot = c;
            }
        }
        if let Some(i) = remove {
            p.charts.remove(i);
            // Keys are positional; let the survivors start clean.
            self.chart_rt.retain(|k, _| !k.starts_with('c'));
        }
    }
}

/// Windows offered on every chart. Fifteen minutes is all history keeps.
const WINDOWS: [f64; 7] = [10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0];

fn fmt_window(s: f64) -> String {
    if s < 60.0 {
        format!("{s:.0} s")
    } else {
        format!("{:.0} min", s / 60.0)
    }
}

/// An axis tick, without the trailing zeros a plain `{}` would print.
fn fmt_tick(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.into()
    }
}

/// Seconds as `m:ss`, or plain seconds under a minute.
fn fmt_ago(s: f64) -> String {
    let s = s.max(0.0);
    if s < 60.0 {
        format!("{s:.1} s")
    } else {
        format!("{}:{:02}", (s / 60.0) as u64, (s % 60.0) as u64)
    }
}

/// A file dialog's answer, waiting to be acted on by the UI thread.
enum FileJob {
    Open(std::path::PathBuf),
    SaveProfile(std::path::PathBuf),
    Export {
        path: std::path::PathBuf,
        from_us: Option<u64>,
        to_us: Option<u64>,
        what: String,
    },
    /// Closed without picking anything, and what to say about it.
    Cancelled(String),
}

/// Pause and clear: per chart, for this session only.
#[derive(Debug, Default, Clone, Copy)]
struct ChartRt {
    /// Station seconds at which the chart was frozen.
    paused_at: Option<f64>,
    /// Nothing older than this is drawn.
    cleared_at: Option<f64>,
    /// How far back the view has been dragged, in seconds. 0 is live.
    pan_s: f64,
}

/// A numeric field that is typed into, never dragged.
///
/// Dragging a number is how a step or a limit gets changed by accident while
/// reaching for something else, so the drag is worth nothing here: click it
/// and type. `speed` only sets how many decimals it offers.
fn num<N: egui::emath::Numeric>(value: &mut N, speed: f64) -> egui::DragValue<'_> {
    let d = egui::DragValue::new(value).speed(0.0);
    if N::INTEGRAL {
        d
    } else {
        d.max_decimals(decimals_for(speed))
    }
}

/// How fine a field used to be draggable, turned into how many decimals it
/// is worth offering now that it is typed.
fn decimals_for(speed: f64) -> usize {
    match speed {
        s if s >= 1.0 => 0,
        s if s >= 0.1 => 2,
        s if s >= 0.01 => 3,
        _ => 4,
    }
}

/// One dropdown over every axis a chart could use.
fn axis_picker(ui: &mut egui::Ui, id: String, axis: &mut Axis, options: &[(String, Axis)]) -> bool {
    let current = options
        .iter()
        .find(|(_, a)| a == axis)
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| axis.label());
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(current).monospace())
        .width(165.0)
        .show_ui(ui, |ui| {
            for (label, candidate) in options {
                if ui.selectable_label(candidate == axis, label).clicked() {
                    *axis = candidate.clone();
                    changed = true;
                }
            }
        });
    changed
}

// --------------------------------------------------------------------- clock

/// Channel names the clock panel looks for. A node that declares these gets a
/// time panel; one that does not simply has no clock to show.
const CH_UNIX: &str = "unix_time";
const CH_RTC_OK: &str = "rtc_ok";
const CH_SET_UNIX: &str = "set_unix";
const CH_SYNC: &str = "sync_time";

/// What one node has to say about time.
struct NodeClock {
    /// The node's own count of unix seconds, if it reports one.
    unix: Option<i64>,
    /// It reports a clock and that clock is running.
    running: bool,
    /// It declares the two control channels a sync needs.
    can_sync: bool,
    /// Why there is nothing to show, when there is nothing to show.
    note: &'static str,
}

impl App {
    /// Every node's clock, side by side, and one button that sets them all.
    ///
    /// Listed per node rather than one at a time: boards differ, and a mixed
    /// bench — one with a battery-backed RTC, one counting milliseconds since
    /// power-up — is the normal case rather than the odd one. Syncing is still
    /// per node underneath; "sync all" is just every one of them at once, each
    /// carrying the same timestamp.
    fn clock_tab(&mut self, ui: &mut egui::Ui) {
        let zone = self.zone();
        ui.horizontal(|ui| {
            ui.label(theme::header("time zone"));
            ui.label(theme::value(zone.describe()));
            ui.label(theme::value_muted(format!("({})", zone.abbrev())));
            ui.separator();
            ui.label(
                RichText::new("Change it in Settings. Every time in the app is shown in it.")
                    .italics()
                    .small()
                    .color(theme::MUTED),
            );
        });
        ui.separator();
        ui.add_space(6.0);

        let station_unix = chrono::Utc::now().timestamp();
        ui.horizontal(|ui| {
            ui.label(theme::header("station"));
            ui.label(
                RichText::new(format!("{} {}", zone.stamp(station_unix), zone.abbrev()))
                    .monospace()
                    .size(16.0)
                    .color(theme::TEXT),
            );
            ui.label(theme::value_muted(zone.offset_summary()));
        });
        ui.add_space(10.0);

        let nodes: Vec<config::NodeConfig> = self.station.profile.lock().unwrap().nodes.clone();
        if nodes.is_empty() {
            ui.label(theme::value_muted(
                "No nodes configured. Add one in Settings.",
            ));
            return;
        }
        let views = self.station.views.lock().unwrap().clone();
        let clocks: Vec<(config::NodeConfig, NodeClock)> = nodes
            .into_iter()
            .map(|n| {
                let clock = views.get(&n.id).map(node_clock).unwrap_or(NodeClock {
                    unix: None,
                    running: false,
                    can_sync: false,
                    note: "not connected",
                });
                (n, clock)
            })
            .collect();

        let mut sync: Vec<u16> = Vec::new();
        egui::Grid::new("clocks")
            .num_columns(5)
            .striped(true)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                for h in ["node", "clock", "node time", "drift", ""] {
                    ui.label(theme::header(h));
                }
                ui.end_row();

                for (n, clock) in &clocks {
                    ui.label(
                        RichText::new(format!("{} #{}", n.label, n.id))
                            .strong()
                            .color(theme::HEADER_TEXT),
                    );

                    match (clock.running, clock.unix) {
                        (true, Some(unix)) => {
                            ui.label(theme::value("running"));
                            ui.label(
                                RichText::new(format!("{} {}", zone.stamp(unix), zone.abbrev()))
                                    .monospace()
                                    .size(15.0)
                                    .color(theme::TEXT),
                            );
                            let drift = unix - station_unix;
                            ui.label(RichText::new(format!("{drift:+} s")).monospace().color(
                                match drift.abs() {
                                    0..=1 => theme::TEXT,
                                    2..=30 => theme::WARN,
                                    _ => theme::BAD,
                                },
                            ));
                        }
                        _ => {
                            ui.label(RichText::new(clock.note).color(theme::MUTED));
                            ui.label(theme::value_muted("—"));
                            ui.label(theme::value_muted("—"));
                        }
                    }

                    if clock.can_sync {
                        if ui
                            .button("sync")
                            .on_hover_text("Set this node's clock from the station")
                            .clicked()
                        {
                            sync.push(n.id);
                        }
                    } else {
                        ui.label(theme::value_muted("no sync channels"))
                            .on_hover_text(format!(
                                "A sync button needs control channels named {CH_SET_UNIX} \
                                 (i32) and {CH_SYNC} (cmd) on the board."
                            ));
                    }
                    ui.end_row();
                }
            });

        let syncable = clocks.iter().filter(|(_, c)| c.can_sync).count();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    syncable > 0,
                    egui::Button::new(RichText::new("  Sync all nodes  ").size(14.0)),
                )
                .on_hover_text("Every node that can be set, from this station's clock")
                .clicked()
            {
                sync.extend(clocks.iter().filter(|(_, c)| c.can_sync).map(|(n, _)| n.id));
            }
            if syncable == 0 {
                ui.label(theme::value_muted("No node here has clock channels."));
            }
        });

        if !sync.is_empty() {
            // One timestamp for the whole batch, so two boards set together
            // agree with each other as well as with the station.
            let now = chrono::Utc::now().timestamp() as i32;
            for id in &sync {
                // Put the timestamp in place first, then bump the counter. The
                // node fires on the counter changing, so the value is always
                // already there when it does.
                self.station.set_intent(*id, CH_SET_UNIX, Intent::I32(now));
                self.station.bump_cmd(*id, CH_SYNC);
            }
            self.status = match sync.len() {
                1 => format!("Sent the station's time to node #{}.", sync[0]),
                n => format!("Sent the station's time to {n} nodes."),
            };
        }

        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "A node keeps a plain count of seconds with no zone attached, which is why \
                 its clock can be shown in yours. Drift is the node's count minus the \
                 station's. Sync sets set_unix and then increments the sync_time counter — \
                 a counter rather than a pulse, so a dropped packet delays the sync instead \
                 of losing it.",
            )
            .italics()
            .small()
            .color(theme::MUTED),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Nothing in the station depends on any of this. Logs, charts and exports are \
                 always stamped with the station's clock, so a board with no RTC — counting \
                 milliseconds since power-up — loses nothing. A node clock matters only to \
                 code on the board that wants the time of day.",
            )
            .italics()
            .small()
            .color(theme::MUTED),
        );
    }
}

/// Read one node's clock out of its schema and last frame.
fn node_clock(view: &crate::station::NodeView) -> NodeClock {
    let none = NodeClock {
        unix: None,
        running: false,
        can_sync: false,
        note: "no schema yet",
    };
    let Some(schema) = &view.schema else {
        return none;
    };
    let find = |name: &str, dir: Dir| {
        schema
            .channels
            .iter()
            .find(|c| c.dir == dir && c.name == name)
    };
    let can_sync =
        find(CH_SET_UNIX, Dir::Control).is_some() && find(CH_SYNC, Dir::Control).is_some();
    let Some(values) = &view.telemetry else {
        return NodeClock {
            can_sync,
            note: "no telemetry yet",
            ..none
        };
    };
    let Some(unix_chan) = find(CH_UNIX, Dir::Telemetry) else {
        return NodeClock {
            can_sync,
            note: "reports no clock",
            ..none
        };
    };
    let unix = values
        .i32s
        .get(unix_chan.slot as usize)
        .copied()
        .unwrap_or(0) as i64;
    // A node with no rtc_ok channel but a running clock is taken at its word;
    // one that says it has no RTC, or reports 0, has no clock.
    let rtc_ok = match find(CH_RTC_OK, Dir::Telemetry) {
        Some(c) => values.bools.get(c.slot as usize).copied().unwrap_or(false),
        None => unix > 0,
    };
    NodeClock {
        unix: Some(unix),
        running: rtc_ok && unix > 0,
        can_sync,
        note: if rtc_ok { "not set" } else { "no RTC" },
    }
}

// --------------------------------------------------------------- joystick

impl App {
    fn joystick_tab(&mut self, ui: &mut egui::Ui) {
        let (devices, error) = {
            let j = self.joy.lock().unwrap();
            (j.devices.clone(), j.error.clone())
        };

        if let Some(e) = error {
            ui.label(RichText::new(e).color(theme::BAD));
        }
        if devices.is_empty() {
            ui.label(theme::value_muted(
                "No controllers detected. This view works with no node connected — \
                 plug one in and every axis and button appears.",
            ));
            return;
        }

        let mut names: Vec<&String> = devices.keys().collect();
        names.sort();

        for name in names {
            let dev = &devices[name];
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).size(15.0).color(theme::TEXT));
                theme::lamp(ui, dev.connected, 10.0);
                if !dev.connected {
                    ui.label(theme::value_muted("disconnected"));
                }
            });

            ui.add_space(4.0);
            ui.label(theme::header("axes"));
            let mut axes: Vec<_> = dev.axes.iter().collect();
            axes.sort_by_key(|(k, _)| (*k).clone());
            // Buttons with travel, such as the triggers, are listed with the
            // axes too: they are mapped like one, from rest at 0 to full at 1.
            // A digital button also reports 0 or 1 here, so only list the
            // triggers, and anything seen part-way pressed.
            let mut travel: Vec<(&String, &f32)> = Vec::new();
            for (code, v) in &dev.analog {
                if code.ends_with("Trigger2")
                    || (*v > 0.001 && *v < 0.999)
                    || self.seen_travel.contains(code)
                {
                    self.seen_travel.insert(code.clone());
                    travel.push((code, v));
                }
            }
            travel.sort_by_key(|(k, _)| (*k).clone());
            egui::Grid::new(format!("ax{name}"))
                .num_columns(3)
                .spacing([6.0, 2.0])
                .show(ui, |ui| {
                    for (code, v) in axes {
                        ui.label(RichText::new(code).monospace().color(theme::TEXT));
                        ui.add(
                            egui::ProgressBar::new((v + 1.0) / 2.0)
                                .desired_width(170.0)
                                .fill(theme::BORDER_BRIGHT)
                                .text(RichText::new(format!("{v:+.3}")).monospace()),
                        );
                        if ui.small_button("map").clicked() {
                            self.new_map_source = code.clone();
                        }
                        ui.end_row();
                    }
                    for (code, v) in travel {
                        ui.label(RichText::new(code).monospace().color(theme::TEXT))
                            .on_hover_text("A button with travel. Rests at 0, fully pressed is 1.");
                        ui.add(
                            egui::ProgressBar::new(*v)
                                .desired_width(170.0)
                                .fill(theme::BORDER_BRIGHT)
                                .text(RichText::new(format!("{v:.3}")).monospace()),
                        );
                        if ui.small_button("map").clicked() {
                            self.new_map_source = code.clone();
                        }
                        ui.end_row();
                    }
                });

            ui.add_space(6.0);
            ui.label(theme::header("buttons"));
            let mut btns: Vec<_> = dev.buttons.iter().collect();
            btns.sort_by_key(|(k, _)| (*k).clone());
            ui.horizontal_wrapped(|ui| {
                for (code, pressed) in btns {
                    ui.scope(|ui| {
                        theme::lamp(ui, *pressed, 12.0);
                        let col = if *pressed { theme::TEXT } else { theme::MUTED };
                        if ui
                            .add(egui::Button::new(
                                RichText::new(code).small().monospace().color(col),
                            ))
                            .clicked()
                        {
                            self.new_map_source = code.clone();
                        }
                    });
                }
            });

            ui.add_space(6.0);
            let mut note = "Every control this device reports is listed, pressed or not — so \
                 a button that never lights is a real answer, not just one you have \
                 not tried. Nothing here is specific to one make of controller."
                .to_string();
            if !dev.raw.is_empty() {
                note.push_str(&format!(
                    "  {} control(s) have no standard name and are listed by their raw \
                     code ({}); those map the same, but the code is specific to this \
                     machine's drivers, so such a mapping may not carry to another.",
                    dev.raw.len(),
                    dev.raw.join(", ")
                ));
            }
            ui.label(RichText::new(note).italics().small().color(theme::MUTED));

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(theme::header("map"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_map_source)
                        .desired_width(130.0)
                        .hint_text("source"),
                );
                ui.label(theme::value_muted(format!(
                    "-> node #{}",
                    self.selected_node
                )));
                let controls: Vec<String> = self
                    .station
                    .views
                    .lock()
                    .unwrap()
                    .get(&self.selected_node)
                    .and_then(|v| v.schema.clone())
                    .map(|s| {
                        s.channels
                            .iter()
                            .filter(|c| {
                                c.dir == Dir::Control
                                    && matches!(
                                        c.ctype,
                                        ChanType::Bool | ChanType::I32 | ChanType::F32
                                    )
                            })
                            .map(|c| c.name.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                if controls.is_empty() {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_map_channel)
                            .desired_width(130.0)
                            .hint_text("channel"),
                    );
                } else {
                    chan_picker(
                        ui,
                        format!("newmap{name}"),
                        &mut self.new_map_channel,
                        &controls,
                    );
                }
                if ui.button("Add mapping").clicked()
                    && !self.new_map_source.is_empty()
                    && !self.new_map_channel.is_empty()
                {
                    let mut p = self.station.profile.lock().unwrap();
                    p.joystick
                        .devices
                        .entry(name.clone())
                        .or_default()
                        .mappings
                        .push(config::Mapping {
                            source: self.new_map_source.clone(),
                            node_id: self.selected_node,
                            channel: self.new_map_channel.clone(),
                            trim: 0.0,
                            // Sticks drift; start with a deadband rather than
                            // discovering the hard way that an idle stick creeps.
                            deadband: 0.05,
                            invert: false,
                        });
                    drop(p);
                    self.status = format!(
                        "Mapped {} -> node #{} {}",
                        self.new_map_source, self.selected_node, self.new_map_channel
                    );
                    self.new_map_source.clear();
                    self.new_map_channel.clear();
                }
            });

            self.mapping_editor(ui, name);
            ui.separator();
        }
    }

    fn mapping_editor(&mut self, ui: &mut egui::Ui, device: &str) {
        let dev_state = self.joy.lock().unwrap().devices.get(device).cloned();
        let mut p = self.station.profile.lock().unwrap();
        let Some(dev) = p.joystick.devices.get_mut(device) else {
            return;
        };
        if dev.mappings.is_empty() {
            return;
        }
        ui.label(theme::header("mappings"));
        let mut remove = None;
        egui::Grid::new(format!("map{device}"))
            .num_columns(8)
            .striped(true)
            .spacing([8.0, 3.0])
            .show(ui, |ui| {
                for h in [
                    "source", "node", "channel", "invert", "trim", "deadband", "out", "",
                ] {
                    ui.label(theme::header(h));
                }
                ui.end_row();
                for (i, m) in dev.mappings.iter_mut().enumerate() {
                    ui.label(RichText::new(&m.source).monospace().color(theme::TEXT));
                    ui.add(num(&mut m.node_id, 1.0));
                    ui.label(RichText::new(&m.channel).monospace().color(theme::TEXT));
                    ui.checkbox(&mut m.invert, "");
                    ui.add(num(&mut m.trim, 0.01));
                    ui.add(num(&mut m.deadband, 0.005).range(0.0..=0.9).max_decimals(3));
                    // What the mapping hands on, before the channel's own
                    // expo and range.
                    match dev_state
                        .as_ref()
                        .and_then(|d| crate::joystick::read_source(d, &m.source))
                    {
                        Some((raw, unipolar)) => {
                            ui.label(theme::value(format!("{:+.3}", m.deflection(raw, unipolar))))
                        }
                        None => ui.label(theme::value_muted("—")),
                    };
                    if ui.small_button("remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            dev.mappings.remove(i);
        }
        drop(p);
        ui.label(
            RichText::new(
                "A mapping only routes and cleans up the input. Expo, range, offset and \
                 limits are the channel's, set in its control row, where the value the \
                 stick produces is shown — and where hold overrides it with a fixed value.",
            )
            .italics()
            .small()
            .color(theme::MUTED),
        );
    }
}

// ------------------------------------------------------------------ logging

impl App {
    fn logging_tab(&mut self, ui: &mut egui::Ui) {
        let zone = self.zone();
        let background = self.station.log_path.lock().unwrap().clone();
        let session = *self.station.log_session.lock().unwrap();
        let recording = session.is_some();

        ui.add_space(4.0);
        egui::Frame::new()
            .fill(if recording {
                Color32::from_rgb(60, 20, 20)
            } else {
                theme::PANEL
            })
            .stroke(egui::Stroke::new(
                1.0,
                if recording { theme::BAD } else { theme::BORDER },
            ))
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if recording {
                        let btn = egui::Button::new(
                            RichText::new("  —  STOP DATA LOG  ")
                                .size(17.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(150, 35, 35));
                        if ui.add_enabled(!self.exporting, btn).clicked() {
                            self.stop_data_log();
                        }
                    } else {
                        let btn = egui::Button::new(
                            RichText::new("  ●  START DATA LOG  ")
                                .size(17.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(35, 110, 55));
                        if ui.add_enabled(!self.exporting, btn).clicked() {
                            *self.station.log_session.lock().unwrap() =
                                Some(crate::station::LogSession {
                                    start_t_us: self.station.t_us(),
                                });
                            self.status =
                                "Recording. Hit stop and you will be asked where to save the CSV."
                                    .into();
                        }
                    }

                    ui.add_space(14.0);
                    match session {
                        Some(s) => {
                            let secs =
                                (self.station.t_us().saturating_sub(s.start_t_us)) as f64 / 1e6;
                            let started_unix =
                                (self.station.started_unix_us + s.start_t_us as i64) / 1_000_000;
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{:02}:{:02}:{:04.1}",
                                        (secs / 3600.0) as u32,
                                        ((secs % 3600.0) / 60.0) as u32,
                                        secs % 60.0
                                    ))
                                    .monospace()
                                    .size(20.0)
                                    .color(theme::TEXT),
                                );
                                ui.label(theme::value_muted(format!(
                                    "started {} {}",
                                    zone.stamp(started_unix),
                                    zone.abbrev()
                                )));
                            });
                        }
                        None => {
                            ui.label(
                                RichText::new("Not recording.")
                                    .size(15.0)
                                    .color(theme::MUTED),
                            );
                        }
                    }
                    if self.exporting {
                        ui.add_space(10.0);
                        ui.spinner();
                        ui.label(theme::value_muted("writing CSV…"));
                    }
                });
            });

        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "Start and stop mark the stretch you care about. On stop you pick where \
                 the CSV goes, and only that stretch is written to it.",
            )
            .italics()
            .color(theme::MUTED),
        );

        ui.add_space(18.0);
        ui.separator();
        ui.label(theme::header("background recording"));
        ui.add_space(4.0);
        match &background {
            Some(p) => {
                ui.label(
                    RichText::new(
                        "Everything is already being recorded, the whole time the app is \
                         open, whether or not you have pressed start. Start and stop only \
                         choose which part of it gets exported — nothing is ever lost by \
                         forgetting to press start.",
                    )
                    .color(theme::TEXT),
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::value_muted("file"));
                    ui.label(theme::value(p));
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.exporting, egui::Button::new("Export whole session…"))
                        .clicked()
                    {
                        self.export_range(None, None, "whole session");
                    }
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Telemetry and control are both recorded. The CSV carries elapsed \
                         microseconds and the time of day in your zone, one row per \
                         telemetry frame.",
                    )
                    .italics()
                    .small()
                    .color(theme::MUTED),
                );
            }
            None => {
                ui.label(
                    RichText::new(
                        "Background recording is off, so there is nothing to export. Turn \
                         it on in Settings and restart the app.",
                    )
                    .color(theme::WARN),
                );
            }
        }
    }

    /// Close the marked window and write it out.
    fn stop_data_log(&mut self) {
        let Some(session) = self.station.log_session.lock().unwrap().take() else {
            return;
        };
        let end = self.station.t_us();
        self.export_range(Some(session.start_t_us), Some(end), "recording");
    }

    /// Ask where to put a CSV, then write it on a worker thread.
    ///
    /// Both halves are off the UI thread: the dialog because it blocks, and
    /// the writing because a long session is a big file.
    fn export_range(&mut self, from_us: Option<u64>, to_us: Option<u64>, what: &str) {
        if self.station.log_path.lock().unwrap().is_none() {
            self.status = "Nothing to export: background recording is off.".into();
            return;
        }
        if self.dialog_open {
            return;
        }
        let start_dir = self
            .station
            .profile
            .lock()
            .unwrap()
            .logging
            .last_export_dir
            .clone();
        let suggested = format!("microrio-{}.csv", self.zone().file_stamp());
        let title = format!("Save the {what} as CSV");
        let what = what.to_string();
        let (w2, cancel) = (
            what.clone(),
            format!("Export cancelled. The {what} is still in the log file."),
        );
        self.ask_file(
            move || {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name(suggested)
                    .set_title(title);
                if let Some(dir) = start_dir {
                    dialog = dialog.set_directory(dir);
                }
                dialog.save_file()
            },
            move |path| FileJob::Export {
                path,
                from_us,
                to_us,
                what: w2,
            },
            &cancel,
        );
    }

    /// Write the CSV the dialog asked for.
    fn write_export(
        &mut self,
        path: std::path::PathBuf,
        from_us: Option<u64>,
        to_us: Option<u64>,
        what: String,
    ) {
        let Some(log) = self.station.log_path.lock().unwrap().clone() else {
            return;
        };
        let zone = self.zone();
        if let Some(dir) = path.parent() {
            self.station.profile.lock().unwrap().logging.last_export_dir = Some(dir.to_path_buf());
        }

        self.exporting = true;
        self.status = format!("Writing {}…", path.display());

        let out = self.export.clone();
        let station = self.station.clone();
        let what = what.to_string();
        std::thread::spawn(move || {
            // The logging thread buffers; without this the last couple of
            // hundred rows would be missing from the file we are about to read.
            station.flush_log(std::time::Duration::from_secs(2));
            let opts = crate::logger::ExportOpts {
                zone: Some(zone),
                from_us,
                to_us,
            };
            let msg = match crate::logger::export_window(std::path::Path::new(&log), &path, &opts) {
                Ok(files) if files.len() == 1 => {
                    format!("Saved the {what} to {}", files[0].display())
                }
                Ok(files) => format!(
                    "Saved {} files, one per node: {}",
                    files.len(),
                    files
                        .iter()
                        .map(|f| f.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Err(e) => format!("Export failed: {e}"),
            };
            *out.lock().unwrap() = Some(msg);
        });
    }
}

// ----------------------------------------------------------------- settings

/// One titled, padded block. The old settings page ran everything together at
/// one size, which made it a wall rather than a page.
fn section<R>(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui) -> R) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(13.0)
            .strong()
            .color(theme::HEADER_TEXT),
    );
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
            ui.set_width(ui.available_width());
            body(ui);
        });
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).italics().small().color(theme::MUTED));
}

impl App {
    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        section(ui, "profile", |ui| {
            match self.station.profile_path.lock().unwrap().clone() {
                Some(p) => ui.label(theme::value(p.display().to_string())),
                None => ui.label(theme::value_muted(
                    "No file yet — use Save as… in the bar at the top.",
                )),
            };
            hint(
                ui,
                "Everything on this page is saved into that file, along with your \
                 charts, scaling and joystick mappings.",
            );
        });

        section(ui, "time zone", |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("tzpick")
                    .selected_text(RichText::new(&self.tz_input).monospace())
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for z in COMMON_ZONES {
                            if ui.selectable_label(self.tz_input == *z, *z).clicked() {
                                self.tz_input = (*z).to_string();
                            }
                        }
                    });
                ui.add_sized(
                    egui::vec2(230.0, 20.0),
                    egui::TextEdit::singleline(&mut self.tz_input)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("system, or an IANA name"),
                );
                let ok = Zone::is_known(&self.tz_input);
                if ok {
                    let zone = Zone::parse(&self.tz_input);
                    ui.label(
                        RichText::new(format!(
                            "{} {}   {}",
                            zone.now_stamp(),
                            zone.abbrev(),
                            zone.offset_summary()
                        ))
                        .monospace()
                        .color(theme::TEXT),
                    )
                    .on_hover_text(zone.describe());
                    let mut p = self.station.profile.lock().unwrap();
                    if p.timezone != self.tz_input {
                        p.timezone = self.tz_input.clone();
                    }
                } else {
                    ui.label(
                        RichText::new("not a zone name — still using the last good one")
                            .color(theme::WARN),
                    );
                }
            });
            hint(
                ui,
                "`system` follows whatever the machine is set to, which is what you want \
                 on a laptop that travels. A named zone pins it, which is what you want \
                 when the logs have to line up with someone else's. Clocks, charts and \
                 exported CSVs all use it; the wire never carries a zone at all.",
            );
        });

        self.settings_nodes(ui);
        self.settings_channels(ui);
        self.settings_crosslinks(ui);

        section(ui, "background recording", |ui| {
            let mut p = self.station.profile.lock().unwrap();
            ui.checkbox(&mut p.logging.enabled, "Record every session to disk");
            let dir = p.logging.resolved_dir();
            drop(p);
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::value_muted("folder"));
                ui.label(theme::value(dir.display().to_string()));
            });
            hint(
                ui,
                "Takes effect when the app next starts. Turning it off means Start Data \
                 Log has nothing to export from, so leave it on unless disk space is the \
                 problem.",
            );
        });
        ui.add_space(20.0);
    }

    fn settings_nodes(&mut self, ui: &mut egui::Ui) {
        section(ui, "nodes", |ui| {
            hint(
                ui,
                "Addresses are typed in, so each board needs a static IP or a DHCP \
                 reservation. Rate, watchdog and address take effect as soon as you \
                 change them; adding or removing a node needs Apply. Rate is how often \
                 the node sends telemetry, and so how many log rows a second.",
            );
            ui.add_space(6.0);

            let mut p = self.station.profile.lock().unwrap();
            let mut remove = None;
            egui::Grid::new("nodes")
                .num_columns(8)
                .striped(true)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    for h in ["id", "label", "ip", "port", "rate", "watchdog", "link", ""] {
                        ui.label(theme::header(h));
                    }
                    ui.end_row();
                    for (i, n) in p.nodes.iter_mut().enumerate() {
                        ui.add_sized(egui::vec2(48.0, 20.0), num(&mut n.id, 1.0));
                        ui.add_sized(
                            egui::vec2(140.0, 20.0),
                            egui::TextEdit::singleline(&mut n.label),
                        );
                        let mut ip = n.ip.to_string();
                        if ui
                            .add_sized(
                                egui::vec2(150.0, 20.0),
                                egui::TextEdit::singleline(&mut ip)
                                    .font(egui::TextStyle::Monospace),
                            )
                            .changed()
                        {
                            if let Ok(parsed) = ip.parse() {
                                n.ip = parsed;
                            }
                        }
                        ui.add_sized(egui::vec2(70.0, 20.0), num(&mut n.port, 1.0));
                        ui.add_sized(
                            egui::vec2(80.0, 20.0),
                            num(&mut n.rate_hz, 1.0).range(1..=1000).suffix(" Hz"),
                        );
                        ui.add_sized(
                            egui::vec2(80.0, 20.0),
                            num(&mut n.watchdog_ms, 1.0)
                                .range(20..=10_000)
                                .suffix(" ms"),
                        )
                        .on_hover_text(
                            "Node-to-station watchdog: if nothing arrives from this node \
                             for this long, its header shows NO COMMS. An alarm only. \
                             The other direction is the node's own watchdog, set in its \
                             sketch with rio.watchdog(ms), where failstates take over.",
                        );
                        ui.checkbox(&mut n.link_active, "").on_hover_text(
                            "Whether the station opens a link to this node at all. \
                                 This is not the enable switch — that is per node, on \
                                 the node row at the top of the window, and is never \
                                 saved to a file.",
                        );
                        if ui.small_button("remove").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            if let Some(i) = remove {
                p.nodes.remove(i);
            }

            ui.add_space(10.0);
            let mut apply = false;
            ui.horizontal(|ui| {
                if ui.button("Add node").clicked() {
                    let next = p.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
                    p.nodes.push(config::NodeConfig {
                        id: next,
                        label: format!("node-{next}"),
                        ..Default::default()
                    });
                }
                if ui.button("Apply node changes").clicked() {
                    apply = true;
                }
            });
            drop(p);
            if apply {
                self.station.restart_links();
                self.status =
                    "Links rebuilt with the current node list. Everything left disabled.".into();
            }
        });
    }

    fn settings_crosslinks(&mut self, ui: &mut egui::Ui) {
        let nodes = self.station.profile.lock().unwrap().nodes.clone();
        let views = self.station.views.lock().unwrap().clone();

        // Channel names to offer, per node and direction.
        let chans = |id: u16, dir: Dir| -> Vec<String> {
            views
                .get(&id)
                .and_then(|v| v.schema.clone())
                .map(|s| {
                    s.channels
                        .iter()
                        .filter(|c| c.dir == dir && plottable(c.ctype))
                        .map(|c| c.name.clone())
                        .collect()
                })
                .unwrap_or_default()
        };

        section(ui, "cross-node links", |ui| {
            hint(
                ui,
                "Take a reading from one board and use it to drive an output on another. \
                 The value is scaled and clamped on the way across. A linked channel \
                 stops being yours to set by hand — the control grid marks it.",
            );
            ui.add_space(6.0);

            let mut p = self.station.profile.lock().unwrap();
            if p.crosslinks.is_empty() {
                ui.label(theme::value_muted("None."));
            } else {
                let mut remove = None;
                egui::Grid::new("xlinks")
                    .num_columns(9)
                    .striped(true)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        for h in [
                            "on",
                            "from node",
                            "telemetry",
                            "to node",
                            "control",
                            "slope",
                            "offset",
                            "min / max",
                            "",
                        ] {
                            ui.label(theme::header(h));
                        }
                        ui.end_row();

                        for (i, l) in p.crosslinks.iter_mut().enumerate() {
                            ui.checkbox(&mut l.active, "");
                            node_picker(ui, format!("xf{i}"), &mut l.from_node, &nodes);
                            chan_picker(
                                ui,
                                format!("xfc{i}"),
                                &mut l.from_channel,
                                &chans(l.from_node, Dir::Telemetry),
                            );
                            node_picker(ui, format!("xt{i}"), &mut l.to_node, &nodes);
                            chan_picker(
                                ui,
                                format!("xtc{i}"),
                                &mut l.to_channel,
                                &chans(l.to_node, Dir::Control),
                            );
                            ui.add(num(&mut l.slope, 0.01));
                            ui.add(num(&mut l.offset, 0.01));
                            ui.horizontal(|ui| {
                                ui.add(num(&mut l.min, 0.1));
                                ui.add(num(&mut l.max, 0.1));
                            });
                            if ui.small_button("remove").clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(i) = remove {
                    p.crosslinks.remove(i);
                }
            }

            ui.add_space(10.0);
            if ui.button("Add link").clicked() {
                let first = nodes.first().map(|n| n.id).unwrap_or(1);
                let second = nodes.get(1).map(|n| n.id).unwrap_or(first);
                p.crosslinks.push(CrossLink {
                    from_node: first,
                    to_node: second,
                    ..Default::default()
                });
            }
            drop(p);
            hint(
                ui,
                "If the source board stops reporting for a fifth of a second, the board \
                 being driven is put into FAULT rather than held at the last value it \
                 saw. A dead sensor must not go on commanding an output.",
            );
        });
    }
}

fn node_picker(ui: &mut egui::Ui, id: String, value: &mut u16, nodes: &[config::NodeConfig]) {
    let current = nodes
        .iter()
        .find(|n| n.id == *value)
        .map(|n| format!("{} #{}", n.label, n.id))
        .unwrap_or_else(|| format!("#{value}"));
    egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(current).monospace())
        .width(130.0)
        .show_ui(ui, |ui| {
            for n in nodes {
                if ui
                    .selectable_label(n.id == *value, format!("{} #{}", n.label, n.id))
                    .clicked()
                {
                    *value = n.id;
                }
            }
        });
}

/// A dropdown of what the board actually declares, and still typable: a link
/// can be set up before the board it names is on the network.
fn chan_picker(ui: &mut egui::Ui, id: String, value: &mut String, options: &[String]) {
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(120.0, 20.0),
            egui::TextEdit::singleline(value)
                .font(egui::TextStyle::Monospace)
                .hint_text("channel"),
        );
        egui::ComboBox::from_id_salt(id)
            .selected_text("v")
            .width(26.0)
            .show_ui(ui, |ui| {
                if options.is_empty() {
                    ui.label(theme::value_muted("node not connected"));
                }
                for o in options {
                    if ui.selectable_label(o == value, o).clicked() {
                        *value = o.clone();
                    }
                }
            });
    });
}
