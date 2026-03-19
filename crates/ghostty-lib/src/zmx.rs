use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::cmd::run_cmd;
use crate::remote::RemoteConfig;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ZmxSession {
    pub name: String,
    pub pid: Option<String>,
    pub clients: u32,
    pub started_in: Option<String>,
}

impl ZmxSession {
    pub fn is_attached(&self) -> bool {
        self.clients > 0
    }
}

/// Parse zmx list output: tab-separated key=value pairs per line.
/// Note: zmx prefixes the current session with "→ " when run inside a session.
fn parse_zmx_list(output: &str) -> Vec<ZmxSession> {
    output
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with("no sessions"))
        .filter_map(|line| {
            // Strip the "→ " prefix that zmx adds to the current session
            let line = line
                .strip_prefix("→ ")
                .or_else(|| line.strip_prefix("→\t"))
                .unwrap_or(line)
                .trim_start();
            let fields: HashMap<&str, &str> = line
                .split('\t')
                .filter_map(|field| field.split_once('='))
                .collect();

            let name = fields.get("session_name")?.to_string();
            let pid = fields.get("pid").map(|s| s.to_string());
            let clients = fields
                .get("clients")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let started_in = fields.get("started_in").map(|s| s.to_string());

            Some(ZmxSession {
                name,
                pid,
                clients,
                started_in,
            })
        })
        .collect()
}

/// List all zmx sessions.
pub fn list_sessions(remote: Option<&RemoteConfig>) -> Result<Vec<ZmxSession>> {
    let output = run_cmd(&["zmx", "list"], remote).unwrap_or_default();
    Ok(parse_zmx_list(&output))
}

/// Find a zmx session by name.
pub fn find_session(name: &str, remote: Option<&RemoteConfig>) -> Result<Option<ZmxSession>> {
    let sessions = list_sessions(remote)?;
    Ok(sessions.into_iter().find(|s| s.name == name))
}

/// Kill a zmx session.
pub fn kill_session(name: &str, remote: Option<&RemoteConfig>) -> Result<()> {
    run_cmd(&["zmx", "kill", name], remote)
        .with_context(|| format!("failed to kill zmx session {}", name))?;
    Ok(())
}

/// Build the zmx attach command for a worktree session.
/// Passes the command to zmx attach so it creates-and-attaches in one step.
/// Note: if the client disconnects, the session dies because zmx ties the
/// session lifecycle to the initial command. Use ensure_session + exec_attach_only
/// for persistent sessions.
pub fn attach_command(session_name: &str, worktree_path: &str) -> Vec<String> {
    vec![
        "zmx".to_string(),
        "attach".to_string(),
        session_name.to_string(),
        "sh".to_string(),
        "-c".to_string(),
        format!("cd '{}' && exec ${{SHELL:-sh}}", worktree_path),
    ]
}

/// Ensure a zmx session exists by creating it with `zmx run` (daemonized).
/// If the session already exists, this is a no-op.
/// The session persists independently of any client.
/// Uses zmx's default shell (no command arg) so it gets a proper PTY.
pub fn ensure_session(
    session_name: &str,
    worktree_path: &str,
    remote: Option<&RemoteConfig>,
) -> Result<()> {
    // Check if session already exists
    if find_session(session_name, remote)?.is_some() {
        return Ok(());
    }
    // Create session with `zmx run` using default shell.
    // cd to the working directory first via sh -c wrapper.
    let cd_and_shell = format!("cd '{}' && exec ${{SHELL:-sh}}", worktree_path);
    run_cmd(
        &["zmx", "run", session_name, "sh", "-c", &cd_and_shell],
        remote,
    )
    .with_context(|| format!("failed to create zmx session {}", session_name))?;
    Ok(())
}

/// Exec into a zmx session (replaces current process). For zmx-only mode.
/// This combines create + attach: if the session doesn't exist, zmx creates it.
/// The session dies when the client disconnects.
#[cfg(unix)]
pub fn exec_attach(session_name: &str, worktree_path: &str) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let args = attach_command(session_name, worktree_path);
    // Clear ZMX_SESSION so zmx doesn't think we're already in a session.
    // This allows `gmx new` to work from inside an existing zmx session.
    let err = std::process::Command::new(&args[0])
        .args(&args[1..])
        .env_remove("ZMX_SESSION")
        .exec();
    // exec only returns on error
    Err(err).context("failed to exec zmx attach")
}

/// Exec into an existing zmx session (client-only, no command).
/// The session must already exist (created via ensure_session).
/// The session persists after the client disconnects.
#[cfg(unix)]
pub fn exec_attach_only(session_name: &str) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new("zmx")
        .args(["attach", session_name])
        .env_remove("ZMX_SESSION")
        .exec();
    Err(err).context("failed to exec zmx attach")
}

/// Base session name from repo name and branch.
/// Uses dot separator since zmx uses names as socket filenames (no slashes).
pub fn session_name(repo_name: &str, branch: &str) -> String {
    format!("{}.{}", repo_name, branch)
}

