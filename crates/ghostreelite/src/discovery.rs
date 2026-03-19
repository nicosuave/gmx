use std::collections::HashSet;
use std::fs;

use anyhow::Result;

use crate::config::{Config, RepoConfig};
use ghostty_lib::cmd::run_cmd;
use ghostty_lib::remote::RemoteConfig;

#[derive(Debug, Clone)]
pub struct DiscoveredRepo {
    pub config: RepoConfig,
    /// "local" or remote host name
    pub env_name: String,
    pub remote: Option<RemoteConfig>,
}

/// Discover all repos: explicit repos + local scan_dirs + remote scan_dirs.
pub fn discover_repos(config: &Config) -> Result<Vec<DiscoveredRepo>> {
    let mut repos = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    // 1. Explicit repos from config.repos
    for repo_cfg in &config.repos {
        let (env_name, remote) = match &repo_cfg.remote {
            Some(r) => (r.host.clone(), Some(r.clone())),
            None => ("local".to_string(), None),
        };
        let key = (repo_cfg.path.clone(), env_name.clone());
        if seen.insert(key) {
            repos.push(DiscoveredRepo {
                config: repo_cfg.clone(),
                env_name,
                remote,
            });
        }
    }

    // 2. Local scan_dirs
    for dir in &config.scan_dirs {
        let expanded = expand_tilde(dir);
        let discovered = scan_local_dir(&expanded);
        for path in discovered {
            let key = (path.clone(), "local".to_string());
            if seen.insert(key) {
                repos.push(DiscoveredRepo {
                    config: RepoConfig { path, remote: None },
                    env_name: "local".to_string(),
                    remote: None,
                });
            }
        }
    }

    // 3. Remote scan_dirs (from config.remote.scan_dirs)
    if let Some(remote) = &config.remote {
        for dir in &remote.scan_dirs {
            let discovered = scan_remote_dir(dir, remote)?;
            let env_name = remote.host.clone();
            for path in discovered {
                let key = (path.clone(), env_name.clone());
                if seen.insert(key) {
                    repos.push(DiscoveredRepo {
                        config: RepoConfig {
                            path,
                            remote: Some(remote.clone()),
                        },
                        env_name: env_name.clone(),
                        remote: Some(remote.clone()),
                    });
                }
            }
        }
    }

    Ok(repos)
}

/// Scan a local directory for git repos with worktrees.
/// Only returns repos that have .git/worktrees directory (actual worktree usage).
fn scan_local_dir(dir: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut repos = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let git_path = path.join(".git");
        // Must be a real git repo (.git is directory, not worktree pointer file)
        // AND must actually have worktrees (.git/worktrees exists)
        if git_path.is_dir() && git_path.join("worktrees").is_dir() {
            repos.push(path.to_string_lossy().to_string());
        }
    }
    repos
}

/// Scan a remote directory for git repos with worktrees via SSH.
/// Only returns repos that have .git/worktrees directory (actual worktree usage),
/// which is dramatically fewer than all git repos.
fn scan_remote_dir(dir: &str, remote: &RemoteConfig) -> Result<Vec<String>> {
    let script = format!(
        "for d in {}/*/; do [ -d \"$d/.git/worktrees\" ] && echo \"$d\"; done 2>/dev/null; true",
        dir
    );
    let output = run_cmd(&["sh", "-c", &script], Some(remote))?;
    Ok(output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_end_matches('/').to_string())
        .collect())
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/")
        && let Some(home) = dirs::home_dir()
    {
        return format!("{}{}", home.display(), &path[1..]);
    }
    path.to_string()
}
