use ghostty_lib::state::TerminalState;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ghostty-lib-integ-{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn test_state_lifecycle() {
    let dir = temp_dir("lifecycle");

    // Start fresh
    let mut state = TerminalState::load(&dir).unwrap();
    assert!(state.terminals.is_empty());

    // Add terminals across sessions
    state.add("project.main", "term-aaa");
    state.add("project.main", "term-bbb");
    state.add("project.main.2", "term-ccc");
    state.add("project.feature", "term-ddd");
    state.save(&dir).unwrap();

    // Reload and verify
    let state = TerminalState::load(&dir).unwrap();
    assert_eq!(state.get("project.main").len(), 2);
    assert_eq!(state.get("project.main.2").len(), 1);
    assert_eq!(state.get("project.feature").len(), 1);

    // Worktree terminals should find main + .2 but not feature
    let wt = state.get_worktree_terminals("project", "main");
    assert_eq!(wt.len(), 2); // project.main and project.main.2

    // Remove a session
    let mut state = state;
    state.remove_session("project.main.2");
    state.save(&dir).unwrap();

    let state = TerminalState::load(&dir).unwrap();
    assert!(state.get("project.main.2").is_empty());
    assert_eq!(state.get("project.main").len(), 2); // still there

    // Remove a specific terminal
    let mut state = state;
    state.remove_terminal("term-aaa");
    state.save(&dir).unwrap();

    let state = TerminalState::load(&dir).unwrap();
    assert_eq!(state.get("project.main"), vec!["term-bbb"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_state_corrupt_json_recovery() {
    let dir = temp_dir("corrupt");
    std::fs::create_dir_all(&dir).unwrap();

    // Write invalid JSON
    std::fs::write(dir.join("terminals.json"), "not valid json{{{").unwrap();

    // Should recover gracefully with empty state
    let state = TerminalState::load(&dir).unwrap();
    assert!(state.terminals.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_state_concurrent_sessions() {
    let dir = temp_dir("concurrent");

    let mut state = TerminalState::default();

    // Simulate multiple sessions for the same worktree (like multiple splits)
    for i in 1..=5 {
        state.add(&format!("repo.main.{}", i), &format!("term-{}", i));
    }
    state.save(&dir).unwrap();

    let loaded = TerminalState::load(&dir).unwrap();
    let wt = loaded.get_worktree_terminals("repo", "main");
    assert_eq!(wt.len(), 5);

    let _ = std::fs::remove_dir_all(&dir);
}
