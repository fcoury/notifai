use anyhow::{bail, Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use regex::Regex;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use strip_ansi_escapes;

use crate::usage::UsageData;

/// Run `codex` and issue `/status`, capturing the output.
fn run_codex_status(codex_path: &str) -> Result<String> {
    let pty_system = NativePtySystem::default();

    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("Failed to open PTY")?;

    // Locate binary (explicit path parameter, with env fallback)
    let cli_path = std::env::var("CODEX_PATH").unwrap_or_else(|_| codex_path.to_string());
    eprintln!("[NotifAI] Codex: using binary path {}", cli_path);
    let mut cmd = CommandBuilder::new(&cli_path);
    cmd.arg("--yolo"); // skip Codex approval prompt per known issue
    cmd.env("TERM", "xterm-256color");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("Failed to spawn codex")?;

    // Drop slave side
    drop(pair.slave);

    // Keep writer to issue commands once the prompt is ready
    let mut writer = pair.master.take_writer()?;

    // Read output: use blocking reader in a dedicated thread, consume via channel with timeout
    let reader = pair.master.try_clone_reader()?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = tx.send(None);
                    break;
                }
                Ok(n) => {
                    let _ = tx.send(Some(buffer[..n].to_vec()));
                }
                Err(e) => {
                    eprintln!("[NotifAI] Codex read error thread: {}", e);
                    let _ = tx.send(None);
                    break;
                }
            }
        }
    });

    let mut output = String::new();

    let start = Instant::now();
    let timeout = Duration::from_secs(45);
    let mut current_screen = String::new();
    let mut resent_command = false;
    let mut sent_status = false;

    loop {
        if start.elapsed() > timeout {
            eprintln!(
                "[NotifAI] Codex status: timeout after {:?}",
                start.elapsed()
            );
            break;
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(bytes)) => {
                let chunk = String::from_utf8_lossy(&bytes);
                output.push_str(&chunk);
                current_screen.push_str(&chunk);

                // Respond to terminal capability queries
                if chunk.contains("\u{1b}[6n") {
                    let _ = writer.write_all(b"\x1b[1;1R");
                    writer.flush().ok();
                    eprintln!("[NotifAI] Codex: replied to cursor position query");
                }
                if chunk.contains("\u{1b}[c") {
                    // Primary DA response (xterm-ish)
                    let _ = writer.write_all(b"\x1b[?1;0c");
                    writer.flush().ok();
                    eprintln!("[NotifAI] Codex: replied to device attributes query");
                }

                // Strip ANSI for detection
                let stripped = strip_ansi_escapes::strip(&current_screen);
                let clean = String::from_utf8_lossy(&stripped);

                // Detect ready prompt then send /status once
                if !sent_status
                    && (clean.contains("context left")
                        || clean.contains("Tip: Start a fresh idea")
                        || clean.contains("Tip: You can run any shell commands")
                        || clean.contains("Tip: Paste an image")
                        || clean.contains("Tip: Type / to open the command popup"))
                {
                    let _ = writer.write_all(b"\r/status\r");
                    writer.flush().ok();
                    sent_status = true;
                    eprintln!("[NotifAI] Codex: prompt ready, sent /status");
                }

                // Handle approval/pause prompts
                if clean.to_lowercase().contains("press enter to continue") {
                    let _ = writer.write_all(b"\n");
                    writer.flush().ok();
                    eprintln!("[NotifAI] Codex: auto-continued past approval prompt");
                }

                let has_five = clean.contains("5h limit");
                let has_week = clean.contains("Weekly limit");
                let has_left = clean.contains("% left");

                if has_five && has_week && has_left {
                    // Good enough snapshot
                    output = clean.to_string();
                    break;
                }
            }
            Ok(None) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => { /* continue loop for timeout/resend checks */
            }
            Err(e) => {
                eprintln!("[NotifAI] Codex channel error: {}", e);
                break;
            }
        }

        // If we haven't seen any data after a few seconds post-command, try resending
        if sent_status && !resent_command && start.elapsed() > Duration::from_secs(10) {
            let _ = writer.write_all(b"\r/status\r");
            writer.flush().ok();
            resent_command = true;
            eprintln!("[NotifAI] Codex: re-sent /status command after 10s");
        }

        // Process exited?
        if let Ok(Some(_status)) = child.try_wait() {
            break;
        }
    }

    let _ = child.kill();

    Ok(output)
}

/// Parse Codex /status output into UsageData codex fields.
fn parse_codex_output(raw_output: &str) -> Result<UsageData> {
    if raw_output
        .to_lowercase()
        .contains("cursor position could not be read")
    {
        bail!("Codex CLI failed to read cursor position (terminal emulation)");
    }

    let mut data = UsageData::new();

    // Example lines:
    // 5h limit:         [████████████████████] 99% left (resets 13:35)
    // Weekly limit:     [████████████████░░░░] 80% left (resets 13:17)
    let line_re =
        Regex::new(r"(?i)(5h limit|weekly limit):.*?(\d+)%\s+left\s*\(resets\s+([^\)]+)\)")?;

    let mut seen_any = false;
    for caps in line_re.captures_iter(raw_output) {
        let label = caps
            .get(1)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();
        let left_pct: f32 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0.0);
        let reset = caps.get(3).map(|m| m.as_str().trim().to_string());

        match label.as_str() {
            "5h limit" => {
                data.codex_five_hour_left = Some(left_pct);
                data.codex_five_hour_reset = reset;
            }
            "weekly limit" => {
                data.codex_week_left = Some(left_pct);
                data.codex_week_reset = reset;
            }
            _ => {}
        }
        seen_any = true;
    }

    if seen_any {
        eprintln!(
            "[NotifAI] Codex parsed: five_hour_left={:?} reset={:?}, week_left={:?} reset={:?}",
            data.codex_five_hour_left,
            data.codex_five_hour_reset,
            data.codex_week_left,
            data.codex_week_reset
        );
    } else {
        eprintln!(
            "[NotifAI] Codex parse found no matches. Raw (first 300 chars): {:?}",
            raw_output.chars().take(300).collect::<String>()
        );
    }

    Ok(data)
}

/// Fetch Codex usage limits.
pub fn fetch_codex_usage(codex_path: &str) -> Result<UsageData> {
    let raw = run_codex_status(codex_path)?;
    parse_codex_output(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_status_lines() {
        let sample = "5h limit:         [████████████████████] 99% left (resets 13:35)\nWeekly limit:     [████████████████░░░░] 80% left (resets 13:17)\n";
        let data = parse_codex_output(sample).unwrap();
        assert_eq!(data.codex_five_hour_left, Some(99.0));
        assert_eq!(data.codex_five_hour_reset.as_deref(), Some("13:35"));
        assert_eq!(data.codex_week_left, Some(80.0));
        assert_eq!(data.codex_week_reset.as_deref(), Some("13:17"));
    }
}
