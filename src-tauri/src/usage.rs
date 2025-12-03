use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use regex::Regex;
use serde::Serialize;
use std::io::Read;
use std::time::{Duration, Instant};

/// Parsed usage data from Claude Code /usage command
#[derive(Debug, Clone, Serialize)]
pub struct UsageData {
    pub current_session_percent: Option<f32>,
    pub current_session_reset: Option<String>,
    pub current_week_all_models_percent: Option<f32>,
    pub current_week_all_models_reset: Option<String>,
    pub current_week_sonnet_percent: Option<f32>,
    pub current_week_sonnet_reset: Option<String>,
    pub extra_usage_enabled: bool,
}

impl UsageData {
    fn new() -> Self {
        Self {
            current_session_percent: None,
            current_session_reset: None,
            current_week_all_models_percent: None,
            current_week_all_models_reset: None,
            current_week_sonnet_percent: None,
            current_week_sonnet_reset: None,
            extra_usage_enabled: false,
        }
    }
}

fn parse_usage_output(raw_output: &str) -> Result<UsageData> {
    // Strip ANSI escape codes
    let stripped = strip_ansi_escapes::strip(raw_output);
    let output = String::from_utf8_lossy(&stripped);

    let mut data = UsageData::new();

    // Parse percentage patterns like "3% used" or "26% used"
    let percent_re = Regex::new(r"(\d+)%\s+used")?;

    // Parse reset time patterns like "Resets 7pm" or "Resets Dec 8 at 4pm"
    let reset_re = Regex::new(r"Resets\s+([^\n]+)")?;

    // Split by sections - look for "Current session", "Current week (all models)", etc.
    let lines: Vec<&str> = output.lines().collect();

    let mut current_section = "";

    for line in lines.iter() {
        let line_lower = line.to_lowercase();

        // Detect sections
        if line_lower.contains("current session") {
            current_section = "session";
        } else if line_lower.contains("current week") && line_lower.contains("all models") {
            current_section = "week_all";
        } else if line_lower.contains("current week") && line_lower.contains("sonnet") {
            current_section = "week_sonnet";
        } else if line_lower.contains("extra usage") {
            current_section = "extra";
            if line_lower.contains("not enabled") {
                data.extra_usage_enabled = false;
            } else if line_lower.contains("enabled") {
                data.extra_usage_enabled = true;
            }
        }

        // Extract percentages
        if let Some(caps) = percent_re.captures(line) {
            if let Some(pct) = caps.get(1) {
                let percent: f32 = pct.as_str().parse().unwrap_or(0.0);
                match current_section {
                    "session" => data.current_session_percent = Some(percent),
                    "week_all" => data.current_week_all_models_percent = Some(percent),
                    "week_sonnet" => data.current_week_sonnet_percent = Some(percent),
                    _ => {}
                }
            }
        }

        // Extract reset times
        if let Some(caps) = reset_re.captures(line) {
            if let Some(reset) = caps.get(1) {
                let reset_str = reset.as_str().trim().to_string();
                match current_section {
                    "session" => data.current_session_reset = Some(reset_str),
                    "week_all" => data.current_week_all_models_reset = Some(reset_str),
                    "week_sonnet" => data.current_week_sonnet_reset = Some(reset_str),
                    _ => {}
                }
            }
        }
    }

    Ok(data)
}

fn run_claude_usage() -> Result<String> {
    let pty_system = NativePtySystem::default();

    // Create a PTY with a reasonable size
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("Failed to open PTY")?;

    // Build the command - look for claude in PATH or use CLAUDE_PATH env var
    let claude_path = std::env::var("CLAUDE_PATH").unwrap_or_else(|_| "claude".to_string());

    let mut cmd = CommandBuilder::new(&claude_path);
    cmd.arg("/usage");

    // Spawn the process
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("Failed to spawn claude")?;

    // Drop the slave to avoid blocking
    drop(pair.slave);

    // Read from the master
    let mut reader = pair.master.try_clone_reader()?;
    let mut output = String::new();
    let mut buffer = [0u8; 4096];

    let start = Instant::now();
    let timeout = Duration::from_secs(30);
    let mut saw_loading = false;
    let mut current_screen = String::new();

    loop {
        // Check timeout
        if start.elapsed() > timeout {
            break;
        }

        // Try to read
        match reader.read(&mut buffer) {
            Ok(0) => {
                break;
            }
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buffer[..n]);
                output.push_str(&chunk);
                current_screen.push_str(&chunk);

                // Detect screen clear sequence [2J (clear screen)
                if chunk.contains("\x1b[2J") || chunk.contains("[2J") {
                    current_screen.clear();
                    current_screen.push_str(&chunk);
                }

                // Strip ANSI from current screen only
                let stripped = strip_ansi_escapes::strip(&current_screen);
                let clean_screen = String::from_utf8_lossy(&stripped);

                // Check current screen state
                let has_loading = clean_screen.contains("Loading usage data");
                let has_percent = clean_screen.contains("% used");
                let has_current_session = clean_screen.contains("Current session");
                let has_extra_usage = clean_screen.contains("Extra usage");

                if has_loading {
                    saw_loading = true;
                }

                // Success: Current screen has usage data without loading indicator
                if saw_loading
                    && has_percent
                    && has_current_session
                    && has_extra_usage
                    && !has_loading
                {
                    output = current_screen;
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                break;
            }
        }

        // Check if process exited
        if let Ok(Some(_status)) = child.try_wait() {
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buffer[..n]);
                        output.push_str(&chunk);
                    }
                    Err(_) => break,
                }
            }
            break;
        }
    }

    // Kill the process if still running
    let _ = child.kill();

    Ok(output)
}

/// Fetch usage data from Claude Code
pub fn fetch_usage() -> Result<UsageData> {
    let raw_output = run_claude_usage()?;
    parse_usage_output(&raw_output)
}
