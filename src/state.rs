use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

use crate::ghostty;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TerminalState {
    /// session_name -> list of Ghostty terminal IDs
    pub terminals: HashMap<String, Vec<String>>,
}

impl TerminalState {
    fn state_path() -> Result<std::path::PathBuf> {
        let dir = crate::config::Config::config_dir()?;
        Ok(dir.join("terminals.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::state_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    }

    pub fn save(&self) -> Result<()> {
        let dir = crate::config::Config::config_dir()?;
        fs::create_dir_all(&dir)?;
        let path = Self::state_path()?;
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    /// Add a terminal ID for a session.
    pub fn add(&mut self, session_name: &str, terminal_id: &str) {
        self.terminals
            .entry(session_name.to_string())
            .or_default()
            .push(terminal_id.to_string());
    }

    /// Get all terminal IDs for a specific session.
    pub fn get(&self, session_name: &str) -> Vec<String> {
        self.terminals.get(session_name).cloned().unwrap_or_default()
    }

    /// Get all terminal IDs for all sessions belonging to a worktree.
    /// Matches base name and numbered variants: repo.branch, repo.branch.2, etc.
    pub fn get_worktree_terminals(
        &self,
        repo_name: &str,
        branch: &str,
    ) -> Vec<(String, Vec<String>)> {
        let base = format!("{}.{}", repo_name, branch);
        self.terminals
            .iter()
            .filter(|(name, _)| **name == base || name.starts_with(&format!("{}.", base)))
            .map(|(name, ids)| (name.clone(), ids.clone()))
            .collect()
    }

    /// Remove a specific terminal ID from all sessions.
    #[allow(dead_code)]
    pub fn remove_terminal(&mut self, terminal_id: &str) {
        for ids in self.terminals.values_mut() {
            ids.retain(|id| id != terminal_id);
        }
        // Clean up empty entries
        self.terminals.retain(|_, ids| !ids.is_empty());
    }

    /// Remove all terminals for a session.
    pub fn remove_session(&mut self, session_name: &str) {
        self.terminals.remove(session_name);
    }

    /// Validate all terminal IDs, removing ones that no longer exist in Ghostty.
    #[allow(dead_code)]
    pub fn cleanup(&mut self) -> Result<()> {
        let mut stale = Vec::new();
        for ids in self.terminals.values() {
            for id in ids {
                if !ghostty::terminal_exists(id) {
                    stale.push(id.clone());
                }
            }
        }
        for id in stale {
            self.remove_terminal(&id);
        }
        Ok(())
    }

    /// Find the first valid terminal ID for any session of a worktree.
    pub fn find_valid_terminal(
        &self,
        repo_name: &str,
        branch: &str,
    ) -> Option<(String, String)> {
        let wt_terminals = self.get_worktree_terminals(repo_name, branch);
        for (session_name, ids) in wt_terminals {
            for id in ids {
                if ghostty::terminal_exists(&id) {
                    return Some((session_name, id));
                }
            }
        }
        None
    }
}
