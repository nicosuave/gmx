mod config;
mod session;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use config::{GmxConfig, SessionRegistry};
use ghostty_lib::ghostty;
use ghostty_lib::remote::RemoteConfig;
use ghostty_lib::zmx;

#[derive(Parser)]
#[command(
    name = "gmx",
    about = "gmx - Ghostty Multiplexer with zmx session persistence",
    override_help = "gmx - Ghostty Multiplexer with zmx session persistence\n\nUsage: gmx <command> [args]\n\nCommands:\n  [n]ew [name] [--tab] [--remote R] [--dir D]   Create a new session (name defaults to cwd)\n  [a]ttach <name> [--tab]                       Reattach to a session\n  [d]etach                                      Detach from current session (ctrl+\\ also works)\n  [k]ill [name]                                 Kill a session (defaults to current)\n  [l]s                                          List sessions\n  [s]plit [right|down]                           Add a split to the current session\n  [r]ename <old> <new>                           Rename a session\n  [c]onfig remote <name> <host>                  Configure a remote host\n  key[b]inds install|uninstall|show              Manage Ghostty keybindings\n\nBy default, new and attach work in the current terminal.\nUse --tab to open in a new Ghostty tab instead."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new session (attaches zmx in current terminal by default)
    #[command(alias = "n")]
    New {
        /// Session name (auto-generated if omitted)
        name: Option<String>,
        /// Remote host (must be configured in gmx config)
        #[arg(long)]
        remote: Option<String>,
        /// Working directory
        #[arg(long)]
        dir: Option<String>,
        /// Open in a new Ghostty tab instead of the current terminal
        #[arg(long)]
        tab: bool,
    },
    /// Reattach to a session (attaches in current terminal by default)
    #[command(alias = "a")]
    Attach {
        /// Session name
        name: String,
        /// Recreate full layout in new Ghostty tab(s) instead of attaching in current terminal
        #[arg(long)]
        tab: bool,
    },
    /// Kill a session and all its zmx sessions (current session if no name given)
    #[command(alias = "k")]
    Kill {
        /// Session name (kills current session if omitted)
        name: Option<String>,
    },
    /// Detach from current session (ctrl+\ also works)
    #[command(alias = "d")]
    Detach,
    /// List sessions
    #[command(alias = "l")]
    Ls,
    /// Add a split to the current session
    #[command(alias = "s")]
    Split {
        /// Split direction
        #[arg(default_value = "right")]
        direction: String,
    },
    /// Rename a session
    #[command(alias = "r")]
    Rename {
        /// Current name
        old: String,
        /// New name
        new: String,
    },
    /// Configure gmx settings
    #[command(alias = "c")]
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Install or uninstall Ghostty keybindings for gmx
    #[command(alias = "b")]
    Keybinds {
        #[command(subcommand)]
        command: KeybindsCommands,
    },
}

#[derive(Subcommand)]
enum KeybindsCommands {
    /// Install gmx keybindings into Ghostty config
    Install {
        /// Use tmux-style prefix key (e.g., ctrl+b) instead of direct ctrl+shift bindings
        #[arg(long)]
        prefix: Option<String>,
    },
    /// Remove gmx keybindings from Ghostty config
    Uninstall,
    /// Show what keybindings would be installed (dry run)
    Show {
        /// Use tmux-style prefix key
        #[arg(long)]
        prefix: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Add or update a remote
    Remote {
        /// Remote name (alias)
        name: String,
        /// Host to connect to
        host: String,
        /// SSH user
        #[arg(long)]
        user: Option<String>,
        /// Transport: ssh or mosh
        #[arg(long, default_value = "ssh")]
        transport: String,
    },
    /// Show current config
    Show,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::New {
            name,
            remote,
            dir,
            tab,
        }) => cmd_new(name.as_deref(), remote.as_deref(), dir.as_deref(), tab),
        Some(Commands::Attach { name, tab }) => cmd_attach(&name, tab),
        Some(Commands::Kill { name }) => cmd_kill(name.as_deref()),
        Some(Commands::Detach) => cmd_detach(),
        Some(Commands::Ls) => cmd_ls(),
        Some(Commands::Split { direction }) => cmd_split(&direction),
        Some(Commands::Rename { old, new }) => cmd_rename(&old, &new),
        Some(Commands::Config { command }) => cmd_config(command),
        Some(Commands::Keybinds { command }) => cmd_keybinds(command),
        None => {
            // Show the same help as --help
            Cli::parse_from(["gmx", "--help"]);
            Ok(())
        }
    }
}

