//! ClumsyMagi: makes your network condition on Windows significantly worse,
//! in a managed and interactive manner. Rust + egui remake of clumsy.

mod app;
mod cli;
mod config;
mod engine;
mod packet;
mod platform;
mod process_target;
mod selftest;
mod wd;

use app::ClumsyMagiApp;
use eframe::egui;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn load_icon() -> egui::IconData {
    let png = include_bytes!("../assets/clumsy-icon.png");
    let img = image::load_from_memory(png).expect("embedded icon should decode");
    let rgba = img.to_rgba8();
    egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(if selftest::run() { 0 } else { 1 });
    }

    let mut config = config::load();
    let cli = match cli::parse(&args, &mut config) {
        Ok(o) => o,
        Err(msg) => {
            let is_help = args.iter().any(|a| a == "--help" || a == "-h");
            if is_help {
                println!("{msg}");
                return;
            }
            eprintln!("{msg}");
            if crate::platform::is_elevated() || !cfg!(windows) {
                // no window manager guarantees; just print
            } else {
                platform::error_box(&msg);
            }
            std::process::exit(2);
        }
    };

    // By default, hide the console window when running as a GUI application unless
    // explicitly requested via --show-console. A UI button allows toggling it on demand.
    if !cli.show_console {
        platform::hide_console();
    }

    // single instance guard; the elevated relaunch path retries while the old
    // (unelevated) instance is shutting down
    let mut guard = None;
    let attempts = if cli.elevated_relaunch { 25 } else { 1 };
    for attempt in 0..attempts {
        if let Some(g) = platform::SingleInstance::acquire() {
            guard = Some(g);
            break;
        }
        if attempt + 1 < attempts {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let Some(_guard) = guard else {
        platform::error_box("Another ClumsyMagi instance is already running.");
        return;
    };

    let elevated = platform::is_elevated();
    let lib = match wd::WinDivertLib::load() {
        Ok(l) => Ok(Arc::new(l)),
        Err(e) => Err(e.friendly()),
    };

    let mut app = ClumsyMagiApp::new(config, lib, elevated);
    app.quit_deadline = cli.quit_after_secs.map(|s| Instant::now() + Duration::from_secs(s));
    if cli.auto_start {
        app.start_filtering();
    }

    let initial_pos = cli
        .window_pos
        .unwrap_or_else(|| platform::get_centered_window_position(580.0, 680.0));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("ClumsyMagi {}", app::VERSION))
            .with_inner_size([580.0, 680.0])
            .with_min_inner_size([520.0, 520.0])
            .with_position(initial_pos)
            .with_icon(Arc::new(load_icon())),
        ..Default::default()
    };

    if let Err(e) = eframe::run_native(
        "ClumsyMagi",
        options,
        Box::new(move |_cc| Ok(Box::new(app) as Box<dyn eframe::App>)),
    ) {
        eprintln!("eframe error: {e}");
        platform::error_box(&format!("ClumsyMagi failed to start its window: {e}"));
        std::process::exit(1);
    }
}
