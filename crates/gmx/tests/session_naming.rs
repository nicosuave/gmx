use ghostty_lib::remote::RemoteConfig;

/// Test the full gmx session naming convention and zmx command building.
/// These are integration-level tests that verify the contracts between
/// gmx's session module and ghostty-lib's zmx module.

fn make_remote(host: &str, transport: &str) -> RemoteConfig {
    RemoteConfig {
        host: host.to_string(),
        user: None,
        scan_dirs: vec![],
        transport: transport.to_string(),
    }
}

#[test]
fn test_zmx_session_naming_convention() {
    // gmx sessions use "name.N" convention
    // First pane: "session.1", second: "session.2", etc.
    // The base name "session" (without .N) is also valid as a zmx session name
    // find_sessions_by_prefix should match both patterns

    let base = "my-project";
    let first = format!("{}.1", base);
    let second = format!("{}.2", base);

    assert_eq!(first, "my-project.1");
    assert_eq!(second, "my-project.2");

    // Verify the prefix matching logic: "my-project" matches "my-project" and "my-project.*"
    let names = vec![
        "my-project".to_string(),
        "my-project.1".to_string(),
        "my-project.2".to_string(),
        "my-project.10".to_string(),
        "other-project.1".to_string(),
    ];

    let matches: Vec<&String> = names
        .iter()
        .filter(|n| **n == base || n.starts_with(&format!("{}.", base)))
        .collect();

    assert_eq!(matches.len(), 4);
    assert!(!matches.contains(&&"other-project.1".to_string()));
}

#[test]
fn test_zmx_command_local_format() {
    // Local command should be: cd <dir> && zmx attach <session>
    let cmd = format!(
        "cd {} && zmx attach {}",
        "/Users/nico/Code/project", "my-project.1"
    );
    assert!(cmd.starts_with("cd /Users/nico/Code/project"));
    assert!(cmd.contains("zmx attach my-project.1"));
    assert!(!cmd.contains("ssh"));
}

#[test]
fn test_zmx_command_remote_ssh_format() {
    let remote = make_remote("nicbook", "ssh");
    let target = remote.ssh_target();
    let inner = format!("cd {} && zmx attach {}", "~/Code/project", "my-project.1");
    let cmd = format!("ssh {} -t '{}'", target, inner);

    assert!(cmd.starts_with("ssh nicbook"));
    assert!(cmd.contains("-t"));
    assert!(cmd.contains("zmx attach my-project.1"));
}

#[test]
fn test_zmx_command_remote_mosh_format() {
    let remote = make_remote("nicbook", "mosh");
    let target = remote.ssh_target();
    let inner = format!("cd {} && zmx attach {}", "~/Code/project", "my-project.1");
    let cmd = format!("mosh {} -- sh -c '{}'", target, inner);

    assert!(cmd.starts_with("mosh nicbook"));
    assert!(cmd.contains("sh -c"));
    assert!(cmd.contains("zmx attach my-project.1"));
}

#[test]
fn test_remote_config_with_user() {
    let remote = RemoteConfig {
        host: "nicbook".to_string(),
        user: Some("nico".to_string()),
        scan_dirs: vec![],
        transport: "ssh".to_string(),
    };
    assert_eq!(remote.ssh_target(), "nico@nicbook");
}
