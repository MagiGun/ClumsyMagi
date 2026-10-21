//! ClumsyMagi egui application: filter bar with presets and live validation,
//! module rows, elevation banner, live event log + throughput graph,
//! dark/light/system themes, persisted config.

use crate::config::{AppConfig, Theme};
use crate::engine::modules::ModuleKind;
use crate::engine::params::ModuleParams;
use crate::engine::{EngineHandle, StatsSnapshot};
use crate::wd::WinDivertLib;
use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const ACTIVITY_WINDOW_MS: u128 = 600;

pub struct ClumsyMagiApp {
    pub config: AppConfig,
    pub config_dirty: bool,
    last_config_save: Instant,

    /// Live module params, shared with the engine while it runs.
    pub params: Vec<Arc<ModuleParams>>,

    pub lib: Option<Arc<WinDivertLib>>,
    lib_load_error: Option<String>,

    pub engine: Option<EngineHandle>,
    pub status: String,
    pub status_error: bool,

    preset_selected: usize,
    filter_validation: Result<(), String>,
    last_validated_filter: String,

    pub show_log: bool,
    log_cache: Vec<crate::engine::LogEvent>,
    last_log_refresh: Instant,

    history_samples: Vec<(Instant, f64, f64)>, // (timestamp, captured_per_sec, sent_per_sec)
    last_stats: StatsSnapshot,
    last_sample: Instant,

    pub elevated: bool,
    pub quit_deadline: Option<Instant>,
    preset_name_edit: String,
    preset_window_open: bool,
    applied_theme: Option<Theme>,
    last_system_dark: Option<bool>,
}

impl ClumsyMagiApp {
    pub fn new(config: AppConfig, lib: Result<Arc<WinDivertLib>, String>, elevated: bool) -> Self {
        let preset_selected = config
            .presets
            .iter()
            .position(|p| p.filter == config.filter)
            .unwrap_or(usize::MAX);
        let (lib_ok, lib_err) = match lib {
            Ok(l) => (Some(l), None),
            Err(e) => (None, Some(e)),
        };
        let filter_validation = if lib_ok.is_some() { Ok(()) } else { Err(lib_err.clone().unwrap()) };
        ClumsyMagiApp {
            params: config.build_params(),
            config,
            config_dirty: false,
            last_config_save: Instant::now(),
            lib: lib_ok,
            lib_load_error: lib_err,
            engine: None,
            status: String::new(),
            status_error: false,
            preset_selected,
            filter_validation,
            last_validated_filter: String::new(),
            show_log: true,
            log_cache: Vec::new(),
            last_log_refresh: Instant::now() - Duration::from_secs(1),
            history_samples: Vec::new(),
            last_stats: StatsSnapshot::default(),
            last_sample: Instant::now(),
            elevated,
            quit_deadline: None,
            preset_name_edit: String::new(),
            preset_window_open: false,
            applied_theme: None,
            last_system_dark: None,
        }
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = text.into();
        self.status_error = error;
    }

    pub fn start_filtering(&mut self) {
        let Some(lib) = self.lib.clone() else {
            self.set_status(
                self.lib_load_error
                    .clone()
                    .unwrap_or_else(|| "WinDivert.dll is not available".into()),
                true,
            );
            return;
        };
        let filter = self.config.filter.clone();
        let params = self.params.clone();
        match EngineHandle::start(lib, &filter, params) {
            Ok(engine) => {
                self.set_status(
                    "Started filtering. Enable functionalities to take effect.",
                    false,
                );
                self.engine = Some(engine);
            }
            Err(e) => {
                self.set_status(e, true);
            }
        }
    }

    fn stop_filtering(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.stop();
            self.set_status("Stopped. To begin again, edit criteria and click Start.", false);
        }
    }

    fn validate_filter(&mut self) {
        if self.last_validated_filter == self.config.filter {
            return;
        }
        self.last_validated_filter = self.config.filter.clone();
        if let Some(lib) = &self.lib {
            self.filter_validation = lib.compile_filter(&self.config.filter);
        }
    }

    fn mark_dirty(&mut self) {
        self.config_dirty = true;
    }

    fn maybe_save_config(&mut self, force: bool) {
        if !self.config_dirty && !force {
            return;
        }
        if !force && self.last_config_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.config.sync_from_params(&self.params);
        if crate::config::save(&self.config).is_ok() {
            self.config_dirty = false;
        }
        self.last_config_save = Instant::now();
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let is_dark = match self.config.theme {
            Theme::Dark => true,
            Theme::Light => false,
            Theme::System => crate::platform::is_system_dark_mode(),
        };

        if self.applied_theme != Some(self.config.theme) || self.last_system_dark != Some(is_dark) {
            let visuals = if is_dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            ctx.set_visuals(visuals);
            let theme_pref = match self.config.theme {
                Theme::System => egui::ThemePreference::System,
                Theme::Dark => egui::ThemePreference::Dark,
                Theme::Light => egui::ThemePreference::Light,
            };
            ctx.set_theme(theme_pref);
            self.applied_theme = Some(self.config.theme);
            self.last_system_dark = Some(is_dark);
        }
    }
}

