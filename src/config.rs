use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Explicit repos to manage
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
    /// Directories to scan for git repos with worktrees (e.g., ["~/Code"])
    #[serde(default)]
    pub scan_dirs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_shell: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteConfig>,
    #[serde(default = "default_split_direction")]
    pub split_direction: String,
}

fn default_split_direction() -> String {
    "right".to_string()
}


impl Config {
    pub fn config_dir() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .context("could not determine config directory")?
            .join("ghostreelite");
        Ok(dir)
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let config: Config = serde_json::from_str(&content)
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

    /// Get the effective remote for a repo (repo-level overrides global)
    pub fn effective_remote<'a>(&'a self, repo: &'a RepoConfig) -> Option<&'a RemoteConfig> {
        repo.remote.as_ref().or(self.remote.as_ref())
    }

}
