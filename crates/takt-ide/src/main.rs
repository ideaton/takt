//! Takt Studio : IDE façon TIA Portal pour les automates Takt.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor;
mod iface_ui;
mod instructions;
mod ladder_ui;
mod sfc_ui;
mod target;
mod theme;
mod views;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Takt Studio")
            .with_icon(std::sync::Arc::new(theme::window_icon()))
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    let path = std::env::args().nth(1).map(std::path::PathBuf::from);
    eframe::run_native("Takt Studio", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, path)))))
}
