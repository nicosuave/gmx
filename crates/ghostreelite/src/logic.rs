use ghostty_lib::zmx::ZmxSession;

/// Find the primary zmx session for a worktree by repo name and branch.
pub fn find_session_for_worktree<'a>(
    sessions: &'a [ZmxSession],
    repo_name: &str,
    branch: &str,
) -> Option<&'a ZmxSession> {
    let name = ghostty_lib::zmx::session_name(repo_name, branch);
    sessions.iter().find(|s| s.name == name)
}

/// Determine the picker action from a key event.
#[derive(Debug, PartialEq)]
pub enum PickerAction {
    /// Open/reattach the selected worktree
    Open,
    /// Create a new zmx session in a new tab
    NewSession,
    /// Split right
    SplitRight,
    /// Split down
    SplitDown,
    /// Create a new worktree
    NewWorktree,
    /// Remove dialog
    Remove,
}

/// Resolve picker action from ctrl+key combination.
pub fn resolve_picker_action(is_ctrl: bool, key_char: Option<char>) -> PickerAction {
    if is_ctrl {
        match key_char {
            Some('a') => PickerAction::NewWorktree,
            Some('d') => PickerAction::Remove,
            Some('n') => PickerAction::NewSession,
            Some('s') => PickerAction::SplitRight,
            Some('v') => PickerAction::SplitDown,
            _ => PickerAction::Open,
        }
    } else {
        PickerAction::Open
    }
}

/// Match a picker selection text back to an index in the items list.
#[allow(dead_code)]
pub fn match_picker_selection(items: &[(String, usize)], selected_text: &str) -> Option<usize> {
    items
        .iter()
        .find(|(display, _)| display == selected_text)
        .map(|(_, idx)| *idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(name: &str, clients: u32) -> ZmxSession {
        ZmxSession {
            name: name.to_string(),
            pid: Some("1234".to_string()),
            clients,
            started_in: None,
        }
    }

    #[test]
    fn test_find_session_for_worktree_exists() {
        let sessions = vec![
            make_session("myrepo.main", 1),
            make_session("myrepo.feature", 0),
            make_session("other.main", 2),
        ];
        let found = find_session_for_worktree(&sessions, "myrepo", "main");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "myrepo.main");
        assert!(found.unwrap().is_attached());
    }

    #[test]
    fn test_find_session_for_worktree_not_found() {
        let sessions = vec![make_session("myrepo.main", 1)];
        let found = find_session_for_worktree(&sessions, "myrepo", "develop");
        assert!(found.is_none());
    }

    #[test]
    fn test_find_session_for_worktree_empty() {
        let sessions: Vec<ZmxSession> = vec![];
        let found = find_session_for_worktree(&sessions, "repo", "main");
        assert!(found.is_none());
    }

    #[test]
    fn test_find_session_no_partial_match() {
        let sessions = vec![make_session("myrepo.main.2", 0)];
        // Should not match "myrepo.main" because exact match is required
        let found = find_session_for_worktree(&sessions, "myrepo", "main");
        assert!(found.is_none());
    }

    #[test]
    fn test_resolve_picker_action_enter() {
        assert_eq!(resolve_picker_action(false, None), PickerAction::Open);
        assert_eq!(resolve_picker_action(false, Some('a')), PickerAction::Open);
    }

    #[test]
    fn test_resolve_picker_action_ctrl_keys() {
        assert_eq!(
            resolve_picker_action(true, Some('a')),
            PickerAction::NewWorktree
        );
        assert_eq!(resolve_picker_action(true, Some('d')), PickerAction::Remove);
        assert_eq!(
            resolve_picker_action(true, Some('n')),
            PickerAction::NewSession
        );
        assert_eq!(
            resolve_picker_action(true, Some('s')),
            PickerAction::SplitRight
        );
        assert_eq!(
            resolve_picker_action(true, Some('v')),
            PickerAction::SplitDown
        );
    }

    #[test]
    fn test_resolve_picker_action_unknown_ctrl() {
        assert_eq!(resolve_picker_action(true, Some('z')), PickerAction::Open);
        assert_eq!(resolve_picker_action(true, None), PickerAction::Open);
    }

    #[test]
    fn test_match_picker_selection() {
        let items = vec![
            ("  local: myrepo/main".to_string(), 0),
            ("  local: myrepo/feature".to_string(), 1),
            ("● nicbook: other/dev".to_string(), 2),
        ];

        assert_eq!(
            match_picker_selection(&items, "  local: myrepo/main"),
            Some(0)
        );
        assert_eq!(
            match_picker_selection(&items, "● nicbook: other/dev"),
            Some(2)
        );
        assert_eq!(match_picker_selection(&items, "nonexistent"), None);
    }
}
