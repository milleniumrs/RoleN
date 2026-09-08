//! Process-global per-provider session limiter (FR-8.1).
//!
//! The scheduler caps total concurrent tasks; this caps how many of those
//! agent sessions may use the same provider at once. A permit is held for the
//! whole agent session and moved across FR-3.3 provider migrations.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

struct ProviderSlot {
    active: Mutex<usize>,
    cv: Condvar,
}

impl Default for ProviderSlot {
    fn default() -> Self {
        Self {
            active: Mutex::new(0),
            cv: Condvar::new(),
        }
    }
}

static SLOTS: OnceLock<Mutex<HashMap<String, Arc<ProviderSlot>>>> = OnceLock::new();

fn slots() -> &'static Mutex<HashMap<String, Arc<ProviderSlot>>> {
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn slot_for(provider_id: &str) -> Arc<ProviderSlot> {
    slots()
        .lock()
        .unwrap()
        .entry(provider_id.to_string())
        .or_default()
        .clone()
}

/// RAII permit for one active session on a provider.
pub struct ProviderPermit {
    slot: Arc<ProviderSlot>,
    counted: bool,
}

impl Drop for ProviderPermit {
    fn drop(&mut self) {
        if !self.counted {
            return;
        }
        let mut active = self.slot.active.lock().unwrap();
        *active = active.saturating_sub(1);
        self.slot.cv.notify_one();
    }
}

/// Acquire a session permit using the configured `parallelism.per_provider_cap`.
pub fn acquire(provider_id: &str) -> ProviderPermit {
    let cap = rolen_core::config::Config::load()
        .map(|c| c.parallelism.per_provider_cap)
        .unwrap_or(2);
    acquire_with_cap(provider_id, cap)
}

/// Acquire a session permit with an explicit cap. `0` means unlimited.
pub fn acquire_with_cap(provider_id: &str, cap: usize) -> ProviderPermit {
    let slot = slot_for(provider_id);
    if cap == 0 {
        return ProviderPermit {
            slot,
            counted: false,
        };
    }
    let mut active = slot.active.lock().unwrap();
    while *active >= cap {
        active = slot.cv.wait(active).unwrap();
    }
    *active += 1;
    drop(active);
    ProviderPermit {
        slot,
        counted: true,
    }
}

#[cfg(test)]
fn active_count(provider_id: &str) -> usize {
    *slot_for(provider_id).active.lock().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn provider(tag: &str) -> String {
        format!(
            "limit-{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        )
    }

    #[test]
    fn drop_releases_the_slot() {
        let p = provider("drop");
        let permit = acquire_with_cap(&p, 1);
        assert_eq!(active_count(&p), 1);
        drop(permit);
        assert_eq!(active_count(&p), 0);
    }

    #[test]
    fn zero_cap_means_unlimited() {
        let p = provider("zero");
        let _a = acquire_with_cap(&p, 0);
        let _b = acquire_with_cap(&p, 0);
        let _c = acquire_with_cap(&p, 0);
        assert_eq!(active_count(&p), 0);
    }

    #[test]
    fn a_second_session_waits_until_the_first_releases() {
        let p = provider("block");
        let first = acquire_with_cap(&p, 1);
        let (tx, rx) = channel();
        let waiter = std::thread::spawn({
            let p = p.clone();
            move || {
                let _second = acquire_with_cap(&p, 1);
                let _ = tx.send(());
            }
        });
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
        drop(first);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        waiter.join().unwrap();
        assert_eq!(active_count(&p), 0);
    }
}
