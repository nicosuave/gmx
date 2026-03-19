use anyhow::{Context, Result, bail};
use std::process::Command;

use crate::remote::RemoteConfig;

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

// --- gmx-style functions using surface configuration ---
//
// Note: Ghostty wraps `command` through /usr/bin/login which breaks shell commands.
// So we use `input text` + `send key "enter"` for the command, and surface config
// only for working directory and environment variables.

/// Create a new Ghostty tab with explicit surface configuration.
/// Uses env vars and working dir from surface config, but runs the command
/// via `input text` + `send key` to avoid Ghostty's login shell wrapping.
/// Returns the terminal ID.
pub fn create_tab_with_config(
    command: &str,
    working_dir: Option<&str>,
    env_vars: &[(&str, &str)],
) -> Result<String> {
    let mut config_lines = Vec::new();
    if let Some(wd) = working_dir {
        config_lines.push(format!(
            "    set initial working directory of cfg to \"{}\"",
            applescript_escape(wd)
        ));
    }
    if !env_vars.is_empty() {
        let env_list: Vec<String> = env_vars
            .iter()
            .map(|(k, v)| format!("\"{}={}\"", applescript_escape(k), applescript_escape(v)))
            .collect();
        config_lines.push(format!(
            "    set environment variables of cfg to {{{}}}",
            env_list.join(", ")
        ));
    }

    let config_block = if config_lines.is_empty() {
        String::new()
    } else {
        config_lines.join("\n")
    };

    let script = format!(
        r#"tell application "Ghostty"
    set cfg to new surface configuration
{config}
    set newTab to new tab in front window with configuration cfg
    set newTerm to focused terminal of newTab
    delay 0.5
    input text "{input}" to newTerm
    send key "enter" to newTerm
    return id of newTerm
end tell"#,
        config = config_block,
        input = applescript_escape(command),
    );
    run_applescript_output(&script).context("failed to create Ghostty tab with config")
}

/// Split an existing terminal with explicit surface configuration.
/// Uses env vars and working dir from surface config, but runs the command
/// via `input text` + `send key` to avoid Ghostty's login shell wrapping.
/// Returns the new terminal ID.
pub fn split_with_config(
    terminal_id: &str,
    direction: &str,
    command: &str,
    working_dir: Option<&str>,
    env_vars: &[(&str, &str)],
) -> Result<String> {
    let mut config_lines = Vec::new();
    if let Some(wd) = working_dir {
        config_lines.push(format!(
            "    set initial working directory of cfg to \"{}\"",
            applescript_escape(wd)
        ));
    }
    if !env_vars.is_empty() {
        let env_list: Vec<String> = env_vars
            .iter()
            .map(|(k, v)| format!("\"{}={}\"", applescript_escape(k), applescript_escape(v)))
            .collect();
        config_lines.push(format!(
            "    set environment variables of cfg to {{{}}}",
            env_list.join(", ")
        ));
    }

    let config_block = if config_lines.is_empty() {
        String::new()
    } else {
        config_lines.join("\n")
    };

    let script = format!(
        r#"tell application "Ghostty"
    set t to first terminal whose id is "{tid}"
    focus t
    set cfg to new surface configuration
{config}
    set newTerm to split t direction {dir} with configuration cfg
    delay 0.5
    input text "{input}" to newTerm
    send key "enter" to newTerm
    return id of newTerm
end tell"#,
        tid = applescript_escape(terminal_id),
        dir = direction,
        config = config_block,
        input = applescript_escape(command),
    );
    run_applescript_output(&script).context("failed to split terminal with config")
}

