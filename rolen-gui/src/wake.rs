//! Waking the UI thread from background work.
//!
//! The shell hands the library a callback (backed by a winit
//! `EventLoopProxy`) that posts a user event, so the event loop redraws
//! instead of sleeping through a finished job or a fresh snapshot.

use std::sync::Arc;

/// A cheap-to-clone "wake the UI" callback.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A waker that does nothing. Used by tests, where nothing needs waking.
pub fn no_op() -> Wake {
    Arc::new(|| {})
}