fn resolve_remote(config: &GmxConfig, name: Option<&str>) -> Result<Option<RemoteConfig>> {
    match name {
        Some(n) => {
            let remote = config.get_remote(n).with_context(|| {
                format!(
                    "remote '{}' not found in gmx config. Add it with: gmx config remote {} <host>",
                    n, n
                )
            })?;
            Ok(Some(remote.clone()))
        }
        None => Ok(None),
    }
}

fn auto_session_name() -> String {
    // Use current directory name as session name, like tmux uses window names
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or_else(|| "gmx".to_string())
}

fn cmd_new(
    name: Option<&str>,
    remote_name: Option<&str>,
    dir: Option<&str>,
    tab: bool,
) -> Result<()> {
    let config = GmxConfig::load()?;
    let remote = resolve_remote(&config, remote_name)?;

    let name = match name {
        Some(n) => n.to_string(),
        None => auto_session_name(),
    };
    let name = &name;

    // Default working directory: current directory for local, or require --dir for remote
    let working_dir = match dir {
        Some(d) => d.to_string(),
        None => {
            if remote.is_some() {
                bail!("--dir is required for remote sessions");
            }
            std::env::current_dir()
                .context("failed to get current directory")?
                .to_string_lossy()
                .to_string()
        }
    };

    // Check if session already has live zmx sessions
    let existing = zmx::find_sessions_by_prefix(name, remote.as_ref())?;
    if !existing.is_empty() {
        bail!(
            "session '{}' already exists ({} zmx session(s)). Use 'gmx attach {}' to reattach.",
            name,
            existing.len(),
            name
        );
    }

    // Save to session registry
    let mut registry = SessionRegistry::load()?;
    registry.register(name, remote_name, &working_dir);
    registry.save()?;

    if tab {
        // Open in a new Ghostty tab
        let zmx_name = format!("{}.1", name);
        let command = session::build_zmx_command(&zmx_name, &working_dir, remote.as_ref());
        let env_vars: Vec<(&str, &str)> = vec![("GMX_SESSION", name), ("GMX_IDX", "1")];
        let local_wd = if remote.is_none() {
            Some(working_dir.as_str())
        } else {
            None
        };
        let terminal_id = ghostty::create_tab_with_config(&command, local_wd, &env_vars)?;
        eprintln!(
            "Created session '{}' in new tab (terminal {})",
            name,
            &terminal_id[..8.min(terminal_id.len())]
        );
    } else {
        // Attach zmx in the current terminal (exec, replaces current process).
        // zmx sessions persist after client disconnect (ctrl+\ to detach,
        // or closing the tab sends SIGHUP which detaches the client).
        let zmx_name = format!("{}.1", name);

        // Set GMX_SESSION so `gmx split` works from inside the session.
        // Safety: we're about to exec (replace the process), so no other threads are affected.
        unsafe {
            std::env::set_var("GMX_SESSION", name);
            std::env::set_var("GMX_IDX", "1");
        }
        eprintln!("Created session '{}'", name);
        zmx::exec_attach(&zmx_name, &working_dir)?;
    }
    Ok(())
}