impl eframe::App for ClumsyMagiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // theme (updates visuals and preferences when theme or system dark mode changes)
        self.apply_theme(&ctx);

        // quit-after deadline (--quit-after)
        if let Some(deadline) = self.quit_deadline {
            if Instant::now() >= deadline {
                self.stop_filtering();
                self.maybe_save_config(true);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        // periodic refreshes
        self.validate_filter();
        self.sample_history();
        if self.last_log_refresh.elapsed() >= Duration::from_millis(200) {
            if let Some(engine) = &self.engine {
                self.log_cache = engine.shared.log.snapshot();
            }
            self.last_log_refresh = Instant::now();
        }
        self.maybe_save_config(false);

        // Schedule repaints so the UI stays responsive without requiring mouse movement
        let repaint_delay = if self.engine.as_ref().map(|e| e.is_running()).unwrap_or(false) {
            Duration::from_millis(50)
        } else if let Some(deadline) = self.quit_deadline {
            deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(100))
        } else {
            Duration::from_millis(250)
        };
        ctx.request_repaint_after(repaint_delay);

        self.ui_main_content(ui);
        self.ui_preset_window(&ctx);
    }
}

impl Drop for ClumsyMagiApp {
    fn drop(&mut self) {
        self.stop_filtering();
        self.maybe_save_config(true);
    }
}

impl ClumsyMagiApp {
    fn ui_main_content(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);

        // 1. Filtering Section
        self.ui_filtering_section(ui);

        // 2. Functions Section
        self.ui_functions_section(ui);

