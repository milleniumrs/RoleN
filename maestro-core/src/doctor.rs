//! `maestro config doctor` — environment diagnostics (PRD FR-14.3).

use crate::{config, config::Config, ledger::Ledger, secrets};

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
        }
    }
}

pub fn run_all() -> Vec<Check> {
    let mut out = Vec::new();

    // 1. config + data directories
    let dirs_ok = match config::ensure_dirs() {
        Ok((cfg, data)) => {
            out.push(Check::ok(
                "config/data dirs",
                format!("{} ; {}", cfg.display(), data.display()),
            ));
            true
        }
        Err(e) => {
            out.push(Check::fail("config/data dirs", e.to_string()));
            false
        }
    };

    // 2. config file loads (creating a default if needed)
    let mut cfg_opt: Option<Config> = None;
    if dirs_ok {
        match Config::ensure() {
            Ok((cfg, created)) => {
                out.push(Check::ok(
                    "config.toml",
                    if created {
                        "created default".to_string()
                    } else {
                        "loaded".to_string()
                    },
                ));
                cfg_opt = Some(cfg);
            }
            Err(e) => out.push(Check::fail("config.toml", e.to_string())),
        }
    }

    // 3. workspace root exists / creatable
    if let Some(cfg) = &cfg_opt {
        match cfg.ensure_workspace_root() {
            Ok(()) => out.push(Check::ok(
                "workspace root",
                cfg.general.workspace_root.display().to_string(),
            )),
            Err(e) => out.push(Check::fail("workspace root", e.to_string())),
        }
    }

    // 4. secret backend: OS keychain, else age vault fallback (FR-2.1/FR-2.2)
    match secrets::keychain_probe() {
        Ok(()) => out.push(Check::ok("keychain", "roundtrip ok (primary backend)")),
        Err(e) => match crate::vault::probe() {
            Ok(()) => out.push(Check::ok(
                "secrets vault",
                format!("keychain unavailable ({e}); age vault fallback ok"),
            )),
            Err(ve) => out.push(Check::fail(
                "secrets",
                format!("keychain: {e}; vault: {ve} (set MAESTRO_VAULT_PASSWORD or fix keychain)"),
            )),
        },
    }

    // 5. SQLite ledger open + schema + write probe
    match Ledger::open_default().and_then(|l| l.probe().map(|_| l)) {
        Ok(l) => {
            let n = l.count_entries().unwrap_or(0);
            out.push(Check::ok(
                "sqlite ledger",
                format!("write probe ok, {n} entries"),
            ));
        }
        Err(e) => out.push(Check::fail("sqlite ledger", e.to_string())),
    }

    out
}

/// True when every check passed.
pub fn all_ok(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.ok)
}
