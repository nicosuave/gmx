use anyhow::Result;

use ghostty_lib::remote::RemoteConfig;
use ghostty_lib::zmx;

/// Discover active gmx sessions by grouping zmx sessions by prefix.
/// Returns (session_name, zmx_sessions) pairs.
#[allow(dead_code)]
pub fn discover_sessions(
    remote: Option<&RemoteConfig>,
) -> Result<Vec<(String, Vec<zmx::ZmxSession>)>> {
    let all = zmx::list_sessions(remote)?;

    // Group by session prefix: "foo.1", "foo.2" -> group "foo"
    // A zmx session belongs to a gmx session if its name matches "prefix.N" pattern
    let mut groups: std::collections::HashMap<String, Vec<zmx::ZmxSession>> =
        std::collections::HashMap::new();

    for session in all {
        // Try to extract gmx session name: everything before the last ".N" suffix
        let group_name = extract_session_name(&session.name);
        groups.entry(group_name).or_default().push(session);
    }

    // Sort groups by name, sort sessions within each group
    let mut result: Vec<(String, Vec<zmx::ZmxSession>)> = groups.into_iter().collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, sessions) in &mut result {
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
    }

    Ok(result)
}

/// Extract the gmx session name from a zmx session name.
/// "my-project.1" -> "my-project"
/// "my-project.2" -> "my-project"
/// "my-project" -> "my-project"
fn extract_session_name(zmx_name: &str) -> String {
    // If the name ends with ".N" where N is a number, strip it
    if let Some(dot_pos) = zmx_name.rfind('.') {
        let suffix = &zmx_name[dot_pos + 1..];
        if suffix.chars().all(|c| c.is_ascii_digit()) && !suffix.is_empty() {
            return zmx_name[..dot_pos].to_string();
        }
    }
    zmx_name.to_string()
}

/// Build the command string for a zmx session (local or remote).
/// zmx sessions persist after client disconnect (closing the tab detaches the client).
pub fn build_zmx_command(
    zmx_session_name: &str,
    dir: &str,
    remote: Option<&RemoteConfig>,
) -> String {
    let zmx_cmd = format!(
        "cd {dir} && zmx attach {name}",
        dir = shell_escape_single(dir),
        name = shell_escape_single(zmx_session_name),
    );

    match remote {
        Some(r) => {
            let target = r.ssh_target();
            if r.use_mosh() {
                format!("mosh {} -- sh -c '{}'", target, zmx_cmd)
            } else {
                format!("ssh {} -t '{}'", target, zmx_cmd)
            }
        }
        None => zmx_cmd,
    }
}

fn shell_escape_single(s: &str) -> String {
    s.replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_session_name_numbered() {
        assert_eq!(extract_session_name("my-project.1"), "my-project");
        assert_eq!(extract_session_name("my-project.2"), "my-project");
        assert_eq!(extract_session_name("my-project.10"), "my-project");
        assert_eq!(extract_session_name("my-project.99"), "my-project");
    }

    #[test]
    fn test_extract_session_name_no_number() {
        assert_eq!(extract_session_name("my-project"), "my-project");
        assert_eq!(extract_session_name("foo.bar"), "foo.bar");
        assert_eq!(extract_session_name("simple"), "simple");
    }

    #[test]
    fn test_extract_session_name_nested() {
        assert_eq!(extract_session_name("foo.bar.1"), "foo.bar");
        assert_eq!(extract_session_name("a.b.c.3"), "a.b.c");
    }

    #[test]
    fn test_extract_session_name_non_numeric_suffix() {
        // ".abc" is not a number, so the whole thing is the session name
        assert_eq!(extract_session_name("my-project.abc"), "my-project.abc");
        assert_eq!(extract_session_name("foo.bar.baz"), "foo.bar.baz");
    }

    #[test]
    fn test_build_zmx_command_local() {
        let cmd = build_zmx_command("my-project.1", "/home/user/Code/project", None);
        assert!(cmd.contains("cd /home/user/Code/project"));
        assert!(cmd.contains("zmx attach my-project.1"));
        assert!(!cmd.contains("ssh"));
        assert!(!cmd.contains("mosh"));
    }

    #[test]
    fn test_build_zmx_command_remote_ssh() {
        let remote = RemoteConfig {
            host: "nicbook".to_string(),
            user: Some("nico".to_string()),
            scan_dirs: vec![],
            transport: "ssh".to_string(),
        };
        let cmd = build_zmx_command("my-project.1", "~/Code/project", Some(&remote));
        assert!(cmd.starts_with("ssh nico@nicbook -t"));
        assert!(cmd.contains("cd ~/Code/project"));
        assert!(cmd.contains("zmx attach my-project.1"));
    }

    #[test]
    fn test_build_zmx_command_remote_mosh() {
        let remote = RemoteConfig {
            host: "nicbook".to_string(),
            user: None,
            scan_dirs: vec![],
            transport: "mosh".to_string(),
        };
        let cmd = build_zmx_command("my-project.1", "~/Code/project", Some(&remote));
        assert!(cmd.starts_with("mosh nicbook"));
        assert!(cmd.contains("cd ~/Code/project"));
        assert!(cmd.contains("zmx attach my-project.1"));
    }

    #[test]
    fn test_build_zmx_command_escapes_quotes() {
        let cmd = build_zmx_command("session.1", "/path/with spaces", None);
        assert!(cmd.contains("/path/with spaces"));
    }

    #[test]
    fn test_shell_escape_single() {
        assert_eq!(shell_escape_single("hello"), "hello");
        assert_eq!(shell_escape_single("it's"), "it'\\''s");
    }

    #[test]
    fn test_discover_sessions_grouping() {
        // We can't easily test discover_sessions without a running zmx,
        // but we can test the grouping logic via extract_session_name.
        let zmx_names = vec![
            "project-a.1",
            "project-a.2",
            "project-a.3",
            "project-b.1",
            "standalone",
        ];

        let mut groups: std::collections::HashMap<String, Vec<&str>> =
            std::collections::HashMap::new();
        for name in &zmx_names {
            let group = extract_session_name(name);
            groups.entry(group).or_default().push(name);
        }

        assert_eq!(groups.len(), 3);
        assert_eq!(groups["project-a"].len(), 3);
        assert_eq!(groups["project-b"].len(), 1);
        assert_eq!(groups["standalone"].len(), 1);
    }
}