fn cmd_attach(name: &str, tab: bool) -> Result<()> {
    let config = GmxConfig::load()?;
    let registry = SessionRegistry::load()?;

    // Look up session registry for remote/dir info
    let entry = registry.get(name).with_context(|| {
        format!(
            "session '{}' not found in registry. Create it with: gmx new {}",
            name, name
        )
    })?;

    let remote = match &entry.remote {
        Some(r) => resolve_remote(&config, Some(r))?,
        None => None,
    };

    // Find live zmx sessions for this group
    let sessions = zmx::find_sessions_by_prefix(name, remote.as_ref())?;
    if sessions.is_empty() {
        bail!(
            "no live zmx sessions for '{}'. Create with: gmx new {}",
            name,
            name
        );
    }

    let dir = &entry.dir;

    if tab {
        // Recreate full layout in new Ghostty tabs/splits
        let split_dir = &config.default_split_direction;
        let mut prev_terminal_id: Option<String> = None;

        for (i, zmx_session) in sessions.iter().enumerate() {
            let command = session::build_zmx_command(&zmx_session.name, dir, remote.as_ref());
            let idx_str = format!("{}", i + 1);
            let env_vars: Vec<(&str, &str)> = vec![("GMX_SESSION", name), ("GMX_IDX", &idx_str)];
            let local_wd = if remote.is_none() {
                Some(dir.as_str())
            } else {
                None
            };

            let terminal_id = if let Some(prev_id) = &prev_terminal_id {
                ghostty::split_with_config(prev_id, split_dir, &command, local_wd, &env_vars)?
            } else {
                ghostty::create_tab_with_config(&command, local_wd, &env_vars)?
            };

            prev_terminal_id = Some(terminal_id);
        }
        eprintln!("Attached to '{}' ({} terminal(s))", name, sessions.len());
    } else {
        // Attach in current terminal: open extra panes as Ghostty splits first,
        // then exec into the first session in this terminal.
        if sessions.len() > 1 {
            let split_dir = &config.default_split_direction;
            let current_tid = ghostty::focused_terminal_id()?;
            let mut prev_tid = current_tid.clone();

            for (i, zmx_session) in sessions.iter().skip(1).enumerate() {
                let command = session::build_zmx_command(&zmx_session.name, dir, remote.as_ref());
                let idx_str = format!("{}", i + 2);
                let env_vars: Vec<(&str, &str)> =
                    vec![("GMX_SESSION", name), ("GMX_IDX", &idx_str)];
                let local_wd = if remote.is_none() {
                    Some(dir.as_str())
                } else {
                    None
                };
                prev_tid = ghostty::split_with_config(
                    &prev_tid, split_dir, &command, local_wd, &env_vars,
                )?;
            }
            // Focus back to the original terminal so exec happens here
            ghostty::focus_terminal(&current_tid)?;
        }

        let first = &sessions[0];
        unsafe {
            std::env::set_var("GMX_SESSION", name);
            std::env::set_var("GMX_IDX", "1");
        }
        eprintln!("Attaching to '{}' ({} pane(s))", name, sessions.len());
        zmx::exec_attach_only(&first.name)?;
    }

    eprintln!("Attached to '{}' ({} terminal(s))", name, sessions.len());
    Ok(())
}

fn cmd_kill(name: Option<&str>) -> Result<()> {
    // Default to current session if no name given
    let env_session = std::env::var("GMX_SESSION").ok();
    let name = match name {
        Some(n) => n.to_string(),
        None => env_session
            .context("no session name given and not inside a gmx session (GMX_SESSION not set)")?,
    };
    let name = &name;
    let config = GmxConfig::load()?;
    let registry = SessionRegistry::load()?;

    let remote = match registry.get(name) {
        Some(entry) => match &entry.remote {
            Some(r) => resolve_remote(&config, Some(r))?,
            None => None,
        },
        None => None,
    };

    let sessions = zmx::find_sessions_by_prefix(name, remote.as_ref())?;
    if sessions.is_empty() {
        // Still remove from registry if present
        let mut registry = registry;
        registry.remove(name);
        registry.save()?;
        eprintln!(
            "No live zmx sessions for '{}' (removed from registry)",
            name
        );
        return Ok(());
    }

    for s in &sessions {
        zmx::kill_session(&s.name, remote.as_ref())?;
    }

    let mut registry = registry;
    registry.remove(name);
    registry.save()?;

    eprintln!("Killed '{}' ({} zmx session(s))", name, sessions.len());
    Ok(())
}

fn cmd_detach() -> Result<()> {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new("zmx").args(["detach"]).exec();
    Err(err).context("failed to exec zmx detach")
}

