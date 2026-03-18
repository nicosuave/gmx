use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::cmd::run_cmd;
use crate::config::RemoteConfig;

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
fn parse_zmx_list(output: &str) -> Vec<ZmxSession> {
    output
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with("no sessions"))
        .filter_map(|line| {
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

/// Exec into a zmx session (replaces current process). For zmx-only mode.
#[cfg(unix)]
pub fn exec_attach(session_name: &str, worktree_path: &str) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let args = attach_command(session_name, worktree_path);
    let err = std::process::Command::new(&args[0])
        .args(&args[1..])
        .exec();
    // exec only returns on error
    Err(err).context("failed to exec zmx attach")
}

/// Base session name from repo name and branch.
/// Uses dot separator since zmx uses names as socket filenames (no slashes).
pub fn session_name(repo_name: &str, branch: &str) -> String {
    format!("{}.{}", repo_name, branch)
}

/// Find all sessions belonging to a worktree (base + numbered: foo.main, foo.main.2, etc.)
pub fn find_worktree_sessions(repo_name: &str, branch: &str, remote: Option<&crate::config::RemoteConfig>) -> Result<Vec<ZmxSession>> {
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

/// Generate the next available session name for a new split.
/// Returns ghostree.main, ghostree.main.2, ghostree.main.3, etc.
pub fn next_session_name(repo_name: &str, branch: &str, remote: Option<&crate::config::RemoteConfig>) -> Result<String> {
    let base = session_name(repo_name, branch);
    let sessions = list_sessions(remote)?;

    // If the base name isn't taken, use it
    if !sessions.iter().any(|s| s.name == base) {
        return Ok(base);
    }

    // Find the next available number
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
