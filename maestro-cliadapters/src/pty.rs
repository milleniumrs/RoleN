//! PTY spawning via portable-pty (D7: ConPTY on Windows, posix PTY elsewhere).
//!
//! Reads happen on a dedicated thread reporting through a channel: the main
//! loop can then enforce an overall timeout and terminate cleanly when the
//! child exits (some children leave daemons holding the PTY open, so a naive
//! blocking read never sees EOF).

use crate::error::AdapterError;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

pub struct PtyResult {
    pub exit_code: Option<i32>,
    /// True when the global timeout fired and the child was killed.
    pub timed_out: bool,
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub fn run_pty(
    program: &Path,
    args: &[String],
    cwd: &Path,
    on_chunk: &mut dyn FnMut(&str),
) -> Result<PtyResult, AdapterError> {
    run_pty_timeout(program, args, cwd, DEFAULT_TIMEOUT, on_chunk)
}

pub fn run_pty_timeout(
    program: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    on_chunk: &mut dyn FnMut(&str),
) -> Result<PtyResult, AdapterError> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 40,
        cols: 140,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = if program
        .extension()
        .map(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
        .unwrap_or(false)
    {
        // script files need a shell host under ConPTY
        let mut c = CommandBuilder::new("cmd.exe");
        c.arg("/c");
        c.arg(program);
        for a in args {
            c.arg(a);
        }
        c
    } else {
        let mut c = CommandBuilder::new(program);
        for a in args {
            c.arg(a);
        }
        c
    };
    cmd.cwd(cwd);

    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let master = pair.master;

    // reader thread → channel
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let exit_code = loop {
        // drain available output
        while let Ok(chunk) = rx.try_recv() {
            on_chunk(&String::from_utf8_lossy(&chunk));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // child exited; drain remaining output briefly
                let drain_deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < drain_deadline {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(chunk) => on_chunk(&String::from_utf8_lossy(&chunk)),
                        Err(_) => break,
                    }
                }
                break if status.success() {
                    Some(0)
                } else {
                    Some(status.exit_code() as i32)
                };
            }
            Ok(None) => {}
            Err(e) => return Err(AdapterError::Pty(format!("try_wait: {e}"))),
        }
        if Instant::now() > deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(80));
    };
    drop(master);
    Ok(PtyResult {
        exit_code,
        timed_out,
    })
}