fn cmd_ls() -> Result<()> {
    let registry = SessionRegistry::load()?;
    let config = GmxConfig::load()?;

    // Discover ALL gmx sessions from zmx directly (not just registry)
    let all_zmx = zmx::list_sessions(None).unwrap_or_default();

    // Group zmx sessions by gmx session name (strip .N suffix)
    let mut discovered: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for s in &all_zmx {
        let group = session::extract_gmx_session_name(&s.name);
        *discovered.entry(group).or_default() += 1;
    }

    // Merge registry entries (for remote/dir info) with discovered sessions
    // Also include registry entries for dead sessions
    for (name, entry) in &registry.sessions {
        if !discovered.contains_key(name.as_str()) {
            let remote = match &entry.remote {
                Some(r) => resolve_remote(&config, Some(r)).ok().flatten(),
                None => None,
            };
            let remote_count = if remote.is_some() {
                zmx::find_sessions_by_prefix(name, remote.as_ref())
                    .unwrap_or_default()
                    .len()
            } else {
                0
            };
            discovered.insert(name.clone(), remote_count);
        }
    }

    if discovered.is_empty() {
        eprintln!("No sessions. Create one with: gmx new <name>");
        return Ok(());
    }

    println!("{:<20} {:<12} {:<8} DIR", "SESSION", "REMOTE", "PANES");

    for (name, pane_count) in &discovered {
        let (remote_label, dir) = match registry.get(name) {
            Some(entry) => (
                entry.remote.as_deref().unwrap_or("local"),
                entry.dir.as_str(),
            ),
            None => ("local", "-"),
        };

        let status = if *pane_count > 0 {
            format!("{}", pane_count)
        } else {
            "dead".to_string()
        };

        println!("{:<20} {:<12} {:<8} {}", name, remote_label, status, dir);
    }

    Ok(())
}

fn cmd_split(direction: &str) -> Result<()> {
    // Read GMX_SESSION from env to know which session we're in
    let session_name = std::env::var("GMX_SESSION").context(
        "not inside a gmx session (GMX_SESSION not set). Use 'gmx new' to create a session first.",
    )?;

    let config = GmxConfig::load()?;
    let registry = SessionRegistry::load()?;

    let entry = registry
        .get(&session_name)
        .with_context(|| format!("session '{}' not found in registry", session_name))?;

    let remote = match &entry.remote {
        Some(r) => resolve_remote(&config, Some(r))?,
        None => None,
    };

    // Find next available zmx session index
    let zmx_name = zmx::next_session_name_from_base(&session_name, remote.as_ref())?;

    // Get the focused terminal ID (the one we're splitting)
    let terminal_id = ghostty::focused_terminal_id()?;

    // Build command
    let command = session::build_zmx_command(&zmx_name, &entry.dir, remote.as_ref());

    // Extract index from zmx_name for env var
    let idx = zmx_name
        .strip_prefix(&format!("{}.", session_name))
        .unwrap_or("1");
    let env_vars: Vec<(&str, &str)> = vec![("GMX_SESSION", &session_name), ("GMX_IDX", idx)];

    let local_wd = if remote.is_none() {
        Some(entry.dir.as_str())
    } else {
        None
    };

    let new_id =
        ghostty::split_with_config(&terminal_id, direction, &command, local_wd, &env_vars)?;

    eprintln!(
        "Split {} ({}) -> {}",
        session_name,
        direction,
        &new_id[..8.min(new_id.len())]
    );
    Ok(())
}

fn cmd_rename(old: &str, new: &str) -> Result<()> {
    let config = GmxConfig::load()?;
    let mut registry = SessionRegistry::load()?;

    let entry = registry
        .get(old)
        .with_context(|| format!("session '{}' not found", old))?
        .clone();

    let remote = match &entry.remote {
        Some(r) => resolve_remote(&config, Some(r))?,
        None => None,
    };

    // Kill old zmx sessions (zmx has no rename, so we kill and let attach recreate)
    let old_sessions = zmx::find_sessions_by_prefix(old, remote.as_ref())?;
    for s in &old_sessions {
        let _ = zmx::kill_session(&s.name, remote.as_ref());
    }

    // Update registry: remove old, insert new with same config
    registry.remove(old);
    registry.sessions.insert(new.to_string(), entry);
    registry.save()?;

    if old_sessions.is_empty() {
        eprintln!("Renamed '{}' -> '{}'", old, new);
    } else {
        eprintln!(
            "Renamed '{}' -> '{}' (killed {} zmx session(s), use 'gmx attach {}' to recreate)",
            old,
            new,
            old_sessions.len(),
            new
        );
    }
    Ok(())
}

fn cmd_config(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Remote {
            name,
            host,
            user,
            transport,
        } => {
            let mut config = GmxConfig::load()?;
            config.remotes.insert(
                name.clone(),
                RemoteConfig {
                    host: host.clone(),
                    user,
                    scan_dirs: vec![],
                    transport,
                },
            );
            config.save()?;
            eprintln!("Configured remote '{}' -> {}", name, host);
        }
        ConfigCommands::Show => {
            let config = GmxConfig::load()?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
    }
    Ok(())
}

