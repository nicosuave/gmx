use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Directories on the remote to scan for git repos
    #[serde(default)]
    pub scan_dirs: Vec<String>,
    /// "ssh" (default) or "mosh" for interactive sessions
    #[serde(default = "default_transport")]
    pub transport: String,
}

fn default_transport() -> String {
    "ssh".to_string()
}

impl RemoteConfig {
    pub fn ssh_target(&self) -> String {
        match &self.user {
            Some(user) => format!("{}@{}", user, self.host),
            None => self.host.clone(),
        }
    }

    pub fn use_mosh(&self) -> bool {
        self.transport == "mosh"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_target_with_user() {
        let r = RemoteConfig {
            host: "myhost".to_string(),
            user: Some("nico".to_string()),
            scan_dirs: vec![],
            transport: "ssh".to_string(),
        };
        assert_eq!(r.ssh_target(), "nico@myhost");
    }

    #[test]
    fn test_ssh_target_no_user() {
        let r = RemoteConfig {
            host: "myhost".to_string(),
            user: None,
            scan_dirs: vec![],
            transport: "ssh".to_string(),
        };
        assert_eq!(r.ssh_target(), "myhost");
    }

    #[test]
    fn test_use_mosh() {
        let ssh = RemoteConfig {
            host: "h".to_string(),
            user: None,
            scan_dirs: vec![],
            transport: "ssh".to_string(),
        };
        assert!(!ssh.use_mosh());

        let mosh = RemoteConfig {
            host: "h".to_string(),
            user: None,
            scan_dirs: vec![],
            transport: "mosh".to_string(),
        };
        assert!(mosh.use_mosh());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let r = RemoteConfig {
            host: "nicbook".to_string(),
            user: Some("nico".to_string()),
            scan_dirs: vec!["~/Code".to_string()],
            transport: "mosh".to_string(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: RemoteConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.host, "nicbook");
        assert_eq!(parsed.user, Some("nico".to_string()));
        assert_eq!(parsed.scan_dirs, vec!["~/Code"]);
        assert_eq!(parsed.transport, "mosh");
    }

    #[test]
    fn test_deserialization_defaults() {
        let json = r#"{"host": "myhost"}"#;
        let r: RemoteConfig = serde_json::from_str(json).unwrap();
        assert_eq!(r.host, "myhost");
        assert_eq!(r.user, None);
        assert!(r.scan_dirs.is_empty());
        assert_eq!(r.transport, "ssh"); // default
    }
}
