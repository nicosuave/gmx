use anyhow::{Context, Result, bail};
use std::process::Command;

use crate::remote::RemoteConfig;

/// SSH ControlMaster socket path for connection multiplexing.
/// All SSH calls reuse a single connection per host.
fn ssh_control_path(target: &str) -> String {
    let tmp = std::env::temp_dir();
    format!(
        "{}/ghostreelite-ssh-{}",
        tmp.display(),
        target.replace(['@', '.'], "_")
    )
}

/// Common SSH args for multiplexing.
fn ssh_mux_args(target: &str) -> Vec<String> {
    let ctl = ssh_control_path(target);
    vec![
        "-o".to_string(),
        "ControlMaster=auto".to_string(),
        "-o".to_string(),
        format!("ControlPath={}", ctl),
        "-o".to_string(),
        "ControlPersist=60".to_string(),
        "-o".to_string(),
        "ConnectTimeout=5".to_string(),
    ]
}

/// Run a command locally or via SSH, capturing stdout.
pub fn run_cmd(args: &[&str], remote: Option<&RemoteConfig>) -> Result<String> {
    let output = match remote {
        Some(r) => {
            let target = r.ssh_target();
            let remote_cmd = shell_join(args);
            let mut cmd = Command::new("ssh");
            for arg in ssh_mux_args(&target) {
                cmd.arg(arg);
            }
            cmd.arg(&target).arg(&remote_cmd);
            cmd.output().context("failed to run ssh")?
        }
        None => {
            let (cmd, rest) = args.split_first().context("empty command")?;
            Command::new(cmd)
                .args(rest)
                .output()
                .with_context(|| format!("failed to run {}", cmd))?
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        bail!("command failed ({}): {}", output.status, detail.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run a command locally or via SSH, inheriting stdin/stdout/stderr.
#[allow(dead_code)]
pub fn run_interactive(args: &[&str], remote: Option<&RemoteConfig>) -> Result<i32> {
    let status = match remote {
        Some(r) => {
            let target = r.ssh_target();
            let remote_cmd = shell_join(args);
            Command::new("ssh")
                .args(["-t", &target, &remote_cmd])
                .status()
                .context("failed to run ssh")?
        }
        None => {
            let (cmd, rest) = args.split_first().context("empty command")?;
            Command::new(cmd)
                .args(rest)
                .status()
                .with_context(|| format!("failed to run {}", cmd))?
        }
    };

    Ok(status.code().unwrap_or(1))
}

fn shell_join(args: &[&str]) -> String {
    args.iter()
        .map(|a| shell_escape(a))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.' || c == ':')
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("foo-bar_baz"), "foo-bar_baz");
        assert_eq!(shell_escape("/usr/bin/zmx"), "/usr/bin/zmx");
        assert_eq!(shell_escape("host:port"), "host:port");
    }

    #[test]
    fn test_shell_escape_special_chars() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape("foo;bar"), "'foo;bar'");
    }

    #[test]
    fn test_shell_join() {
        assert_eq!(
            shell_join(&["zmx", "attach", "foo.main"]),
            "zmx attach foo.main"
        );
        assert_eq!(
            shell_join(&["sh", "-c", "echo hello"]),
            "sh -c 'echo hello'"
        );
    }

    #[test]
    fn test_ssh_control_path() {
        let path = ssh_control_path("user@host.example.com");
        assert!(path.contains("ghostreelite-ssh-"));
        assert!(path.contains("user_host_example_com"));
        assert!(!path.contains('@'));
        assert!(!path.contains('.'));
    }

    #[test]
    fn test_ssh_mux_args() {
        let args = ssh_mux_args("user@host");
        assert_eq!(args.len(), 8); // 4 pairs of -o + value
        assert!(args.contains(&"ControlMaster=auto".to_string()));
        assert!(args.contains(&"ControlPersist=60".to_string()));
        assert!(args.contains(&"ConnectTimeout=5".to_string()));
    }
}