        // 3. Activity Section
        self.ui_activity_section(ui);
    }

    // ---------------------------------------------------------------- Filtering
    fn ui_filtering_section(&mut self, ui: &mut egui::Ui) {
        let running = self.engine.as_ref().map(|e| e.is_running()).unwrap_or(false);

        ui.group(|ui| {
            ui.set_width(ui.available_width());

            // Header line: Title, Dot, Theme, Elevate
            ui.horizontal(|ui| {
                let dot_color = match &self.engine {
                    Some(e) => {
                        let now = crate::engine::unix_ms();
                        let ok = e.shared.send_ok_ms.load(Ordering::Relaxed);
                        let fail = e.shared.send_fail_ms.load(Ordering::Relaxed);
                        let recent_fail = fail != 0 && now.saturating_sub(fail) < ACTIVITY_WINDOW_MS as u64;
                        let recent_ok = ok != 0 && now.saturating_sub(ok) < ACTIVITY_WINDOW_MS as u64;
                        if recent_fail {
                            error_color(ui)
                        } else if recent_ok {
                            ok_color(ui)
                        } else {
                            idle_color(ui)
                        }
                    }
                    None => idle_color(ui),
                };
                draw_dot(ui, dot_color);
                ui.label(egui::RichText::new("Filtering").strong());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 1. Theme selector (pinned to top-right)
                    let current_theme_label = match self.config.theme {
                        Theme::System => {
                            if crate::platform::is_system_dark_mode() {
                                "💻 System (Dark)"
                            } else {
                                "💻 System (Light)"
                            }
                        }
                        Theme::Dark => "🌙 Dark",
                        Theme::Light => "☀ Light",
                    };
                    let mut chosen_theme = self.config.theme;
                    egui::ComboBox::from_id_salt("theme_selector")
                        .width(115.0)
                        .selected_text(current_theme_label)
                        .show_ui(ui, |ui| {
                            let sys_label = if crate::platform::is_system_dark_mode() {
                                "💻 System (Dark)"
                            } else {
                                "💻 System (Light)"
                            };
                            ui.selectable_value(&mut chosen_theme, Theme::System, sys_label);
                            ui.selectable_value(&mut chosen_theme, Theme::Dark, "🌙 Dark");
                            ui.selectable_value(&mut chosen_theme, Theme::Light, "☀ Light");
                        });
                    if chosen_theme != self.config.theme {
                        self.config.theme = chosen_theme;
                        self.mark_dirty();
                        ui.ctx().request_repaint();
                    }

                    // 2. Console toggle button
                    let console_vis = crate::platform::is_console_visible();
                    let (console_icon, console_label) = if console_vis {
                        ("📟", "Hide Console")
                    } else {
                        ("📟", "Show Console")
                    };
                    if ui.button(format!("{console_icon} {console_label}"))
                        .on_hover_text(if console_vis {
                            "Hide the Windows command prompt / console window"
                        } else {
                            "Show the Windows command prompt / console window"
                        })
                        .clicked()
                    {
                        crate::platform::toggle_console();
                    }

                    // 3. Elevate button (visible only when not already running as Administrator)
                    if !self.elevated {
                        if ui.button(egui::RichText::new("🛡 Elevate").color(warn_color(ui))).on_hover_text("Restart as Administrator").clicked() {
                            let args = vec!["--elevated-relaunch".to_string()];
                            let _ = crate::platform::relaunch_elevated(&args);
                            self.maybe_save_config(true);
                            std::process::exit(0);
                        }
                    }
                });
            });

            // Filter Text Edit: full width
            let filter_edit = egui::TextEdit::singleline(&mut self.config.filter)
                .hint_text("WinDivert filter expression, e.g. outbound and loopback")
                .desired_width(f32::INFINITY);
            let response = ui.add_enabled(!running, filter_edit);
            if response.changed() {
                self.mark_dirty();
                self.preset_selected = usize::MAX;
            }

            // Controls Row: Start/Stop, Presets, Save
            ui.horizontal(|ui| {
                let btn_text = if running { "Stop" } else { "Start" };
                let btn_color = if running { error_color(ui) } else { ok_color(ui) };
                let btn = egui::Button::new(egui::RichText::new(btn_text).strong().size(13.0).color(btn_color))
                    .min_size(egui::vec2(60.0, 22.0));
                if ui.add(btn).clicked() {
                    if running {
                        self.stop_filtering();
                    } else {
                        self.start_filtering();
                    }
                }

                ui.separator();

                ui.add_enabled_ui(!running, |ui| {
                    ui.label("Presets:");
                    let preset_names: Vec<String> = self.config.presets.iter().map(|p| p.name.clone()).collect();
                    let selected_text = if self.preset_selected < preset_names.len() {
                        preset_names[self.preset_selected].clone()
                    } else {
                        "Custom...".to_string()
                    };
                    egui::ComboBox::from_id_salt("presets")
                        .width(220.0)
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for (i, name) in preset_names.iter().enumerate() {
                                ui.selectable_value(&mut self.preset_selected, i, name);
                            }
                        });

                    if self.preset_selected != usize::MAX {
                        if let Some(preset) = self.config.presets.get(self.preset_selected) {
                            let f = preset.filter.clone();
                            if self.config.filter != f {
                                self.config.filter = f;
                                self.mark_dirty();
                            }
                        }
                    }

                    if ui.button("Save...").on_hover_text("Save current filter as a preset").clicked() {
                        self.preset_window_open = true;
                        self.preset_name_edit.clear();
                    }
                });
            });

            // Status message
            if let Err(e) = &self.filter_validation {
                ui.colored_label(error_color(ui), format!("⚠ {e}"));
            } else if !self.status.is_empty() {
                if self.status_error {
                    ui.colored_label(error_color(ui), format!("⚠ {}", self.status));
                } else {
                    ui.label(&self.status);
                }
            } else if !self.elevated {
                ui.colored_label(warn_color(ui), "⚠ Not running as Administrator: WinDivert requires admin privileges to capture packets.");
            } else {
                ui.label(egui::RichText::new("Notice: loopback packets must use 'outbound' criteria (e.g. 'udp and outbound and loopback').").weak().small());
            }
        });
    }

    // ---------------------------------------------------------------- Functions
    fn ui_functions_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Functions").strong());
                ui.weak("Enable functionalities to take effect in real-time.");
            });
            ui.add_space(2.0);

            for kind in crate::engine::all_modules() {
                let params = self.params[kind.index()].clone();
                self.module_row(ui, *kind, &params);
            }
        });
    }

    fn module_row(&mut self, ui: &mut egui::Ui, kind: ModuleKind, params: &Arc<ModuleParams>) {
        ui.horizontal(|ui| {
            // Activity indicator dot (fixed size)
            let active = match &self.engine {
                Some(e) => match e.shared.since_triggered(kind) {
                    Some(ms) => ms < ACTIVITY_WINDOW_MS as u64,
                    None => false,
                },
                None => false,
            };
            draw_dot(ui, if active { ok_color(ui) } else { idle_color(ui) });

            // Checkbox with fixed width for column alignment
            let mut enabled = params.enabled.load(Ordering::Relaxed);
            ui.allocate_ui(egui::vec2(105.0, 18.0), |ui| {
                if ui.checkbox(&mut enabled, kind.display_name()).changed() {
                    params.enabled.store(enabled, Ordering::Relaxed);
                    self.mark_dirty();
                }
            });

            // Controls dimmed when not enabled
            ui.add_enabled_ui(enabled, |ui| {
                self.module_controls(ui, kind, params);
            });
        });
    }

    fn module_controls(&mut self, ui: &mut egui::Ui, kind: ModuleKind, params: &Arc<ModuleParams>) {
        // Direction toggles (compact: In / Out)
        let mut inbound = params.direction.inbound.load(Ordering::Relaxed);
        let mut outbound = params.direction.outbound.load(Ordering::Relaxed);
        if ui.checkbox(&mut inbound, "In").on_hover_text("Process inbound packets").changed() {
            params.direction.inbound.store(inbound, Ordering::Relaxed);
            self.mark_dirty();
        }
        if ui.checkbox(&mut outbound, "Out").on_hover_text("Process outbound packets").changed() {
            params.direction.outbound.store(outbound, Ordering::Relaxed);
            self.mark_dirty();
        }

        ui.separator();

        match kind {
            ModuleKind::Lag => {
                ui.label("Delay:");
                let mut delay = params.delay_ms.load(Ordering::Relaxed);
                if ui
                    .add(egui::DragValue::new(&mut delay).speed(5.0).range(0..=15000).suffix("ms"))
                    .changed()
                {
                    params.delay_ms.store(delay, Ordering::Relaxed);
                    self.mark_dirty();
                }
            }
            ModuleKind::Drop => {
                ui.label("Chance:");
                let mut pct = params.chance_percent();
                if ui
                    .add(egui::DragValue::new(&mut pct).speed(0.1).range(0.0..=100.0).suffix("%").max_decimals(1))
                    .changed()
                {
                    params.set_chance_percent(pct);
                    self.mark_dirty();
                }
            }
            ModuleKind::Throttle => {
                ui.label("Time:");
                let mut frame = params.throttle_frame_ms.load(Ordering::Relaxed);
                if ui
                    .add(egui::DragValue::new(&mut frame).speed(2.0).range(0..=1000).suffix("ms"))
                    .changed()
                {
                    params.throttle_frame_ms.store(frame, Ordering::Relaxed);
                    self.mark_dirty();
                }
                let mut drop_t = params.drop_throttled.load(Ordering::Relaxed);
                if ui.checkbox(&mut drop_t, "Drop").on_hover_text("Drop throttled packets instead of sending").changed() {
                    params.drop_throttled.store(drop_t, Ordering::Relaxed);
                    self.mark_dirty();
                }
                ui.label("Chance:");
                let mut pct = params.chance_percent();
                if ui
                    .add(egui::DragValue::new(&mut pct).speed(0.1).range(0.0..=100.0).suffix("%").max_decimals(1))
                    .changed()
                {
                    params.set_chance_percent(pct);
                    self.mark_dirty();
                }
            }
            ModuleKind::Duplicate => {
                ui.label("Count:");
                let mut count = params.dup_count.load(Ordering::Relaxed);
                if ui.add(egui::DragValue::new(&mut count).speed(0.2).range(2..=50)).changed() {
                    params.dup_count.store(count, Ordering::Relaxed);
                    self.mark_dirty();
                }
                ui.label("Chance:");
                let mut pct = params.chance_percent();
                if ui
                    .add(egui::DragValue::new(&mut pct).speed(0.1).range(0.0..=100.0).suffix("%").max_decimals(1))
                    .changed()
                {
                    params.set_chance_percent(pct);
                    self.mark_dirty();
                }
            }
            ModuleKind::Ood => {
                ui.label("Chance:");
                let mut pct = params.chance_percent();
                if ui
                    .add(egui::DragValue::new(&mut pct).speed(0.1).range(0.0..=100.0).suffix("%").max_decimals(1))
                    .changed()
                {
                    params.set_chance_percent(pct);
                    self.mark_dirty();
                }
            }
            ModuleKind::Tamper => {
                let mut redo = params.redo_checksum.load(Ordering::Relaxed);
                if ui.checkbox(&mut redo, "Redo Checksum").on_hover_text("Recompute checksums after tampering").changed() {
                    params.redo_checksum.store(redo, Ordering::Relaxed);
                    self.mark_dirty();
                }
                ui.label("Chance:");
                let mut pct = params.chance_percent();
                if ui
                    .add(egui::DragValue::new(&mut pct).speed(0.1).range(0.0..=100.0).suffix("%").max_decimals(1))
                    .changed()
                {
                    params.set_chance_percent(pct);
                    self.mark_dirty();
                }
            }
            ModuleKind::Reset => {
                let enabled = params.enabled.load(Ordering::Relaxed);
                if ui
                    .add_enabled(enabled, egui::Button::new("RST next").small())
                    .on_hover_text("Force a TCP RST on the next matching packet")
                    .clicked()
                {
                    params.rst_next.fetch_add(1, Ordering::Relaxed);
                }
                ui.label("Chance:");
                let mut pct = params.chance_percent();
                if ui
                    .add(egui::DragValue::new(&mut pct).speed(0.1).range(0.0..=100.0).suffix("%").max_decimals(1))
                    .changed()
                {
                    params.set_chance_percent(pct);
                    self.mark_dirty();
                }
            }
            ModuleKind::Bandwidth => {
                ui.label("Limit:");
                let mut limit = params.bandwidth_limit_kbps.load(Ordering::Relaxed);
                if ui
                    .add(egui::DragValue::new(&mut limit).speed(10.0).range(0..=99999).suffix(" KB/s"))
                    .changed()
                {
                    params.bandwidth_limit_kbps.store(limit, Ordering::Relaxed);
                    self.mark_dirty();
                }
            }
        }
    }

    // ---------------------------------------------------------------- Activity
    fn ui_activity_section(&mut self, ui: &mut egui::Ui) {
        let stats = self
            .engine
            .as_ref()
            .map(|e| e.shared.stats.snapshot())
            .unwrap_or_default();

        ui.group(|ui| {
            ui.set_width(ui.available_width());

            // Header line with collapse button and stats summary
            ui.horizontal(|ui| {
                let chevron = if self.show_log { "▾" } else { "▸" };
                if ui.button(format!("{chevron} Activity")).clicked() {
                    self.show_log = !self.show_log;
                    self.mark_dirty();
                }

                ui.separator();
                ui.label(
                    egui::RichText::new(format!(
                        "Cap: {} · Sent: {} · Drop: {} · Mod: {} · Fail: {}",
                        stats.captured, stats.sent, stats.dropped, stats.modified, stats.send_failed
                    )).monospace().size(11.0)
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear").on_hover_text("Clear event log").clicked() {
                        if let Some(e) = &self.engine {
                            e.shared.log.clear();
                        }
                        self.log_cache.clear();
                    }
                    if ui.button("Copy").on_hover_text("Copy activity log").clicked() {
                        let text = self
                            .log_cache
                            .iter()
                            .map(|e| format_log_line(e))
                            .collect::<Vec<_>>()
                            .join("\n");
                        ui.ctx().copy_text(text);
                    }
                });
            });

            if self.show_log {
                ui.add_space(2.0);

                // Fixed-height, non-jittering throughput plot
                self.ui_throughput_plot(ui);

                ui.add_space(2.0);

                // Log table
                egui::ScrollArea::vertical()
                    .max_height(100.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        egui::Grid::new("log_grid")
                            .num_columns(3)
                            .striped(true)
                            .spacing([8.0, 2.0])
                            .show(ui, |ui| {
                                for ev in &self.log_cache {
                                    let t = format_unix_ms(ev.unix_ms);
                                    ui.monospace(&t);
                                    ui.monospace(
                                        ev.module
                                            .map(|m| m.display_name().to_string())
                                            .unwrap_or_else(|| "engine".into()),
                                    );
                                    let color = match ev.level {
                                        crate::engine::LogLevel::Info => ui.visuals().text_color(),
                                        crate::engine::LogLevel::Traffic => ok_color(ui),
                                        crate::engine::LogLevel::Warn => warn_color(ui),
                                        crate::engine::LogLevel::Error => error_color(ui),
                                    };
                                    ui.colored_label(color, &ev.text);
                                    ui.end_row();
                                }
                            });
                    });
            }
        });
    }

    fn ui_throughput_plot(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        let mut captured_pts = Vec::with_capacity(self.history_samples.len() + 1);
        let mut sent_pts = Vec::with_capacity(self.history_samples.len() + 1);

        for (t, cap, sent) in &self.history_samples {
            let x = -(now.duration_since(*t).as_secs_f64());
            captured_pts.push([x, *cap]);
            sent_pts.push([x, *sent]);
        }
        if captured_pts.is_empty() {
            captured_pts.push([-30.0, 0.0]);
            captured_pts.push([0.0, 0.0]);
            sent_pts.push([-30.0, 0.0]);
            sent_pts.push([0.0, 0.0]);
        }

        let line_captured = Line::new("captured/s", PlotPoints::from(captured_pts))
            .stroke(egui::Stroke::new(1.5, ok_color(ui)));
        let line_sent = Line::new("sent/s", PlotPoints::from(sent_pts))
            .stroke(egui::Stroke::new(1.5, ui.visuals().strong_text_color()));

        Plot::new("throughput")
            .legend(Legend::default().position(egui_plot::Corner::LeftTop))
            .height(80.0)
            .show_axes([false, true])
            .include_x(-30.0)
            .include_x(0.0)
            .include_y(0.0)
            .include_y(10.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show(ui, |plot_ui| {
                plot_ui.line(line_captured);
                plot_ui.line(line_sent);
            });
    }

    fn sample_history(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_sample).as_secs_f64();
        if dt < 0.25 {
            return;
        }
        let stats = self
            .engine
            .as_ref()
            .map(|e| e.shared.stats.snapshot())
            .unwrap_or_default();
        let cap_rate = (stats.captured.saturating_sub(self.last_stats.captured)) as f64 / dt;
        let sent_rate = (stats.sent.saturating_sub(self.last_stats.sent)) as f64 / dt;
        self.history_samples.push((now, cap_rate, sent_rate));
        self.history_samples.retain(|(t, _, _)| now.duration_since(*t).as_secs_f64() <= 30.0);
        self.last_stats = stats;
        self.last_sample = now;
    }

    // ---------------------------------------------------------------- preset window

    fn ui_preset_window(&mut self, ctx: &egui::Context) {
        if !self.preset_window_open {
            return;
        }
        let mut open = self.preset_window_open;
        egui::Window::new("Save preset")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Filter to save:");
                ui.monospace(&self.config.filter);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.preset_name_edit);
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let can_save = !self.preset_name_edit.trim().is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save"))
                        .clicked()
                    {
                        self.config.presets.push(crate::config::Preset {
                            name: self.preset_name_edit.trim().to_string(),
                            filter: self.config.filter.clone(),
                            builtin: false,
                        });
                        self.preset_selected = self.config.presets.len() - 1;
                        self.mark_dirty();
                        self.preset_window_open = false;
                    }
                    // list custom presets with delete buttons
                    ui.separator();
                    ui.label("Custom presets:");
                });
                let mut to_delete: Option<usize> = None;
                for (i, p) in self.config.presets.iter().enumerate() {
                    if p.builtin {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("✕").clicked() {
                            to_delete = Some(i);
                        }
                        ui.label(&p.name);
                    });
                }
                if let Some(i) = to_delete {
                    self.config.presets.remove(i);
                    self.preset_selected = usize::MAX;
                    self.mark_dirty();
                }
            });
        self.preset_window_open = open;
    }
}

// ---------------------------------------------------------------- helpers

fn draw_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn ok_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0x6a, 0xd4, 0x6a)
    } else {
        egui::Color32::from_rgb(0x2d, 0x8a, 0x2d)
    }
}

fn error_color(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().error_fg_color
}

fn warn_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(0xe5, 0xc0, 0x4a)
    } else {
        egui::Color32::from_rgb(0x9a, 0x71, 0x0a)
    }
}

fn idle_color(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().weak_text_color()
}

fn format_unix_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let frac = ms % 1000;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // only render time-of-day (UTC)
    format!("{h:02}:{m:02}:{s:02}.{frac:03}")
}

fn format_log_line(ev: &crate::engine::LogEvent) -> String {
    let module = ev
        .module
        .map(|m| m.display_name().to_string())
        .unwrap_or_else(|| "engine".into());
    format!("{} [{module}] {}", format_unix_ms(ev.unix_ms), ev.text)
}