// --- Keybinds ---

const GMX_KEYBINDS_MARKER: &str = "# gmx keybindings";
const GMX_KEYBINDS_END: &str = "# end gmx keybindings";

fn ghostty_config_path() -> Result<std::path::PathBuf> {
    // macOS: ~/Library/Application Support/com.mitchellh.ghostty/config
    // Linux: ~/.config/ghostty/config
    if cfg!(target_os = "macos") {
        let dir = dirs::home_dir().context("no home directory")?;
        let path = dir.join("Library/Application Support/com.mitchellh.ghostty/config");
        if path.exists() {
            return Ok(path);
        }
    }
    let dir = dirs::config_dir().context("no config directory")?;
    Ok(dir.join("ghostty").join("config"))
}

fn generate_keybinds(prefix: Option<&str>) -> String {
    match prefix {
        Some(pfx) => {
            // tmux-style: prefix key activates a key table, then single key triggers action
            format!(
                r#"{marker}
# Prefix mode: press {pfx}, then a key to trigger gmx actions
keybind = {pfx}=activate_key_table_once:gmx
keybind = gmx/d=text:gmx split right\n
keybind = gmx/e=text:gmx split down\n
keybind = gmx/c=text:gmx new --tab\n
keybind = gmx/a=text:gmx attach
keybind = gmx/x=text:gmx kill\n
keybind = gmx/s=text:gmx ls\n
keybind = gmx/escape=deactivate_key_table
{end}"#,
                marker = GMX_KEYBINDS_MARKER,
                end = GMX_KEYBINDS_END,
                pfx = pfx,
            )
        }
        None => {
            // Direct ctrl+shift bindings
            format!(
                r#"{marker}
keybind = ctrl+shift+d=text:gmx split right\n
keybind = ctrl+shift+e=text:gmx split down\n
keybind = ctrl+shift+t=text:gmx new --tab\n
keybind = ctrl+shift+a=text:gmx attach
keybind = ctrl+shift+x=text:gmx kill\n
keybind = ctrl+shift+s=text:gmx ls\n
{end}"#,
                marker = GMX_KEYBINDS_MARKER,
                end = GMX_KEYBINDS_END,
            )
        }
    }
}

fn cmd_keybinds(command: KeybindsCommands) -> Result<()> {
    match command {
        KeybindsCommands::Show { prefix } => {
            println!("{}", generate_keybinds(prefix.as_deref()));
            Ok(())
        }
        KeybindsCommands::Install { prefix } => {
            let path = ghostty_config_path()?;
            let content = if path.exists() {
                std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?
            } else {
                String::new()
            };

            // Remove existing gmx keybinds if present
            let cleaned = remove_gmx_block(&content);
            let keybinds = generate_keybinds(prefix.as_deref());
            let new_content = format!("{}\n{}\n", cleaned.trim_end(), keybinds);

            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, new_content)
                .with_context(|| format!("failed to write {}", path.display()))?;

            eprintln!("Installed gmx keybindings to {}", path.display());
            eprintln!("Reload Ghostty config (or restart) to apply.");
            Ok(())
        }
        KeybindsCommands::Uninstall => {
            let path = ghostty_config_path()?;
            if !path.exists() {
                eprintln!("No Ghostty config found at {}", path.display());
                return Ok(());
            }

            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;

            if !content.contains(GMX_KEYBINDS_MARKER) {
                eprintln!("No gmx keybindings found in {}", path.display());
                return Ok(());
            }

            let cleaned = remove_gmx_block(&content);
            std::fs::write(&path, cleaned.trim_end().to_string() + "\n")
                .with_context(|| format!("failed to write {}", path.display()))?;

            eprintln!("Removed gmx keybindings from {}", path.display());
            eprintln!("Reload Ghostty config (or restart) to apply.");
            Ok(())
        }
    }
}

fn remove_gmx_block(content: &str) -> String {
    let mut result = String::new();
    let mut skipping = false;
    for line in content.lines() {
        if line.trim() == GMX_KEYBINDS_MARKER {
            skipping = true;
            continue;
        }
        if line.trim() == GMX_KEYBINDS_END {
            skipping = false;
            continue;
        }
        if !skipping {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}
