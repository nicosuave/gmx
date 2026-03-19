use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::ghostty;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TerminalState {
    /// session_name -> list of Ghostty terminal IDs
    pub terminals: HashMap<String, Vec<String>>,
}

impl TerminalState {
    fn state_path(base_dir: &Path) -> std::path::PathBuf {
        base_dir.join("terminals.json")
    }

    pub fn load(base_dir: &Path) -> Result<Self> {
        let path = Self::state_path(base_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    }

    pub fn save(&self, base_dir: &Path) -> Result<()> {
        fs::create_dir_all(base_dir)?;
        let path = Self::state_path(base_dir);
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
        self.terminals
            .get(session_name)
            .cloned()
            .unwrap_or_default()
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
    pub fn find_valid_terminal(&self, repo_name: &str, branch: &str) -> Option<(String, String)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_add_and_get() {
        let mut state = TerminalState::default();
        state.add("foo.main", "term-1");
        state.add("foo.main", "term-2");
        state.add("bar.dev", "term-3");

        assert_eq!(state.get("foo.main"), vec!["term-1", "term-2"]);
        assert_eq!(state.get("bar.dev"), vec!["term-3"]);
        assert!(state.get("nonexistent").is_empty());
    }

    #[test]
    fn test_remove_session() {
        let mut state = TerminalState::default();
        state.add("foo.main", "term-1");
        state.add("foo.main", "term-2");
        state.add("bar.dev", "term-3");

        state.remove_session("foo.main");
        assert!(state.get("foo.main").is_empty());
        assert_eq!(state.get("bar.dev"), vec!["term-3"]);
    }

    #[test]
    fn test_remove_terminal() {
        let mut state = TerminalState::default();
        state.add("foo.main", "term-1");
        state.add("foo.main", "term-2");
        state.add("bar.dev", "term-2"); // same ID in different session

        state.remove_terminal("term-2");
        assert_eq!(state.get("foo.main"), vec!["term-1"]);
        assert!(state.get("bar.dev").is_empty()); // cleaned up empty entry
    }

    #[test]
    fn test_get_worktree_terminals() {
        let mut state = TerminalState::default();
        state.add("myrepo.main", "t1");
        state.add("myrepo.main.2", "t2");
        state.add("myrepo.main.3", "t3");
        state.add("myrepo.feature", "t4");
        state.add("other.main", "t5");

        let wt = state.get_worktree_terminals("myrepo", "main");
        let names: Vec<String> = wt.iter().map(|(n, _)| n.clone()).collect();
        assert!(names.contains(&"myrepo.main".to_string()));
        assert!(names.contains(&"myrepo.main.2".to_string()));
        assert!(names.contains(&"myrepo.main.3".to_string()));
        assert_eq!(wt.len(), 3);
    }

    #[test]
    fn test_get_worktree_terminals_no_false_match() {
        let mut state = TerminalState::default();
        state.add("repo.main", "t1");
        state.add("repo.main-v2", "t2"); // different branch, not a variant

        let wt = state.get_worktree_terminals("repo", "main");
        // "repo.main-v2" should NOT match "repo.main" prefix
        // But with current implementation it starts_with("repo.main.") so it won't match
        assert_eq!(wt.len(), 1);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = PathBuf::from(std::env::temp_dir()).join("ghostty-lib-test-state");
        let _ = std::fs::remove_dir_all(&dir);

        let mut state = TerminalState::default();
        state.add("session.1", "terminal-abc");
        state.add("session.1", "terminal-def");
        state.add("session.2", "terminal-ghi");

        state.save(&dir).unwrap();

        let loaded = TerminalState::load(&dir).unwrap();
        assert_eq!(
            loaded.get("session.1"),
            vec!["terminal-abc", "terminal-def"]
        );
        assert_eq!(loaded.get("session.2"), vec!["terminal-ghi"]);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let dir = PathBuf::from("/tmp/ghostty-lib-test-nonexistent-12345");
        let state = TerminalState::load(&dir).unwrap();
        assert!(state.terminals.is_empty());
    }

    #[test]
    fn test_serialization_format() {
        let mut state = TerminalState::default();
        state.add("test", "id-1");
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"test\""));
        assert!(json.contains("\"id-1\""));

        let parsed: TerminalState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.get("test"), vec!["id-1"]);
    }
}
