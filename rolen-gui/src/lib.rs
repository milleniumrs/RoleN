//! RoleN's desktop GUI front-end.
//!
//! A peer of `rolen-tui`, not a replacement: both drive the same UI-free
//! core. The split that matters here is threading. Every core API is blocking -
//! synchronous HTTP, SQLite, subprocesses - so this crate is built around two
//! rules:
//!
//! 1. Periodic reads happen on one poller thread that owns a persistent ledger
//!    connection and publishes an immutable [`state::Snapshot`].
//! 2. User-initiated slow work goes through [`jobs::Jobs`], which runs it on a
//!    worker thread and wakes the UI when the result lands.
//!
//! The UI thread itself performs no IO.

pub mod app;
pub mod dialogs;
pub mod jobs;
pub mod menu;
pub mod state;
pub mod text;
pub mod views;

/// Launch the desktop window. Blocks until it closes.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("RoleN"),
        ..Default::default()
    };
    eframe::run_native(
        "RoleN",
        options,
        Box::new(|cc| Ok(Box::new(app::RoleNApp::new(cc)))),
    )
}
