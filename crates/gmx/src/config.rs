use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub use ghostty_lib::remote::RemoteConfig;

/// gmx config: remotes and defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmxConfig {
    #[serde(default = "default_split_direction")]
    pub default_split_direction: String,
    #[serde(default)]
    pub remotes: HashMap<String, RemoteConfig>,
}

fn default_split_direction() -> String {
    "right".to_string()
}

impl Default for GmxConfig {
    fn default() -> Self {
        Self {
            default_split_direction: default_split_direction(),
            remotes: HashMap::new(),
        }
    }
}

/// Session registry: maps session name -> metadata (remote, dir).
/// This stores only what can't be derived from zmx.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionRegistry {
    #[serde(default)]
    pub sessions: HashMap<String, SessionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    pub dir: String,
}

impl GmxConfig {
    pub fn config_dir() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .context("could not determine config directory")?
            .join("gmx");
        Ok(dir)
    }

    fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let config: GmxConfig = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse config at {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir()?;
        fs::create_dir_all(&dir)?;
        let path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn get_remote(&self, name: &str) -> Option<&RemoteConfig> {
        self.remotes.get(name)
    }
}

impl SessionRegistry {
    fn registry_path() -> Result<PathBuf> {
        Ok(GmxConfig::config_dir()?.join("sessions.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::registry_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read session registry at {}", path.display()))?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    }

    pub fn save(&self) -> Result<()> {
        let dir = GmxConfig::config_dir()?;
        fs::create_dir_all(&dir)?;
        let path = Self::registry_path()?;
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn register(&mut self, name: &str, remote: Option<&str>, dir: &str) {
        self.sessions.insert(
            name.to_string(),
            SessionEntry {
                remote: remote.map(|s| s.to_string()),
                dir: dir.to_string(),
            },
        );
    }

    pub fn get(&self, name: &str) -> Option<&SessionEntry> {
        self.sessions.get(name)
    }

    pub fn remove(&mut self, name: &str) {
        self.sessions.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gmx_config_defaults() {
        let config = GmxConfig::default();
        assert_eq!(config.default_split_direction, "right");
        assert!(config.remotes.is_empty());
    }

    #[test]
    fn test_gmx_config_serialization() {
        let mut config = GmxConfig::default();
        config.remotes.insert(
            "nicbook".to_string(),
            RemoteConfig {
                host: "nicbook".to_string(),
                user: Some("nico".to_string()),
                scan_dirs: vec![],
                transport: "ssh".to_string(),
            },
        );

        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: GmxConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.default_split_direction, "right");
        let remote = parsed.get_remote("nicbook").unwrap();
        assert_eq!(remote.host, "nicbook");
        assert_eq!(remote.user, Some("nico".to_string()));
    }

    #[test]
    fn test_gmx_config_deserialization_missing_fields() {
        let json = "{}";
        let config: GmxConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.default_split_direction, "right");
        assert!(config.remotes.is_empty());
    }

    #[test]
    fn test_session_registry_operations() {
        let mut reg = SessionRegistry::default();
        assert!(reg.get("test").is_none());

        reg.register("test", Some("nicbook"), "/home/user/Code/test");
        let entry = reg.get("test").unwrap();
        assert_eq!(entry.remote, Some("nicbook".to_string()));
        assert_eq!(entry.dir, "/home/user/Code/test");

        reg.register("local-project", None, "/Users/nico/Code/project");
        let entry = reg.get("local-project").unwrap();
        assert!(entry.remote.is_none());
        assert_eq!(entry.dir, "/Users/nico/Code/project");

        reg.remove("test");
        assert!(reg.get("test").is_none());
        assert!(reg.get("local-project").is_some());
    }

    #[test]
    fn test_session_registry_serialization() {
        let mut reg = SessionRegistry::default();
        reg.register("my-project", Some("nicbook"), "~/Code/project");
        reg.register("local-thing", None, "/Users/nico/Code/thing");

        let json = serde_json::to_string_pretty(&reg).unwrap();
        let parsed: SessionRegistry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.sessions.len(), 2);
        let entry = parsed.get("my-project").unwrap();
        assert_eq!(entry.remote, Some("nicbook".to_string()));
        assert_eq!(entry.dir, "~/Code/project");
    }

    #[test]
    fn test_session_registry_overwrite() {
        let mut reg = SessionRegistry::default();
        reg.register("test", Some("host1"), "/path1");
        reg.register("test", Some("host2"), "/path2");

        let entry = reg.get("test").unwrap();
        assert_eq!(entry.remote, Some("host2".to_string()));
        assert_eq!(entry.dir, "/path2");
    }

    #[test]
    fn test_session_registry_save_load_roundtrip() {
        let dir = std::path::PathBuf::from(std::env::temp_dir()).join("gmx-test-registry");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Write registry file directly (bypass config_dir)
        let mut reg = SessionRegistry::default();
        reg.register("session1", None, "/tmp/s1");
        reg.register("session2", Some("host"), "/tmp/s2");

        let path = dir.join("sessions.json");
        let content = serde_json::to_string_pretty(&reg).unwrap();
        std::fs::write(&path, content).unwrap();

        // Read it back
        let loaded_content = std::fs::read_to_string(&path).unwrap();
        let loaded: SessionRegistry = serde_json::from_str(&loaded_content).unwrap();
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(loaded.get("session1").unwrap().dir, "/tmp/s1");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
