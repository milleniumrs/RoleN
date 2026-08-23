//! RoleN's desktop GUI front-end, built on Dear ImGui (`dear-imgui-rs`).
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
//!
//! This crate is a library plus a thin `main.rs` binary. The binary owns the
//! winit/glutin/glow shell (window, GL context, event loop); everything that
//! decides *what* to draw lives here and is testable headless, because
//! `dear_imgui_rs::Context` can build frames without a window or GPU.

pub mod app;
pub mod dialogs;
pub mod jobs;
pub mod menu;
pub mod state;
pub mod text;
pub mod views;
pub mod wake;

pub use wake::Wake;
