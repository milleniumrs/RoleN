//! OS toast notifications (PRD FR-9.4, opt-in via
//! `general.os_notifications` in config.toml). Best effort: a failure to
//! show a toast is never an error, so callers can fire-and-forget.
//!
//! No extra crates: Windows uses a PowerShell balloon tip, macOS
//! `osascript`, Linux `notify-send`.

/// Show an OS notification. Silently does nothing on failure or when the
/// platform has no notifier available.
pub fn toast(title: &str, body: &str) {
    let esc = |s: &str| s.replace('\'', "''").replace('\n', " ");
    let title = esc(title);
    let body = esc(body);
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        {
            let script = format!(
                "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
                 $n = New-Object System.Windows.Forms.NotifyIcon; \
                 $n.Icon = [System.Drawing.SystemIcons]::Information; \
                 $n.Visible = $true; \
                 $n.ShowBalloonTip(5000, '{title}', '{body}', [System.Windows.Forms.ToolTipIcon]::Info); \
                 Start-Sleep 6; $n.Dispose()"
            );
            let _ = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-WindowStyle",
                    "Hidden",
                    "-Command",
                    &script,
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("osascript")
                .args([
                    "-e",
                    &format!("display notification \"{body}\" with title \"{title}\""),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("notify-send")
                .args([&title, &body])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        let _ = (title, body);
    });
}
