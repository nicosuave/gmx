use colored::Colorize;

use crate::worktree::Worktree;
use crate::zmx::ZmxSession;

/// Combined view of a worktree and its optional zmx session.
pub struct WorktreeDisplay<'a> {
    pub env_name: &'a str,
    pub repo_name: &'a str,
    pub worktree: &'a Worktree,
    pub session: Option<&'a ZmxSession>,
}

impl<'a> WorktreeDisplay<'a> {
    pub fn session_indicator(&self) -> String {
        match &self.session {
            Some(s) if s.is_attached() => format!("{}", "●".green()),
            Some(_) => format!("{}", "◌".yellow()),
            None => format!("{}", "-".dimmed()),
        }
    }

    pub fn session_label(&self) -> String {
        match &self.session {
            Some(s) if s.is_attached() => "attached".green().to_string(),
            Some(_) => "detached".yellow().to_string(),
            None => "none".dimmed().to_string(),
        }
    }

    pub fn tracking_info(&self) -> String {
        let mut parts = Vec::new();

        if let Some(remote) = &self.worktree.remote {
            if remote.ahead > 0 {
                parts.push(format!("{}{}", "↑".green(), remote.ahead));
            }
            if remote.behind > 0 {
                parts.push(format!("{}{}", "↓".red(), remote.behind));
            }
        }

        if let Some(wt) = &self.worktree.working_tree {
            let mut status = Vec::new();
            if wt.staged {
                status.push("staged".yellow().to_string());
            }
            if wt.modified {
                status.push("modified".red().to_string());
            }
            if wt.untracked {
                status.push("untracked".dimmed().to_string());
            }
            if !status.is_empty() {
                parts.push(status.join(","));
            }
            if wt.diff.added > 0 || wt.diff.deleted > 0 {
                parts.push(format!(
                    "{}{}",
                    format!("+{}", wt.diff.added).green(),
                    format!("-{}", wt.diff.deleted).red()
                ));
            }
        }

        if parts.is_empty() {
            "clean".dimmed().to_string()
        } else {
            parts.join(" ")
        }
    }

    /// Plain-text single-line display for the picker (no ANSI codes).
    pub fn picker_line(&self) -> String {
        let indicator = match &self.session {
            Some(s) if s.is_attached() => "●",
            Some(_) => "◌",
            None => " ",
        };

        let status = self.tracking_info_plain();
        let status_part = if status == "clean" {
            String::new()
        } else {
            format!(" [{}]", status)
        };

        format!("{} {}: {}/{}{}", indicator, self.env_name, self.repo_name, self.worktree.branch, status_part)
    }

    /// Sort key: (has_session descending, commit_timestamp descending)
    pub fn sort_key(&self) -> (u8, u64) {
        let session_rank = match &self.session {
            Some(s) if s.is_attached() => 0, // attached first
            Some(_) => 1,                     // detached second
            None => 2,                        // no session last
        };
        let timestamp = self.worktree.commit
            .as_ref()
            .map(|c| c.timestamp)
            .unwrap_or(0);
        // Invert timestamp so higher = earlier in sort (most recent first)
        (session_rank, u64::MAX - timestamp)
    }

    fn tracking_info_plain(&self) -> String {
        let mut parts = Vec::new();

        if let Some(remote) = &self.worktree.remote {
            if remote.ahead > 0 {
                parts.push(format!("↑{}", remote.ahead));
            }
            if remote.behind > 0 {
                parts.push(format!("↓{}", remote.behind));
            }
        }

        if let Some(wt) = &self.worktree.working_tree {
            let mut status = Vec::new();
            if wt.staged { status.push("staged"); }
            if wt.modified { status.push("modified"); }
            if wt.untracked { status.push("untracked"); }
            if !status.is_empty() {
                parts.push(status.join(","));
            }
            if wt.diff.added > 0 || wt.diff.deleted > 0 {
                parts.push(format!("+{}-{}", wt.diff.added, wt.diff.deleted));
            }
        }

        if parts.is_empty() {
            "clean".to_string()
        } else {
            parts.join(" ")
        }
    }
}

/// Print the full worktree table.
pub fn print_worktree_table(entries: &[WorktreeDisplay]) {
    if entries.is_empty() {
        println!("{}", "No worktrees found. Add a repo with: ghostreelite repo add <path>".dimmed());
        return;
    }

    // Header
    println!(
        "  {:<10} {:<16} {:<28} {:<20} {}",
        "ENV".dimmed(),
        "REPO".dimmed(),
        "BRANCH".dimmed(),
        "STATUS".dimmed(),
        "SESSION".dimmed(),
    );

    for entry in entries {
        let branch = if entry.worktree.is_main {
            entry.worktree.branch.bold().to_string()
        } else {
            entry.worktree.branch.clone()
        };

        println!(
            "  {:<10} {:<16} {:<28} {:<20} {} {}",
            entry.env_name,
            entry.repo_name,
            branch,
            entry.tracking_info(),
            entry.session_indicator(),
            entry.session_label(),
        );
    }
}
