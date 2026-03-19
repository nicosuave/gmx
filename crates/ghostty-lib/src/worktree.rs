use anyhow::{Context, Result};
use serde::Deserialize;

use crate::cmd::run_cmd;
use crate::remote::RemoteConfig;

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
pub struct WorktreeCommit {
    #[serde(default)]
    pub short_sha: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorktreeWorkingTree {
    pub staged: bool,
    pub modified: bool,
    pub untracked: bool,
    pub diff: WorktreeDiff,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorktreeDiff {
    pub added: u32,
    pub deleted: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorktreeRemote {
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Worktree {
    pub branch: String,
    pub path: String,
    pub is_main: bool,
    #[serde(default)]
    pub is_current: bool,
    #[serde(default)]
    pub commit: Option<WorktreeCommit>,
    #[serde(default)]
    pub working_tree: Option<WorktreeWorkingTree>,
    #[serde(default)]
    pub remote: Option<WorktreeRemote>,
}

/// List worktrees for a repo using `wt list --format=json`.
pub fn list_worktrees(repo_path: &str, remote: Option<&RemoteConfig>) -> Result<Vec<Worktree>> {
    let output = run_cmd(&["wt", "-C", repo_path, "list", "--format=json"], remote)
        .with_context(|| format!("failed to list worktrees for {}", repo_path))?;

    let worktrees: Vec<Worktree> = serde_json::from_str(&output)
        .with_context(|| format!("failed to parse wt output for {}", repo_path))?;

    Ok(worktrees)
}

/// Create a new worktree via `wt switch --yes --create`.
pub fn create_worktree(
    repo_path: &str,
    branch: &str,
    base: Option<&str>,
    remote: Option<&RemoteConfig>,
) -> Result<()> {
    let mut args = vec!["wt", "-C", repo_path, "switch", "--yes", "--create", branch];
    if let Some(b) = base {
        args.push("--base");
        args.push(b);
    }

    run_cmd(&args, remote).with_context(|| format!("failed to create worktree {}", branch))?;

    Ok(())
}

/// Remove a worktree via `wt remove`.
pub fn remove_worktree(
    repo_path: &str,
    branch: &str,
    force: bool,
    remote: Option<&RemoteConfig>,
) -> Result<()> {
    let mut args = vec![
        "wt",
        "-C",
        repo_path,
        "remove",
        "--yes",
        "--foreground",
        branch,
    ];
    if force {
        args.push("--force");
    }

    run_cmd(&args, remote).with_context(|| format!("failed to remove worktree {}", branch))?;

    Ok(())
}

/// Fast batch listing using git commands directly instead of `wt`.
/// Works for both local (remote=None) and remote (remote=Some) repos.
/// Returns a map of repo_path -> Vec<Worktree>.
pub fn list_worktrees_batch_fast(
    repo_paths: &[String],
    remote: Option<&RemoteConfig>,
) -> Result<std::collections::HashMap<String, Vec<Worktree>>> {
    use std::collections::HashMap;

    if repo_paths.is_empty() {
        return Ok(HashMap::new());
    }

    // Build a script that runs git commands for each repo
    let paths_joined: Vec<String> = repo_paths
        .iter()
        .map(|p| format!("'{}'", p.replace('\'', "'\\''")))
        .collect();

    let script = format!(
        r#"for d in {}; do
echo "___REPO___$d"
git -C "$d" worktree list --porcelain 2>/dev/null
echo "___STATUS___"
git -C "$d" status --porcelain 2>/dev/null | head -20
echo "___REMOTE___"
git -C "$d" rev-list --left-right --count HEAD...@{{upstream}} 2>/dev/null
echo "___ENDREPO___"
done"#,
        paths_joined.join(" ")
    );

    let output = run_cmd(&["sh", "-c", &script], remote)?;
    Ok(parse_batch_git_output(&output))
}

/// Parse the batch git output into worktrees per repo.
fn parse_batch_git_output(output: &str) -> std::collections::HashMap<String, Vec<Worktree>> {
    use std::collections::HashMap;

    let mut result: HashMap<String, Vec<Worktree>> = HashMap::new();
    let mut current_repo: Option<String> = None;
    let mut section = "worktree"; // "worktree", "status", "remote"
    let mut worktrees: Vec<Worktree> = Vec::new();
    let mut current_wt_path: Option<String> = None;
    let mut current_branch: Option<String> = None;
    let mut is_first_wt = true;
    let mut staged = false;
    let mut modified = false;
    let mut untracked = false;
    let mut ahead: u32 = 0;
    let mut behind: u32 = 0;

    for line in output.lines() {
        if let Some(repo_path) = line.strip_prefix("___REPO___") {
            // Flush previous repo
            flush_worktree(
                &mut worktrees,
                &mut current_wt_path,
                &mut current_branch,
                is_first_wt,
                staged,
                modified,
                untracked,
                ahead,
                behind,
            );
            if let Some(prev_repo) = current_repo.take()
                && !worktrees.is_empty()
            {
                result.insert(prev_repo, worktrees.clone());
            }

            current_repo = Some(repo_path.trim_end_matches('/').to_string());
            worktrees.clear();
            current_wt_path = None;
            current_branch = None;
            is_first_wt = true;
            section = "worktree";
            staged = false;
            modified = false;
            untracked = false;
            ahead = 0;
            behind = 0;
            continue;
        }

        if line == "___STATUS___" {
            // Flush current worktree before status section
            flush_worktree(
                &mut worktrees,
                &mut current_wt_path,
                &mut current_branch,
                is_first_wt,
                false,
                false,
                false,
                0,
                0,
            );
            section = "status";
            staged = false;
            modified = false;
            untracked = false;
            continue;
        }
        if line == "___REMOTE___" {
            section = "remote";
            ahead = 0;
            behind = 0;
            continue;
        }
        if line == "___ENDREPO___" {
            // Apply status/remote to all worktrees of this repo (main worktree)
            // Status/remote only applies to the first (main) worktree's current state
            if !worktrees.is_empty() {
                worktrees[0].working_tree = Some(WorktreeWorkingTree {
                    staged,
                    modified,
                    untracked,
                    diff: WorktreeDiff::default(),
                });
                if ahead > 0 || behind > 0 {
                    worktrees[0].remote = Some(WorktreeRemote { ahead, behind });
                }
            }
            if let Some(prev_repo) = current_repo.take()
                && !worktrees.is_empty()
            {
                result.insert(prev_repo, worktrees.clone());
            }
            worktrees.clear();
            continue;
        }

        match section {
            "worktree" => {
                if let Some(path) = line.strip_prefix("worktree ") {
                    let had_previous = current_wt_path.is_some();
                    // Flush previous worktree
                    flush_worktree(
                        &mut worktrees,
                        &mut current_wt_path,
                        &mut current_branch,
                        is_first_wt,
                        false,
                        false,
                        false,
                        0,
                        0,
                    );
                    if had_previous {
                        is_first_wt = false;
                    }
                    current_wt_path = Some(path.to_string());
                    current_branch = None;
                } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                    current_branch = Some(branch.to_string());
                }
                // Ignore HEAD, bare, detached lines
            }
            "status" => {
                if line.is_empty() {
                    continue;
                }
                let bytes = line.as_bytes();
                if bytes.len() >= 2 {
                    let x = bytes[0];
                    let y = bytes[1];
                    if x == b'?' && y == b'?' {
                        untracked = true;
                    } else {
                        if x != b' ' && x != b'?' {
                            staged = true;
                        }
                        if y != b' ' && y != b'?' {
                            modified = true;
                        }
                    }
                }
            }
            "remote" => {
                // Format: "3\t5" (ahead\tbehind)
                if let Some((a, b)) = line.split_once('\t') {
                    ahead = a.trim().parse().unwrap_or(0);
                    behind = b.trim().parse().unwrap_or(0);
                }
            }
            _ => {}
        }
    }

    result
}

#[allow(clippy::too_many_arguments)]
fn flush_worktree(
    worktrees: &mut Vec<Worktree>,
    current_path: &mut Option<String>,
    current_branch: &mut Option<String>,
    is_first: bool,
    staged: bool,
    modified: bool,
    untracked: bool,
    ahead: u32,
    behind: u32,
) {
    if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
        let wt = Worktree {
            branch,
            path,
            is_main: is_first,
            is_current: false,
            commit: None,
            working_tree: if staged || modified || untracked {
                Some(WorktreeWorkingTree {
                    staged,
                    modified,
                    untracked,
                    diff: WorktreeDiff::default(),
                })
            } else {
                Some(WorktreeWorkingTree::default())
            },
            remote: if ahead > 0 || behind > 0 {
                Some(WorktreeRemote { ahead, behind })
            } else {
                None
            },
        };
        worktrees.push(wt);
    }
}

/// Find a worktree by branch name across a repo.
pub fn find_worktree(
    repo_path: &str,
    branch: &str,
    remote: Option<&RemoteConfig>,
) -> Result<Option<Worktree>> {
    let worktrees = list_worktrees(repo_path, remote)?;
    Ok(worktrees.into_iter().find(|w| w.branch == branch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_batch_single_repo_single_worktree() {
        let output = "\
___REPO___/home/user/Code/myrepo
worktree /home/user/Code/myrepo
HEAD abc123
branch refs/heads/main

___STATUS___
___REMOTE___
3\t1
___ENDREPO___
";
        let result = parse_batch_git_output(output);
        assert_eq!(result.len(), 1);
        let wts = &result["/home/user/Code/myrepo"];
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].branch, "main");
        assert_eq!(wts[0].path, "/home/user/Code/myrepo");
        assert!(wts[0].is_main);
        // Remote ahead/behind applied to first worktree
        let remote = wts[0].remote.as_ref().unwrap();
        assert_eq!(remote.ahead, 3);
        assert_eq!(remote.behind, 1);
    }

    #[test]
    fn test_parse_batch_multiple_worktrees() {
        let output = "\
___REPO___/home/user/Code/myrepo
worktree /home/user/Code/myrepo
HEAD abc123
branch refs/heads/main

worktree /home/user/Code/myrepo-feature
HEAD def456
branch refs/heads/feature

___STATUS___
 M src/main.rs
?? untracked.txt
___REMOTE___
___ENDREPO___
";
        let result = parse_batch_git_output(output);
        let wts = &result["/home/user/Code/myrepo"];
        assert_eq!(wts.len(), 2);

        // First worktree is main
        assert_eq!(wts[0].branch, "main");
        assert!(wts[0].is_main);

        // Second worktree is not main
        assert_eq!(wts[1].branch, "feature");
        assert!(!wts[1].is_main);

        // Status applied to first worktree
        // " M" = modified in working tree (space in index, M in worktree)
        let wt = wts[0].working_tree.as_ref().unwrap();
        assert!(wt.modified);
        assert!(wt.untracked);
        assert!(!wt.staged);
    }

    #[test]
    fn test_parse_batch_status_parsing() {
        let output = "\
___REPO___/repo
worktree /repo
HEAD abc
branch refs/heads/main

___STATUS___
A  new_file.rs
 M modified.rs
?? untracked.txt
___REMOTE___
___ENDREPO___
";
        let result = parse_batch_git_output(output);
        let wts = &result["/repo"];
        let wt = wts[0].working_tree.as_ref().unwrap();
        assert!(wt.staged); // A in index column
        assert!(wt.modified); // M in working tree column
        assert!(wt.untracked); // ??
    }

    #[test]
    fn test_parse_batch_multiple_repos() {
        let output = "\
___REPO___/repo1
worktree /repo1
HEAD abc
branch refs/heads/main

___STATUS___
___REMOTE___
___ENDREPO___
___REPO___/repo2
worktree /repo2
HEAD def
branch refs/heads/develop

___STATUS___
___REMOTE___
___ENDREPO___
";
        let result = parse_batch_git_output(output);
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("/repo1"));
        assert!(result.contains_key("/repo2"));
        assert_eq!(result["/repo1"][0].branch, "main");
        assert_eq!(result["/repo2"][0].branch, "develop");
    }

    #[test]
    fn test_parse_batch_trailing_slash_stripped() {
        let output = "\
___REPO___/repo/
worktree /repo
HEAD abc
branch refs/heads/main

___STATUS___
___REMOTE___
___ENDREPO___
";
        let result = parse_batch_git_output(output);
        assert!(result.contains_key("/repo"));
    }

    #[test]
    fn test_parse_batch_empty_input() {
        let result = parse_batch_git_output("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_batch_no_remote_tracking() {
        let output = "\
___REPO___/repo
worktree /repo
HEAD abc
branch refs/heads/main

___STATUS___
___REMOTE___
___ENDREPO___
";
        let result = parse_batch_git_output(output);
        let wts = &result["/repo"];
        assert!(wts[0].remote.is_none());
    }

    #[test]
    fn test_parse_batch_clean_status() {
        let output = "\
___REPO___/repo
worktree /repo
HEAD abc
branch refs/heads/main

___STATUS___
___REMOTE___
0\t0
___ENDREPO___
";
        let result = parse_batch_git_output(output);
        let wts = &result["/repo"];
        let wt = wts[0].working_tree.as_ref().unwrap();
        assert!(!wt.staged);
        assert!(!wt.modified);
        assert!(!wt.untracked);
        // 0/0 ahead/behind means no remote tracking info stored
        assert!(wts[0].remote.is_none());
    }
}
