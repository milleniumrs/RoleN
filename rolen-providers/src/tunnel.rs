//! Ollama-over-SSH support: manages `ssh -N -L` local port-forwards so a
//! remote Ollama server (e.g. in docker on another host) appears as
//! `http://127.0.0.1:<local_port>`. Uses the system ssh client with the
//! user's default key resolution (~/.ssh, ssh-agent). BatchMode is on, so a
//! missing key/passphrase fails fast instead of hanging on a prompt.

use crate::error::ProviderError;
use rolen_core::types::{Provider, TunnelSpec};
use std::collections::HashMap;
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static TUNNELS: LazyLock<Mutex<HashMap<String, Child>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
const UP_TIMEOUT: Duration = Duration::from_secs(20);

/// If the provider has a tunnel spec, make sure the forward is up and return
/// the local endpoint. Returns None for providers without a tunnel.
pub fn local_endpoint(provider: &Provider) -> Result<Option<String>, ProviderError> {
    let Some(spec) = &provider.tunnel else {
        return Ok(None);
    };
    ensure_up(&provider.id, spec)?;
    Ok(Some(format!("http://127.0.0.1:{}", spec.local_port)))
}

/// Is something already listening on the local forward port?
fn port_open(local_port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{local_port}").parse().unwrap(),
        CONNECT_TIMEOUT,
    )
    .is_ok()
}

fn ensure_up(provider_id: &str, spec: &TunnelSpec) -> Result<(), ProviderError> {
    // Reuse a healthy existing forward (ours or one the user started manually).
    if port_open(spec.local_port) {
        reap_dead(provider_id);
        return Ok(());
    }

    let mut cmd = Command::new("ssh");
    cmd.arg("-N") // no remote command
        .arg("-T") // no pty
        .arg("-o")
        .arg("BatchMode=yes") // never prompt (keys/agent only)
        .arg("-o")
        .arg("ConnectTimeout=10") // fail fast on dead hosts
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(format!(
            "127.0.0.1:{}:{}:{}",
            spec.local_port, spec.remote_host, spec.remote_port
        ))
        .arg("-p")
        .arg(spec.port.to_string());
    if let Some(identity) = &spec.identity_file {
        cmd.arg("-i").arg(identity);
    }
    cmd.arg(format!("{}@{}", spec.user, spec.host))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd.spawn().map_err(|e| {
        ProviderError::Api(format!("failed to spawn ssh (is OpenSSH installed?): {e}"))
    })?;

    TUNNELS
        .lock()
        .unwrap()
        .insert(provider_id.to_string(), child);

    // Wait for the forward to come up.
    let deadline = Instant::now() + UP_TIMEOUT;
    while Instant::now() < deadline {
        if port_open(spec.local_port) {
            return Ok(());
        }
        // Bail early if ssh already died (bad host/key/port).
        let exited = TUNNELS
            .lock()
            .unwrap()
            .get_mut(provider_id)
            .and_then(|c| c.try_wait().ok().flatten());
        if let Some(status) = exited {
            return Err(ProviderError::Api(format!(
                "ssh tunnel to {}@{}:{} exited early ({status}) — check host reachability and that key auth works non-interactively (ssh-agent or passphrase-less key in ~/.ssh)",
                spec.user, spec.host, spec.port
            )));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(ProviderError::Api(format!(
        "ssh tunnel to {}@{}:{} did not come up within {}s",
        spec.user,
        spec.host,
        spec.port,
        UP_TIMEOUT.as_secs()
    )))
}

fn reap_dead(provider_id: &str) {
    let mut map = TUNNELS.lock().unwrap();
    let dead = map
        .get_mut(provider_id)
        .and_then(|c| c.try_wait().ok().flatten())
        .is_some();
    if dead {
        map.remove(provider_id);
    }
}

/// Stop a tunnel we spawned (best effort).
pub fn close(provider_id: &str) {
    if let Some(mut child) = TUNNELS.lock().unwrap().remove(provider_id) {
        let _ = child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tunnel_specs() {
        let t = TunnelSpec::parse("vcraciun@vcraciun.ddns.net:5050").unwrap();
        assert_eq!(t.user, "vcraciun");
        assert_eq!(t.host, "vcraciun.ddns.net");
        assert_eq!(t.port, 5050);
        assert_eq!(t.remote_port, 11434);
        assert_eq!(t.local_port, 11435);

        let t = TunnelSpec::parse("u@host").unwrap();
        assert_eq!(t.port, 22);

        assert!(TunnelSpec::parse("host-only").is_err());
        assert!(TunnelSpec::parse("u@host:notaport").is_err());
        assert!(TunnelSpec::parse("@host").is_err());
    }
}