/// Find all sessions belonging to a worktree (base + numbered: foo.main, foo.main.2, etc.)
pub fn find_worktree_sessions(
    repo_name: &str,
    branch: &str,
    remote: Option<&RemoteConfig>,
) -> Result<Vec<ZmxSession>> {
    let base = session_name(repo_name, branch);
    let sessions = list_sessions(remote)?;
    let mut matches: Vec<ZmxSession> = sessions
        .into_iter()
        .filter(|s| s.name == base || s.name.starts_with(&format!("{}.", base)))
        .collect();
    // Sort so base comes first, then .2, .3, etc.
    matches.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(matches)
}

/// Find all sessions matching a prefix (for gmx session groups).
/// Sorted by numeric suffix: name.1, name.2, name.10 (not alphabetical).
pub fn find_sessions_by_prefix(
    prefix: &str,
    remote: Option<&RemoteConfig>,
) -> Result<Vec<ZmxSession>> {
    let sessions = list_sessions(remote)?;
    let prefix_dot = format!("{}.", prefix);
    let mut matches: Vec<ZmxSession> = sessions
        .into_iter()
        .filter(|s| s.name == prefix || s.name.starts_with(&prefix_dot))
        .collect();
    // Sort by numeric suffix so .1 < .2 < .10
    matches.sort_by(|a, b| {
        let a_num = a
            .name
            .strip_prefix(&prefix_dot)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let b_num = b
            .name
            .strip_prefix(&prefix_dot)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        a_num.cmp(&b_num)
    });
    Ok(matches)
}

/// Generate the next available session name for a new split.
/// Returns ghostree.main, ghostree.main.2, ghostree.main.3, etc.
pub fn next_session_name(
    repo_name: &str,
    branch: &str,
    remote: Option<&RemoteConfig>,
) -> Result<String> {
    let base = session_name(repo_name, branch);
    next_session_name_from_base(&base, remote)
}

/// Generate the next available session name from a base prefix.
/// Always uses numbered suffixes (.2, .3, etc.) since .1 is the first pane.
pub fn next_session_name_from_base(base: &str, remote: Option<&RemoteConfig>) -> Result<String> {
    let sessions = list_sessions(remote)?;

    // Find the next available number (starting from 2 since .1 is the first pane)
    for i in 2.. {
        let candidate = format!("{}.{}", base, i);
        if !sessions.iter().any(|s| s.name == candidate) {
            return Ok(candidate);
        }
    }

    unreachable!()
}

/// Extract repo name from a path.
pub fn repo_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_zmx_list_basic() {
        let output = "session_name=foo.main\tpid=1234\tclients=1\tstarted_in=/tmp\n\
                       session_name=bar.dev\tpid=5678\tclients=0\tstarted_in=/home\n";
        let sessions = parse_zmx_list(output);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "foo.main");
        assert_eq!(sessions[0].pid, Some("1234".to_string()));
        assert_eq!(sessions[0].clients, 1);
        assert!(sessions[0].is_attached());
        assert_eq!(sessions[1].name, "bar.dev");
        assert_eq!(sessions[1].clients, 0);
        assert!(!sessions[1].is_attached());
    }

    #[test]
    fn test_parse_zmx_list_empty() {
        assert!(parse_zmx_list("").is_empty());
        assert!(parse_zmx_list("no sessions\n").is_empty());
    }

    #[test]
    fn test_parse_zmx_list_arrow_prefix() {
        // zmx prefixes the current session with "→ " when run inside a session
        let output = "  session_name=other\tpid=111\tclients=0\n\
                       → session_name=current.1\tpid=222\tclients=1\tstarted_in=/tmp\n";
        let sessions = parse_zmx_list(output);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "other");
        assert_eq!(sessions[1].name, "current.1");
        assert_eq!(sessions[1].clients, 1);
        assert!(sessions[1].is_attached());
    }

    #[test]
    fn test_parse_zmx_list_missing_fields() {
        let output = "session_name=minimal\n";
        let sessions = parse_zmx_list(output);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "minimal");
        assert_eq!(sessions[0].pid, None);
        assert_eq!(sessions[0].clients, 0);
        assert_eq!(sessions[0].started_in, None);
    }

    #[test]
    fn test_parse_zmx_list_no_session_name() {
        // Lines without session_name should be skipped
        let output = "pid=1234\tclients=1\n";
        let sessions = parse_zmx_list(output);
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_session_name() {
        assert_eq!(session_name("myrepo", "main"), "myrepo.main");
        assert_eq!(session_name("foo", "feature-branch"), "foo.feature-branch");
    }

    #[test]
    fn test_repo_name_from_path() {
        assert_eq!(repo_name_from_path("/home/user/Code/myrepo"), "myrepo");
        assert_eq!(repo_name_from_path("~/Code/foo"), "foo");
        assert_eq!(repo_name_from_path("/home/user/Code/myrepo/"), "myrepo");
    }

    #[test]
    fn test_attach_command() {
        let cmd = attach_command("foo.main", "/home/user/Code/foo");
        assert_eq!(cmd[0], "zmx");
        assert_eq!(cmd[1], "attach");
        assert_eq!(cmd[2], "foo.main");
        assert!(cmd[5].contains("/home/user/Code/foo"));
    }

    #[test]
    fn test_is_attached() {
        let attached = ZmxSession {
            name: "test".to_string(),
            pid: None,
            clients: 2,
            started_in: None,
        };
        assert!(attached.is_attached());

        let detached = ZmxSession {
            name: "test".to_string(),
            pid: None,
            clients: 0,
            started_in: None,
        };
        assert!(!detached.is_attached());
    }
}