/// Get the focused terminal ID in the front window.
pub fn focused_terminal_id() -> Result<String> {
    let script = r#"tell application "Ghostty"
    set t to focused terminal of selected tab of front window
    return id of t
end tell"#;
    run_applescript_output(script).context("failed to get focused terminal ID")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_applescript_escape() {
        assert_eq!(applescript_escape("hello"), "hello");
        assert_eq!(applescript_escape(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(applescript_escape(r"path\to\file"), r"path\\to\\file");
        assert_eq!(applescript_escape(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    #[test]
    fn test_shell_escape_single() {
        assert_eq!(shell_escape_single("hello"), "hello");
        assert_eq!(shell_escape_single("it's"), "it'\\''s");
        assert_eq!(shell_escape_single("a'b'c"), "a'\\''b'\\''c");
    }

    #[test]
    fn test_build_interactive_command_local() {
        let cmd = build_interactive_command("myrepo.main", "/home/user/Code/myrepo", None);
        assert!(cmd.contains("cd /home/user/Code/myrepo"));
        assert!(cmd.contains("zmx attach myrepo.main"));
    }

    #[test]
    fn test_build_interactive_command_remote_ssh() {
        let remote = RemoteConfig {
            host: "myhost".to_string(),
            user: Some("user".to_string()),
            scan_dirs: vec![],
            transport: "ssh".to_string(),
        };
        let cmd = build_interactive_command("myrepo.main", "/home/user/Code/myrepo", Some(&remote));
        assert!(cmd.starts_with("ssh user@myhost -t"));
        assert!(cmd.contains("zmx attach myrepo.main"));
    }

    #[test]
    fn test_build_interactive_command_remote_mosh() {
        let remote = RemoteConfig {
            host: "myhost".to_string(),
            user: None,
            scan_dirs: vec![],
            transport: "mosh".to_string(),
        };
        let cmd = build_interactive_command("myrepo.main", "/home/user/Code/myrepo", Some(&remote));
        assert!(cmd.starts_with("mosh myhost"));
        assert!(cmd.contains("zmx attach myrepo.main"));
    }

    #[test]
    fn test_build_tab_script_contains_applescript_structure() {
        let script = build_tab_script("test.main", "/tmp/test", None);
        assert!(script.contains("tell application \"Ghostty\""));
        assert!(script.contains("new surface configuration"));
        assert!(script.contains("initial working directory"));
        assert!(script.contains("/tmp/test"));
        assert!(script.contains("zmx attach test.main"));
        assert!(script.contains("new tab in front window"));
        assert!(script.contains("return id of newTerm"));
    }

    #[test]
    fn test_build_tab_script_remote_no_working_dir() {
        let remote = RemoteConfig {
            host: "myhost".to_string(),
            user: None,
            scan_dirs: vec![],
            transport: "ssh".to_string(),
        };
        let script = build_tab_script("test.main", "/remote/path", Some(&remote));
        // Remote tabs don't set initial working directory (SSH handles it)
        assert!(!script.contains("initial working directory"));
        assert!(script.contains("ssh myhost"));
    }

    #[test]
    fn test_build_split_script_direction() {
        let script = build_split_script("test.main", "/tmp/test", "right", None);
        assert!(script.contains("direction right"));
        assert!(script.contains("split t direction right"));

        let script = build_split_script("test.main", "/tmp/test", "down", None);
        assert!(script.contains("direction down"));
    }

    #[test]
    fn test_build_tab_with_config_basic() {
        // Test the AppleScript generation for create_tab_with_config
        // We can't run AppleScript in tests, but we can test the script building
        // by extracting the logic into a testable function.
        // For now, verify the escape functions work correctly for config building.
        let env_vars: Vec<(&str, &str)> = vec![("GMX_SESSION", "my-project"), ("GMX_IDX", "1")];
        let env_list: Vec<String> = env_vars
            .iter()
            .map(|(k, v)| format!("\"{}={}\"", applescript_escape(k), applescript_escape(v)))
            .collect();
        let result = format!("{{{}}}", env_list.join(", "));
        assert_eq!(result, "{\"GMX_SESSION=my-project\", \"GMX_IDX=1\"}");
    }

    #[test]
    fn test_is_available_env_check() {
        // In test environment, TERM_PROGRAM is typically not "ghostty"
        // This just verifies the function doesn't panic
        let _ = is_available();
    }
}
