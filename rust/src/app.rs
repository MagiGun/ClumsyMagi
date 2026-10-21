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

    history: Vec<[f64; 3]>, // [t_secs, captured_per_sec, sent_per_sec]
    history_t: f64,
    last_stats: StatsSnapshot,
    last_sample: Instant,

    pub elevated: bool,
    pub quit_deadline: Option<Instant>,
    preset_name_edit: String,
    preset_window_open: bool,
    applied_theme: Option<Theme>,
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
            history: Vec::new(),
            history_t: 0.0,
            last_stats: StatsSnapshot::default(),
            last_sample: Instant::now(),
            elevated,
            quit_deadline: None,
            preset_name_edit: String::new(),
            preset_window_open: false,
            applied_theme: None,
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
}

impl eframe::App for ClumsyMagiApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();

        // theme (only poke the context when the preference actually changed)
        if self.applied_theme != Some(self.config.theme) {
            let theme_pref = match self.config.theme {
                Theme::System => egui::ThemePreference::System,
                Theme::Dark => egui::ThemePreference::Dark,
                Theme::Light => egui::ThemePreference::Light,
            };
            ctx.set_theme(theme_pref);
            self.applied_theme = Some(self.config.theme);
        }

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

        // Schedule repaints so the UI stays responsive without requiring mouse movement:
        // - fast (50ms ~ 20 FPS) when filtering is running for smooth throughput plot & activity dots
        // - or near the quit deadline if one is set
        // - low rate (250ms) when idle to check logs and engine state
        let repaint_delay = if self.engine.as_ref().map(|e| e.is_running()).unwrap_or(false) {
            Duration::from_millis(50)
        } else if let Some(deadline) = self.quit_deadline {
            deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(100))
        } else {
            Duration::from_millis(250)
        };
        ctx.request_repaint_after(repaint_delay);

        self.ui_header(root);
        self.ui_log_panel(root);
        self.ui_functions(root);
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
    // ---------------------------------------------------------------- header

    fn ui_header(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("header").show(root, |ui| {
            ui.add_space(6.0);

            // --- filter row
            ui.horizontal(|ui| {
                let running = self.engine.is_some() && self.engine.as_ref().unwrap().is_running();

                // global send-state dot: green while sending, red on failure
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

                let btn_text = if running { "Stop" } else { "Start" };
                let btn = egui::Button::new(egui::RichText::new(btn_text).strong());
                if ui.add(btn).clicked() {
                    if running {
                        self.stop_filtering();
                    } else {
                        self.start_filtering();
                    }
                }

                let filter_edit = egui::TextEdit::singleline(&mut self.config.filter)
                    .hint_text("WinDivert filter, e.g. udp and outbound")
                    .desired_width(ui.available_width() - 240.0);
                let response = ui.add_enabled(!running, filter_edit);
                if response.changed() {
                    self.mark_dirty();
                    self.preset_selected = usize::MAX;
                }

                // presets dropdown and save button (disabled while filtering)
                ui.add_enabled_ui(!running, |ui| {
                    let preset_names: Vec<String> = self.config.presets.iter().map(|p| p.name.clone()).collect();
                    let selected_text = if self.preset_selected < preset_names.len() {
                        preset_names[self.preset_selected].clone()
                    } else {
                        "custom".to_string()
                    };
                    egui::ComboBox::from_id_salt("presets")
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
                    if ui.button("Save preset").clicked() {
                        self.preset_window_open = true;
                        self.preset_name_edit.clear();
                    }
                });

                // theme toggle
                let next = match self.config.theme {
                    Theme::Dark => Theme::Light,
                    Theme::Light => Theme::System,
                    Theme::System => Theme::Dark,
                };
                let theme_label = match next {
                    Theme::Light => "Light",
                    Theme::Dark => "Dark",
                    Theme::System => "System",
                };
                if ui
                    .button(format!("Theme: {theme_label}"))
                    .on_hover_text("Switch between dark, light, and system theme")
                    .clicked()
                {
                    self.config.theme = next;
                    self.mark_dirty();
                }
            });

            // --- filter validation error / status
            ui.horizontal(|ui| {
                if let Err(e) = &self.filter_validation {
                    ui.colored_label(error_color(ui), format!("⚠ {e}"));
                }
            });

            // --- status line
            ui.horizontal(|ui| {
                if !self.status.is_empty() {
                    if self.status_error {
                        ui.colored_label(error_color(ui), &self.status);
                    } else {
                        ui.label(&self.status);
                    }
                } else if !self.elevated {
                    ui.colored_label(
                        warn_color(ui),
                        "⚠ Not running as Administrator: filtering will fail. Use the button below to elevate.",
                    );
                } else {
                    ui.label(
                        egui::RichText::new(
                            "NOTICE: loopback packets can only be captured with 'outbound' (e.g. 'udp and outbound'), \
                             never with inbound criteria.",
                        )
                        .weak()
                        .small(),
                    );
                }
            });

            // --- elevation banner
            if !self.elevated {
                ui.horizontal(|ui| {
                    if ui.button("🛡 Restart as Administrator").clicked() {
                        let args = vec!["--elevated-relaunch".to_string()];
                        match crate::platform::relaunch_elevated(&args) {
                            Ok(()) => {
                                // new instance takes over; save config and exit
                                self.maybe_save_config(true);
                                std::process::exit(0);
                            }
                            Err(e) => self.set_status(e, true),
                        }
                    }
                });
            }

            ui.add_space(4.0);
        });
    }

    // ---------------------------------------------------------------- functions

    fn ui_functions(&mut self, root: &mut egui::Ui) {
        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("Functions");
            ui.add_space(2.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for kind in crate::engine::all_modules() {
                    let params = self.params[kind.index()].clone();
                    self.module_row(ui, *kind, &params);
                    ui.add_space(4.0);
                }
            });
        });
    }

    fn module_row(&mut self, ui: &mut egui::Ui, kind: ModuleKind, params: &Arc<ModuleParams>) {
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                // activity dot
                let active = match &self.engine {
                    Some(e) => match e.shared.since_triggered(kind) {
                        Some(ms) => ms < ACTIVITY_WINDOW_MS as u64,
                        None => false,
                    },
                    None => false,
                };
                draw_dot(ui, if active { ok_color(ui) } else { idle_color(ui) });

                let mut enabled = params.enabled.load(Ordering::Relaxed);
                if ui
                    .checkbox(&mut enabled, kind.display_name())
                    .changed()
                {
                    params.enabled.store(enabled, Ordering::Relaxed);
                    self.mark_dirty();
                }

                ui.separator();
                self.module_controls(ui, kind, params);
            });
        });
    }

    fn module_controls(&mut self, ui: &mut egui::Ui, kind: ModuleKind, params: &Arc<ModuleParams>) {
        // direction toggles helper
        macro_rules! direction_toggles {
            () => {{
                let mut inbound = params.direction.inbound.load(Ordering::Relaxed);
                let mut outbound = params.direction.outbound.load(Ordering::Relaxed);
                if ui.checkbox(&mut inbound, "Inbound").changed() {
                    params.direction.inbound.store(inbound, Ordering::Relaxed);
                    self.mark_dirty();
                }
                if ui.checkbox(&mut outbound, "Outbound").changed() {
                    params.direction.outbound.store(outbound, Ordering::Relaxed);
                    self.mark_dirty();
                }
            }};
        }

        macro_rules! chance_control {
            () => {{
                ui.label("Chance:");
                let mut pct = params.chance_percent();
                if ui
                    .add(
                        egui::DragValue::new(&mut pct)
                            .speed(0.1)
                            .range(0.0..=100.0)
                            .suffix("%")
                            .max_decimals(1),
                    )
                    .changed()
                {
                    params.set_chance_percent(pct);
                    self.mark_dirty();
                }
            }};
        }

        macro_rules! int_control {
            ($label:expr, $field:ident, $min:literal, $max:literal) => {{
                ui.label($label);
                let mut v = params.$field.load(Ordering::Relaxed);
                if ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .range($min..=$max)
                            .suffix(if stringify!($field).contains("ms") { "ms" } else { "" }),
                    )
                    .changed()
                {
                    params.$field.store(v, Ordering::Relaxed);
                    self.mark_dirty();
                }
            }};
        }

        match kind {
            ModuleKind::Lag => {
                direction_toggles!();
                int_control!("Delay:", delay_ms, 0, 15000);
            }
            ModuleKind::Drop => {
                direction_toggles!();
                chance_control!();
            }
            ModuleKind::Throttle => {
                let mut drop_t = params.drop_throttled.load(Ordering::Relaxed);
                if ui.checkbox(&mut drop_t, "Drop Throttled").changed() {
                    params.drop_throttled.store(drop_t, Ordering::Relaxed);
                    self.mark_dirty();
                }
                int_control!("Timeframe:", throttle_frame_ms, 0, 1000);
                direction_toggles!();
                chance_control!();
            }
            ModuleKind::Duplicate => {
                int_control!("Count:", dup_count, 2, 50);
                direction_toggles!();
                chance_control!();
            }
            ModuleKind::Ood => {
                direction_toggles!();
                chance_control!();
            }
            ModuleKind::Tamper => {
                let mut redo = params.redo_checksum.load(Ordering::Relaxed);
                if ui.checkbox(&mut redo, "Redo Checksum").changed() {
                    params.redo_checksum.store(redo, Ordering::Relaxed);
                    self.mark_dirty();
                }
                direction_toggles!();
                chance_control!();
            }
            ModuleKind::Reset => {
                let enabled = params.enabled.load(Ordering::Relaxed);
                if ui
                    .add_enabled(enabled, egui::Button::new("RST next packet"))
                    .on_hover_text("Force a TCP RST on the next matching packet (only while enabled)")
                    .clicked()
                {
                    params.rst_next.fetch_add(1, Ordering::Relaxed);
                }
                direction_toggles!();
                chance_control!();
            }
            ModuleKind::Bandwidth => {
                direction_toggles!();
                int_control!("Limit(KB/s):", bandwidth_limit_kbps, 0, 99999);
            }
        }
    }

    // ---------------------------------------------------------------- log panel

    fn ui_log_panel(&mut self, root: &mut egui::Ui) {
        let stats = self
            .engine
            .as_ref()
            .map(|e| e.shared.stats.snapshot())
            .unwrap_or_default();

        egui::Panel::bottom("log_panel")
            .resizable(true)
            .default_size(200.0)
            .min_size(64.0)
            .show(root, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let chevron = if self.show_log { "▾" } else { "▸" };
                    if ui.button(format!("{chevron} Activity")).clicked() {
                        self.show_log = !self.show_log;
                        self.mark_dirty();
                    }
                    ui.separator();
                    ui.label(format!(
                        "Captured {} · Sent {} · Dropped {} · Modified {} · Send failed {}",
                        stats.captured, stats.sent, stats.dropped, stats.modified, stats.send_failed
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Clear").clicked() {
                            if let Some(e) = &self.engine {
                                e.shared.log.clear();
                            }
                            self.log_cache.clear();
                        }
                        if ui.button("Copy").on_hover_text("Copy the whole event log").clicked() {
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
                    ui.add_space(4.0);
                    self.ui_throughput_plot(ui);

                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            egui::Grid::new("log_grid")
                                .num_columns(3)
                                .striped(true)
                                .min_col_width(0.0)
                                .show(ui, |ui| {
                                    for ev in &self.log_cache {
                                        let t = format_unix_ms(ev.unix_ms);
                                        ui.monospace(t);
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
                ui.add_space(2.0);
            });
    }

    fn ui_throughput_plot(&mut self, ui: &mut egui::Ui) {
        if self.history.len() < 2 {
            ui.set_min_height(70.0);
            ui.weak("waiting for traffic…");
            return;
        }
        let captured: PlotPoints = self
            .history
            .iter()
            .map(|p| [p[0], p[1]])
            .collect::<Vec<[f64; 2]>>()
            .into();
        let sent: PlotPoints = self
            .history
            .iter()
            .map(|p| [p[0], p[2]])
            .collect::<Vec<[f64; 2]>>()
            .into();

        let line_captured = Line::new("captured/s", captured).stroke(egui::Stroke::new(1.5, ok_color(ui)));
        let line_sent =
            Line::new("sent/s", sent).stroke(egui::Stroke::new(1.5, ui.visuals().strong_text_color()));
        Plot::new("throughput")
            .legend(Legend::default())
            .height(90.0)
            .show_axes([false, true])
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
        self.history_t += dt;
        self.history.push([self.history_t, cap_rate, sent_rate]);
        // keep the last ~30 seconds (at 4Hz => 120 points); rebase t to zero
        // so the x axis stays bounded
        while self.history.len() > 120 {
            self.history.remove(0);
        }
        let t0 = self.history[0][0];
        for p in self.history.iter_mut() {
            p[0] -= t0;
        }
        self.history_t -= t0;
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
