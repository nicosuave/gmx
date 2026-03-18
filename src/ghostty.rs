use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::config::RemoteConfig;

/// Check if we're running inside Ghostty on macOS.
pub fn is_available() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    std::env::var("TERM_PROGRAM")
        .map(|v| v == "ghostty")
        .unwrap_or(false)
}

/// Build the shell command string for an interactive session.
/// Used by all script builders to avoid duplication.
fn build_interactive_command(
    session_name: &str,
    worktree_path: &str,
    remote: Option<&RemoteConfig>,
) -> String {
    match remote {
        Some(r) => {
            let target = r.ssh_target();
            let inner = format!(
                "cd {} && zmx attach {}",
                shell_escape_single(worktree_path),
                shell_escape_single(session_name),
            );
            if r.use_mosh() {
                format!("mosh {} -- sh -c '{}'", target, inner)
            } else {
                format!("ssh {} -t '{}'", target, inner)
            }
        }
        None => {
            format!(
                "cd {} && zmx attach {}",
                shell_escape_single(worktree_path),
                shell_escape_single(session_name),
            )
        }
    }
}

/// Open a worktree in Ghostty via AppleScript.
/// If split_direction is Some, creates a split. Otherwise creates a new tab.
/// Returns the terminal ID of the new terminal.
pub fn open(
    session_name: &str,
    worktree_path: &str,
    split_direction: Option<&str>,
    remote: Option<&RemoteConfig>,
) -> Result<String> {
    let script = match split_direction {
        Some(dir) => build_split_script(session_name, worktree_path, dir, remote),
        None => build_tab_script(session_name, worktree_path, remote),
    };
    run_applescript_output(&script).context("failed to create Ghostty tab/split")
}

/// Build AppleScript to create a split with the right command.
fn build_split_script(
    session_name: &str,
    worktree_path: &str,
    direction: &str,
    remote: Option<&RemoteConfig>,
) -> String {
    let command = build_interactive_command(session_name, worktree_path, remote);

    match remote {
        Some(_) => {
            format!(
                r#"tell application "Ghostty"
    set t to focused terminal of selected tab of front window
    set newTerm to split t direction {dir}
    delay 0.5
    input text "{input}" to newTerm
    send key "enter" to newTerm
    return id of newTerm
end tell"#,
                input = applescript_escape(&command),
                dir = direction,
            )
        }
        None => {
            format!(
                r#"tell application "Ghostty"
    set t to focused terminal of selected tab of front window
    set cfg to new surface configuration
    set initial working directory of cfg to "{wd}"
    set newTerm to split t direction {dir} with configuration cfg
    delay 0.5
    input text "{input}" to newTerm
    send key "enter" to newTerm
    return id of newTerm
end tell"#,
                wd = applescript_escape(worktree_path),
                input = applescript_escape(&format!(
                    "zmx attach {}",
                    shell_escape_single(session_name),
                )),
                dir = direction,
            )
        }
    }
}

/// Build AppleScript to create a new tab.
fn build_tab_script(
    session_name: &str,
    worktree_path: &str,
    remote: Option<&RemoteConfig>,
) -> String {
    let command = build_interactive_command(session_name, worktree_path, remote);

    match remote {
        Some(_) => {
            format!(
                r#"tell application "Ghostty"
    set newTab to new tab in front window
    set newTerm to focused terminal of newTab
    delay 0.5
    input text "{input}" to newTerm
    send key "enter" to newTerm
    return id of newTerm
end tell"#,
                input = applescript_escape(&command),
            )
        }
        None => {
            format!(
                r#"tell application "Ghostty"
    set cfg to new surface configuration
    set initial working directory of cfg to "{wd}"
    set newTab to new tab in front window with configuration cfg
    set newTerm to focused terminal of newTab
    delay 0.5
    input text "{input}" to newTerm
    send key "enter" to newTerm
    return id of newTerm
end tell"#,
                wd = applescript_escape(worktree_path),
                input = applescript_escape(&format!(
                    "zmx attach {}",
                    shell_escape_single(session_name),
                )),
            )
        }
    }
}

/// Split an existing terminal by ID. Returns the new terminal's ID.
pub fn split_at(
    terminal_id: &str,
    session_name: &str,
    worktree_path: &str,
    direction: &str,
    remote: Option<&RemoteConfig>,
) -> Result<String> {
    let command = build_interactive_command(session_name, worktree_path, remote);
    let script = format!(
        r#"tell application "Ghostty"
    set t to first terminal whose id is "{tid}"
    focus t
    set newTerm to split t direction {dir}
    delay 0.5
    input text "{input}" to newTerm
    send key "enter" to newTerm
    return id of newTerm
end tell"#,
        tid = applescript_escape(terminal_id),
        dir = direction,
        input = applescript_escape(&command),
    );
    run_applescript_output(&script).context("failed to split terminal")
}

/// Check if a terminal with the given ID still exists.
pub fn terminal_exists(terminal_id: &str) -> bool {
    let script = format!(
        r#"tell application "Ghostty"
    try
        set t to first terminal whose id is "{tid}"
        return "ok"
    on error
        return "gone"
    end try
end tell"#,
        tid = applescript_escape(terminal_id),
    );
    match run_applescript_output(&script) {
        Ok(output) => output == "ok",
        Err(_) => false,
    }
}

/// Focus an existing terminal by ID (brings its tab to front).
pub fn focus_terminal(terminal_id: &str) -> Result<()> {
    let script = format!(
        r#"tell application "Ghostty"
    set t to first terminal whose id is "{tid}"
    focus t
end tell"#,
        tid = applescript_escape(terminal_id),
    );
    run_applescript(&script).context("failed to focus terminal")
}

/// Build AppleScript to equalize splits.
#[allow(dead_code)]
pub fn equalize_splits() -> Result<()> {
    let script = r#"tell application "Ghostty"
    set t to focused terminal of selected tab of front window
    perform action "equalize_splits" on t
end tell"#;
    run_applescript(script).context("failed to equalize splits")
}

/// Run an AppleScript string via osascript, discarding stdout.
fn run_applescript(script: &str) -> Result<()> {
    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .context("failed to run osascript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("osascript failed: {}", stderr.trim());
    }

    Ok(())
}

/// Run an AppleScript string via osascript, returning stdout as a trimmed String.
fn run_applescript_output(script: &str) -> Result<String> {
    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .context("failed to run osascript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("osascript failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout)
}

/// Escape a string for AppleScript double-quoted string.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape for single-quoted shell strings.
fn shell_escape_single(s: &str) -> String {
    s.replace('\'', "'\\''")
}
