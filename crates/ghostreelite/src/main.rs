mod config;
mod discovery;
mod display;
mod logic;
mod worktree;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;
use crossterm::event::{KeyCode, KeyModifiers};
use skim::prelude::*;
use std::io::Cursor;

use crate::worktree::Worktree;
use config::{Config, RemoteConfig, RepoConfig};
use discovery::DiscoveredRepo;
use display::{WorktreeDisplay, print_worktree_table};
use ghostty_lib::ghostty;
use ghostty_lib::state::TerminalState;
use ghostty_lib::zmx::{self, ZmxSession};

#[derive(Parser)]
#[command(
    name = "ghostreelite",
    about = "Worktree manager with zmx sessions + Ghostty splits"
)]
struct Cli {
    /// Force a session backend: ghostty, zmx, or auto
    #[arg(long, default_value = "auto")]
    mode: String,

    /// Override remote host
    #[arg(long)]
    remote: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List all worktrees and session status
    List,
    /// Open a worktree (new tab in Ghostty or attach zmx session)
    Open {
        /// Branch name to open
        branch: String,
        /// Open as split instead of tab (right or down). Targets the worktree's existing tab.
        #[arg(long)]
        split: Option<String>,
        /// Create a new zmx session instead of reusing existing
        #[arg(long)]
        new_session: bool,
        /// Repo path (if branch exists in multiple repos)
        #[arg(long)]
        repo: Option<String>,
    },
    /// Create a new worktree and open it
    New {
        /// Branch name
        branch: String,
        /// Repo to create in
        #[arg(long)]
        repo: Option<String>,
        /// Base branch
        #[arg(long)]
        base: Option<String>,
        /// Open as split instead of tab (right or down)
        #[arg(long)]
        split: Option<String>,
    },
    /// Remove a worktree and its session
    Remove {
        /// Branch name
        branch: String,
        /// Repo path
        #[arg(long)]
        repo: Option<String>,
        /// Force removal
        #[arg(long)]
        force: bool,
    },
    /// List active sessions for managed worktrees
    Sessions,
    /// Manage repos
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    /// Configure settings
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand)]
enum RepoCommands {
    /// Add a repo to manage
    Add {
        /// Path to the repo (on the target machine)
        path: String,
        /// This repo is on a remote host
        #[arg(long)]
        remote: Option<String>,
    },
    /// List managed repos
    List,
    /// Remove a repo
    Remove { path: String },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set a config value
    Set {
        /// Key (e.g., remote.host, remote.user, remote.transport, remote.scan_dirs)
        key: String,
        /// Value
        value: String,
    },
    /// Show current config
    Show,
}

fn config_dir() -> Result<std::path::PathBuf> {
    Config::config_dir()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config = Config::load()?;

    // Apply remote override
    if let Some(host) = &cli.remote {
        config.remote = Some(RemoteConfig {
            host: host.clone(),
            user: None,
            scan_dirs: vec![],
            transport: "ssh".to_string(),
        });
    }

    let use_ghostty = match cli.mode.as_str() {
        "ghostty" => true,
        "zmx" => false,
        _ => ghostty::is_available(),
    };

    match cli.command {
        None => run_picker(&config, use_ghostty),
        Some(Commands::List) => cmd_list(&config),
        Some(Commands::Open {
            branch,
            split,
            new_session,
            repo,
        }) => cmd_open(
            &config,
            &branch,
            split.as_deref(),
            repo.as_deref(),
            new_session,
            use_ghostty,
        ),
        Some(Commands::New {
            branch,
            repo,
            base,
            split,
        }) => cmd_new(
            &config,
            &branch,
            repo.as_deref(),
            base.as_deref(),
            split.as_deref(),
            use_ghostty,
        ),
        Some(Commands::Remove {
            branch,
            repo,
            force,
        }) => cmd_remove(&config, &branch, repo.as_deref(), force),
        Some(Commands::Sessions) => cmd_sessions(&config),
        Some(Commands::Repo { command }) => cmd_repo(&mut config, command),
        Some(Commands::Config { command }) => cmd_config(&mut config, command),
    }
}

// --- Gathered state ---

struct GatheredState {
    /// (discovered_repo, repo_name, worktrees)
    repos: Vec<(DiscoveredRepo, String, Vec<Worktree>)>,
    /// Local zmx sessions
    local_sessions: Vec<ZmxSession>,
    /// Remote zmx sessions (if remote configured)
    remote_sessions: Vec<ZmxSession>,
}

impl GatheredState {
    fn sessions_for_env(&self, env_name: &str) -> &[ZmxSession] {
        if env_name == "local" {
            &self.local_sessions
        } else {
            &self.remote_sessions
        }
    }
}

fn gather_state(config: &Config) -> Result<GatheredState> {
    let discovered = discovery::discover_repos(config)?;

    // Separate local and remote repos for batch processing
    let mut local_repos: Vec<DiscoveredRepo> = Vec::new();
    let mut remote_repos: Vec<DiscoveredRepo> = Vec::new();
    for dr in discovered {
        if dr.remote.is_some() {
            remote_repos.push(dr);
        } else {
            local_repos.push(dr);
        }
    }

    let mut repos = Vec::new();

    // Local repos: batch using git directly (fast, no network)
    if !local_repos.is_empty() {
        let paths: Vec<String> = local_repos
            .iter()
            .map(|dr| dr.config.path.clone())
            .collect();
        match worktree::list_worktrees_batch_fast(&paths, None) {
            Ok(batch_result) => {
                for dr in local_repos {
                    let repo_name = zmx::repo_name_from_path(&dr.config.path);
                    if let Some(wts) = batch_result.get(&dr.config.path) {
                        repos.push((dr, repo_name, wts.clone()));
                    }
                }
            }
            Err(e) => eprintln!("{}: local batch listing failed: {}", "warning".yellow(), e),
        }
    }

    // Remote repos: batch using git directly over SSH (single SSH call)
    if !remote_repos.is_empty() {
        let paths: Vec<String> = remote_repos
            .iter()
            .map(|dr| dr.config.path.clone())
            .collect();
        match worktree::list_worktrees_batch_fast(&paths, config.remote.as_ref()) {
            Ok(batch_result) => {
                for dr in remote_repos {
                    let repo_name = zmx::repo_name_from_path(&dr.config.path);
                    if let Some(wts) = batch_result.get(&dr.config.path) {
                        repos.push((dr, repo_name, wts.clone()));
                    }
                }
            }
            Err(e) => eprintln!("{}: remote batch listing failed: {}", "warning".yellow(), e),
        }
    }

    // Gather local zmx sessions
    let local_sessions = zmx::list_sessions(None).unwrap_or_default();

    // Gather remote zmx sessions
    let remote_sessions = match &config.remote {
        Some(r) => zmx::list_sessions(Some(r)).unwrap_or_default(),
        None => vec![],
    };

    Ok(GatheredState {
        repos,
        local_sessions,
        remote_sessions,
    })
}

fn find_session_for_worktree<'a>(
    sessions: &'a [ZmxSession],
    repo_name: &str,
    branch: &str,
) -> Option<&'a ZmxSession> {
    logic::find_session_for_worktree(sessions, repo_name, branch)
}

// --- Commands ---

fn load_terminal_state() -> Result<TerminalState> {
    let dir = config_dir()?;
    TerminalState::load(&dir)
}

fn save_terminal_state(state: &TerminalState) -> Result<()> {
    let dir = config_dir()?;
    state.save(&dir)
}

fn cmd_list(config: &Config) -> Result<()> {
    let state = gather_state(config)?;

    let mut entries: Vec<WorktreeDisplay> = Vec::new();
    for (dr, repo_name, wts) in &state.repos {
        let sessions = state.sessions_for_env(&dr.env_name);
        for wt in wts {
            let session = find_session_for_worktree(sessions, repo_name, &wt.branch);
            entries.push(WorktreeDisplay {
                env_name: &dr.env_name,
                repo_name,
                worktree: wt,
                session,
            });
        }
    }

    entries.sort_by_key(|e| e.sort_key());
    print_worktree_table(&entries);
    Ok(())
}

fn cmd_open(
    config: &Config,
    branch: &str,
    split: Option<&str>,
    repo_filter: Option<&str>,
    new_session: bool,
    use_ghostty: bool,
) -> Result<()> {
    let (dr, repo_name, wt) = resolve_worktree(config, branch, repo_filter)?;
    let remote = dr.remote.as_ref();
    let mut term_state = load_terminal_state()?;

    if new_session {
        let session_name = zmx::next_session_name(&repo_name, branch, remote)?;
        if use_ghostty {
            // For splits with --new-session, target the worktree's existing tab
            if let Some(dir) = split {
                if let Some((_sess, tid)) = term_state.find_valid_terminal(&repo_name, branch) {
                    println!(
                        "New session {} as split ({}) in existing tab",
                        session_name.bold(),
                        dir.dimmed()
                    );
                    let new_id = ghostty::split_at(&tid, &session_name, &wt.path, dir, remote)?;
                    term_state.add(&session_name, &new_id);
                    save_terminal_state(&term_state)?;
                } else {
                    println!(
                        "New session {} in tab (no existing tab found)",
                        session_name.bold()
                    );
                    let id = ghostty::open(&session_name, &wt.path, None, remote)?;
                    term_state.add(&session_name, &id);
                    save_terminal_state(&term_state)?;
                }
            } else {
                println!("New session {} in tab", session_name.bold());
                let id = ghostty::open(&session_name, &wt.path, None, remote)?;
                term_state.add(&session_name, &id);
                save_terminal_state(&term_state)?;
            }
        } else {
            println!("Attaching to new zmx session {}", session_name.bold());
            zmx::exec_attach(&session_name, &wt.path)?;
        }
    } else if use_ghostty {
        // Reattach all existing sessions for this worktree, or create one if none exist
        let existing = zmx::find_worktree_sessions(&repo_name, branch, remote)?;
        if existing.is_empty() {
            let session_name = zmx::session_name(&repo_name, branch);
            println!(
                "Opening {} in new Ghostty tab [{}]",
                branch.bold(),
                session_name.dimmed()
            );
            let id = ghostty::open(&session_name, &wt.path, None, remote)?;
            term_state.add(&session_name, &id);
            save_terminal_state(&term_state)?;
        } else {
            // For each existing session, check if we already have a live terminal.
            // If so, focus it. If not, open a new tab/split.
            let names: Vec<String> = existing.iter().map(|s| s.name.clone()).collect();
            println!("Reopening {} session(s) for {}", names.len(), branch.bold());
            let mut first = true;
            for name in &names {
                let existing_ids = term_state.get(name);
                let has_live = existing_ids.iter().any(|id| ghostty::terminal_exists(id));
                if has_live {
                    // Focus the existing terminal
                    if let Some(id) = existing_ids.iter().find(|id| ghostty::terminal_exists(id)) {
                        ghostty::focus_terminal(id)?;
                    }
                } else if first {
                    let id = ghostty::open(name, &wt.path, None, remote)?;
                    term_state.add(name, &id);
                    first = false;
                } else {
                    let id = ghostty::open(name, &wt.path, Some("right"), remote)?;
                    term_state.add(name, &id);
                }
            }
            save_terminal_state(&term_state)?;
        }
    } else {
        let session_name = zmx::session_name(&repo_name, branch);
        println!("Attaching to zmx session {}", session_name.bold());
        zmx::exec_attach(&session_name, &wt.path)?;
    }

    Ok(())
}

fn cmd_new(
    config: &Config,
    branch: &str,
    repo_filter: Option<&str>,
    base: Option<&str>,
    split: Option<&str>,
    use_ghostty: bool,
) -> Result<()> {
    let dr = resolve_repo(config, repo_filter)?;
    let remote = dr.remote.as_ref();
    let repo_name = zmx::repo_name_from_path(&dr.config.path);

    println!("Creating worktree {}", branch.bold());
    worktree::create_worktree(&dr.config.path, branch, base, remote)?;

    let wt = worktree::find_worktree(&dr.config.path, branch, remote)?
        .context("worktree created but not found in listing")?;

    let session_name = zmx::session_name(&repo_name, branch);

    if use_ghostty {
        match split {
            Some(dir) => println!(
                "Opening {} in Ghostty split ({})",
                branch.bold(),
                dir.dimmed()
            ),
            None => println!("Opening {} in new Ghostty tab", branch.bold()),
        }
        let id = ghostty::open(&session_name, &wt.path, split, remote)?;
        let mut term_state = load_terminal_state()?;
        term_state.add(&session_name, &id);
        save_terminal_state(&term_state)?;
    } else {
        println!("Attaching to zmx session {}", session_name.bold());
        zmx::exec_attach(&session_name, &wt.path)?;
    }

    Ok(())
}

fn cmd_remove(config: &Config, branch: &str, repo_filter: Option<&str>, force: bool) -> Result<()> {
    let (dr, repo_name, _wt) = resolve_worktree(config, branch, repo_filter)?;
    let remote = dr.remote.as_ref();
    let session_name = zmx::session_name(&repo_name, branch);

    if zmx::find_session(&session_name, remote)?.is_some() {
        println!("Killing zmx session {}", session_name.dimmed());
        zmx::kill_session(&session_name, remote)?;
    }

    // Clean up terminal state
    let mut term_state = load_terminal_state()?;
    term_state.remove_session(&session_name);
    save_terminal_state(&term_state)?;

    println!("Removing worktree {}", branch.bold());
    worktree::remove_worktree(&dr.config.path, branch, force, remote)?;

    println!("{} Removed {}", "✓".green(), branch);
    Ok(())
}

fn cmd_sessions(config: &Config) -> Result<()> {
    let state = gather_state(config)?;

    let all_sessions: Vec<(&str, &ZmxSession)> = state
        .local_sessions
        .iter()
        .map(|s| ("local", s))
        .chain(state.remote_sessions.iter().map(|s| {
            let name = config
                .remote
                .as_ref()
                .map(|r| r.host.as_str())
                .unwrap_or("remote");
            (name, s)
        }))
        .collect();

    if all_sessions.is_empty() {
        println!("{}", "No active zmx sessions".dimmed());
        return Ok(());
    }

    println!(
        "  {:<10} {:<30} {:<10} {}",
        "ENV".dimmed(),
        "SESSION".dimmed(),
        "CLIENTS".dimmed(),
        "STARTED IN".dimmed(),
    );

    for (env, session) in &all_sessions {
        let status = if session.is_attached() {
            format!("{} {}", "●".green(), session.clients)
        } else {
            format!("{} {}", "◌".yellow(), session.clients)
        };

        println!(
            "  {:<10} {:<30} {:<10} {}",
            env,
            session.name,
            status,
            session.started_in.as_deref().unwrap_or("-").dimmed(),
        );
    }

    Ok(())
}

fn cmd_repo(config: &mut Config, command: RepoCommands) -> Result<()> {
    match command {
        RepoCommands::Add { path, remote } => {
            let path = if !path.starts_with('/') && remote.is_none() && config.remote.is_none() {
                std::fs::canonicalize(&path)
                    .with_context(|| format!("path not found: {}", path))?
                    .to_string_lossy()
                    .to_string()
            } else {
                path
            };

            if config.repos.iter().any(|r| r.path == path) {
                println!("Repo already tracked: {}", path);
                return Ok(());
            }

            let repo_remote = remote.map(|host| RemoteConfig {
                host,
                user: None,
                scan_dirs: vec![],
                transport: "ssh".to_string(),
            });

            config.repos.push(RepoConfig {
                path: path.clone(),
                remote: repo_remote,
            });
            config.save()?;
            println!("Added repo: {}", path.bold());
        }
        RepoCommands::List => {
            if config.repos.is_empty() {
                println!(
                    "{}",
                    "No explicit repos. Using scan_dirs for discovery.".dimmed()
                );
            }
            for repo in &config.repos {
                let remote_label = match config.effective_remote(repo) {
                    Some(r) => format!(" ({})", r.ssh_target().dimmed()),
                    None => " (local)".dimmed().to_string(),
                };
                println!("  {}{}", repo.path, remote_label);
            }
        }
        RepoCommands::Remove { path } => {
            let before = config.repos.len();
            config.repos.retain(|r| r.path != path);
            if config.repos.len() == before {
                bail!("repo not found: {}", path);
            }
            config.save()?;
            println!("Removed repo: {}", path);
        }
    }
    Ok(())
}

fn cmd_config(config: &mut Config, command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Set { key, value } => {
            match key.as_str() {
                "remote.host" => {
                    let remote = config.remote.get_or_insert(RemoteConfig {
                        host: String::new(),
                        user: None,
                        scan_dirs: vec![],
                        transport: "ssh".to_string(),
                    });
                    remote.host = value.clone();
                }
                "remote.user" => {
                    let remote = config.remote.get_or_insert(RemoteConfig {
                        host: String::new(),
                        user: None,
                        scan_dirs: vec![],
                        transport: "ssh".to_string(),
                    });
                    remote.user = Some(value.clone());
                }
                "remote.transport" => {
                    let remote = config.remote.get_or_insert(RemoteConfig {
                        host: String::new(),
                        user: None,
                        scan_dirs: vec![],
                        transport: "ssh".to_string(),
                    });
                    remote.transport = value.clone();
                }
                "remote.scan_dirs" => {
                    let remote = config.remote.get_or_insert(RemoteConfig {
                        host: String::new(),
                        user: None,
                        scan_dirs: vec![],
                        transport: "ssh".to_string(),
                    });
                    remote.scan_dirs = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "split_direction" => {
                    config.split_direction = value.clone();
                }
                "default_agent" => {
                    config.default_agent = Some(value.clone());
                }
                "default_shell" => {
                    config.default_shell = Some(value.clone());
                }
                "scan_dirs" => {
                    config.scan_dirs = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                _ => bail!("unknown config key: {}", key),
            }
            config.save()?;
            println!("Set {} = {}", key, value.bold());
        }
        ConfigCommands::Show => {
            println!("{}", serde_json::to_string_pretty(config)?);
        }
    }
    Ok(())
}

// --- Picker ---

fn run_picker(config: &Config, use_ghostty: bool) -> Result<()> {
    let state = gather_state(config)?;

    if state.repos.is_empty() {
        println!(
            "{}",
            "No repos found. Set scan_dirs or add repos explicitly.".dimmed()
        );
        return Ok(());
    }

    // Build picker items: (sort_key, display, env_name, repo_path, branch, remote)
    #[allow(clippy::type_complexity)]
    let mut items: Vec<(
        (u8, u64),
        String,
        String,
        String,
        String,
        Option<RemoteConfig>,
    )> = Vec::new();

    for (dr, repo_name, wts) in &state.repos {
        let sessions = state.sessions_for_env(&dr.env_name);
        for wt in wts {
            let session = find_session_for_worktree(sessions, repo_name, &wt.branch);
            let disp = WorktreeDisplay {
                env_name: &dr.env_name,
                repo_name,
                worktree: wt,
                session,
            };
            let sort_key = disp.sort_key();
            let display_line = disp.picker_line();
            items.push((
                sort_key,
                display_line,
                dr.env_name.clone(),
                dr.config.path.clone(),
                wt.branch.clone(),
                dr.remote.clone(),
            ));
        }
    }

    items.sort_by_key(|(k, _, _, _, _, _)| *k);

    if items.is_empty() {
        println!("{}", "No worktrees found across configured repos.".dimmed());
        return Ok(());
    }

    let input = items
        .iter()
        .map(|(_, display, _, _, _, _)| display.clone())
        .collect::<Vec<_>>()
        .join("\n");

    let options = SkimOptionsBuilder::default()
        .prompt("worktree> ".to_string())
        .header("  enter: open  ctrl-n: new session  ctrl-s: split  ctrl-v: vsplit  ctrl-a: new worktree  ctrl-d: remove  esc: quit".to_string())
        .multi(false)
        .bind(vec![
            "ctrl-a:accept".to_string(),
            "ctrl-n:accept".to_string(),
            "ctrl-s:accept".to_string(),
            "ctrl-v:accept".to_string(),
            "ctrl-d:accept".to_string(),
        ])
        .build()
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let item_reader = SkimItemReader::default();
    let skim_items = item_reader.of_bufread(Cursor::new(input));

    let output = Skim::run_with(options, Some(skim_items));

    match output {
        Ok(out) if !out.is_abort => {
            if let Some(selected) = out.selected_items.first() {
                let text = selected.item.output().to_string();
                let idx = items
                    .iter()
                    .position(|(_, display, _, _, _, _)| *display == text)
                    .context("failed to match picker selection")?;

                let (_, _, _env_name, repo_path, branch, remote) = &items[idx];
                let repo_name = zmx::repo_name_from_path(repo_path);

                let is_ctrl = out.final_key.modifiers.contains(KeyModifiers::CONTROL);
                let key_char = match out.final_key.code {
                    KeyCode::Char(c) => Some(c),
                    _ => None,
                };

                match logic::resolve_picker_action(is_ctrl, key_char) {
                    logic::PickerAction::NewWorktree => {
                        create_worktree_interactive(config, &state, use_ghostty)?;
                    }
                    logic::PickerAction::Remove => {
                        picker_remove(repo_path, branch, &repo_name, remote.as_ref())?;
                    }
                    logic::PickerAction::NewSession => {
                        let session_name =
                            zmx::next_session_name(&repo_name, branch, remote.as_ref())?;
                        if use_ghostty {
                            println!("New session {} in tab", session_name.bold());
                            let id = ghostty::open(
                                &session_name,
                                &worktree_path_for(repo_path, branch, remote.as_ref())?,
                                None,
                                remote.as_ref(),
                            )?;
                            let mut ts = load_terminal_state()?;
                            ts.add(&session_name, &id);
                            save_terminal_state(&ts)?;
                        }
                    }
                    logic::PickerAction::SplitRight => {
                        picker_split(
                            config,
                            &repo_name,
                            repo_path,
                            branch,
                            remote.as_ref(),
                            "right",
                            use_ghostty,
                        )?;
                    }
                    logic::PickerAction::SplitDown => {
                        picker_split(
                            config,
                            &repo_name,
                            repo_path,
                            branch,
                            remote.as_ref(),
                            "down",
                            use_ghostty,
                        )?;
                    }
                    logic::PickerAction::Open => {
                        picker_open(
                            config,
                            &repo_name,
                            repo_path,
                            branch,
                            remote.as_ref(),
                            use_ghostty,
                        )?;
                    }
                }
            }
        }
        Ok(_) | Err(_) => {}
    }

    Ok(())
}

/// Helper: get worktree path for a branch in a repo
fn worktree_path_for(
    repo_path: &str,
    branch: &str,
    remote: Option<&RemoteConfig>,
) -> Result<String> {
    let wt = worktree::find_worktree(repo_path, branch, remote)?
        .with_context(|| format!("worktree '{}' not found", branch))?;
    Ok(wt.path)
}

/// Picker: open/reattach all sessions for a worktree
fn picker_open(
    _config: &Config,
    repo_name: &str,
    repo_path: &str,
    branch: &str,
    remote: Option<&RemoteConfig>,
    use_ghostty: bool,
) -> Result<()> {
    if !use_ghostty {
        let session_name = zmx::session_name(repo_name, branch);
        let wt_path = worktree_path_for(repo_path, branch, remote)?;
        zmx::exec_attach(&session_name, &wt_path)?;
        return Ok(());
    }

    let wt_path = worktree_path_for(repo_path, branch, remote)?;
    let existing = zmx::find_worktree_sessions(repo_name, branch, remote)?;
    let mut term_state = load_terminal_state()?;

    if existing.is_empty() {
        let session_name = zmx::session_name(repo_name, branch);
        println!(
            "Opening {} in new tab [{}]",
            branch.bold(),
            session_name.dimmed()
        );
        let id = ghostty::open(&session_name, &wt_path, None, remote)?;
        term_state.add(&session_name, &id);
    } else {
        let names: Vec<String> = existing.iter().map(|s| s.name.clone()).collect();
        println!("Reopening {} session(s) for {}", names.len(), branch.bold());
        let mut first = true;
        for name in &names {
            let existing_ids = term_state.get(name);
            let live_id = existing_ids.iter().find(|id| ghostty::terminal_exists(id));
            if let Some(id) = live_id {
                ghostty::focus_terminal(id)?;
            } else if first {
                let id = ghostty::open(name, &wt_path, None, remote)?;
                term_state.add(name, &id);
                first = false;
            } else {
                let id = ghostty::open(name, &wt_path, Some("right"), remote)?;
                term_state.add(name, &id);
            }
        }
    }

    save_terminal_state(&term_state)?;
    Ok(())
}

/// Picker: split an existing worktree tab with a new session
fn picker_split(
    _config: &Config,
    repo_name: &str,
    repo_path: &str,
    branch: &str,
    remote: Option<&RemoteConfig>,
    direction: &str,
    use_ghostty: bool,
) -> Result<()> {
    if !use_ghostty {
        println!("Splits only available in Ghostty mode");
        return Ok(());
    }

    let wt_path = worktree_path_for(repo_path, branch, remote)?;
    let session_name = zmx::next_session_name(repo_name, branch, remote)?;
    let mut term_state = load_terminal_state()?;

    // Try to find an existing terminal for this worktree
    if let Some((_sess, tid)) = term_state.find_valid_terminal(repo_name, branch) {
        println!(
            "Splitting {} ({}) [{}]",
            branch.bold(),
            direction.dimmed(),
            session_name.dimmed()
        );
        let new_id = ghostty::split_at(&tid, &session_name, &wt_path, direction, remote)?;
        term_state.add(&session_name, &new_id);
    } else {
        // No existing tab found, create a new tab instead
        println!(
            "No existing tab for {}, opening new tab [{}]",
            branch.bold(),
            session_name.dimmed()
        );
        let id = ghostty::open(&session_name, &wt_path, None, remote)?;
        term_state.add(&session_name, &id);
    }

    save_terminal_state(&term_state)?;
    Ok(())
}

/// Picker: remove dialog
fn picker_remove(
    repo_path: &str,
    branch: &str,
    repo_name: &str,
    remote: Option<&RemoteConfig>,
) -> Result<()> {
    use std::io::Write;

    let sessions = zmx::find_worktree_sessions(repo_name, branch, remote)?;
    let has_sessions = !sessions.is_empty();

    println!("What to do with {}?", branch.bold());
    if has_sessions {
        println!("  1) Kill zmx session(s) only {}", "(default)".dimmed());
        println!("  2) Kill session(s) + remove worktree");
    } else {
        println!("  1) Remove worktree {}", "(default)".dimmed());
    }
    println!("  q) Cancel");
    print!("> ");
    std::io::stdout().flush()?;

    let mut input_buf = String::new();
    std::io::stdin().read_line(&mut input_buf)?;
    let choice = input_buf.trim();

    // Clean up terminal state for killed sessions
    let mut term_state = load_terminal_state()?;

    if has_sessions {
        match choice {
            "" | "1" => {
                for s in &sessions {
                    zmx::kill_session(&s.name, remote)?;
                    term_state.remove_session(&s.name);
                }
                save_terminal_state(&term_state)?;
                println!("{} Killed {} session(s)", "✓".green(), sessions.len());
            }
            "2" => {
                for s in &sessions {
                    zmx::kill_session(&s.name, remote)?;
                    term_state.remove_session(&s.name);
                }
                worktree::remove_worktree(repo_path, branch, false, remote)?;
                save_terminal_state(&term_state)?;
                println!(
                    "{} Removed {} and {} session(s)",
                    "✓".green(),
                    branch,
                    sessions.len()
                );
            }
            _ => println!("Cancelled"),
        }
    } else {
        match choice {
            "" | "1" => {
                worktree::remove_worktree(repo_path, branch, false, remote)?;
                println!("{} Removed {}", "✓".green(), branch);
            }
            _ => println!("Cancelled"),
        }
    }

    Ok(())
}

// --- Interactive worktree creation ---

fn create_worktree_interactive(
    _config: &Config,
    state: &GatheredState,
    use_ghostty: bool,
) -> Result<()> {
    use std::io::Write;

    // Always show repo picker
    let repo_input = state
        .repos
        .iter()
        .map(|(dr, name, _)| format!("{}: {} ({})", dr.env_name, name, dr.config.path))
        .collect::<Vec<_>>()
        .join("\n");

    let repo_options = SkimOptionsBuilder::default()
        .prompt("repo> ".to_string())
        .header("Select repo for new worktree".to_string())
        .multi(false)
        .build()
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let item_reader = SkimItemReader::default();
    let skim_items = item_reader.of_bufread(Cursor::new(repo_input));
    let output = Skim::run_with(repo_options, Some(skim_items));

    let dr = match output {
        Ok(out) if !out.is_abort => {
            if let Some(selected) = out.selected_items.first() {
                let text = selected.item.output().to_string();
                let idx = state
                    .repos
                    .iter()
                    .position(|(dr, name, _)| {
                        format!("{}: {} ({})", dr.env_name, name, dr.config.path) == text
                    })
                    .context("failed to match repo selection")?;
                state.repos[idx].0.clone()
            } else {
                return Ok(());
            }
        }
        _ => return Ok(()),
    };

    print!("Branch name: ");
    std::io::stdout().flush()?;
    let mut branch = String::new();
    std::io::stdin().read_line(&mut branch)?;
    let branch = branch.trim();
    if branch.is_empty() {
        println!("Cancelled");
        return Ok(());
    }

    print!("Base branch (enter for default): ");
    std::io::stdout().flush()?;
    let mut base = String::new();
    std::io::stdin().read_line(&mut base)?;
    let base = base.trim();
    let base = if base.is_empty() { None } else { Some(base) };

    let remote = dr.remote.as_ref();
    let repo_name = zmx::repo_name_from_path(&dr.config.path);

    println!("Creating worktree {}", branch.bold());
    worktree::create_worktree(&dr.config.path, branch, base, remote)?;

    let wt = worktree::find_worktree(&dr.config.path, branch, remote)?
        .context("worktree created but not found")?;

    let session_name = zmx::session_name(&repo_name, branch);

    if use_ghostty {
        println!("Opening {} in new tab", branch.bold());
        let id = ghostty::open(&session_name, &wt.path, None, remote)?;
        let mut term_state = load_terminal_state()?;
        term_state.add(&session_name, &id);
        save_terminal_state(&term_state)?;
    } else {
        zmx::exec_attach(&session_name, &wt.path)?;
    }

    Ok(())
}

// --- Resolution helpers ---

fn resolve_worktree(
    config: &Config,
    branch: &str,
    repo_filter: Option<&str>,
) -> Result<(DiscoveredRepo, String, Worktree)> {
    let discovered = discovery::discover_repos(config)?;
    for dr in discovered {
        if let Some(filter) = repo_filter
            && dr.config.path != filter
            && !dr.config.path.ends_with(filter)
        {
            continue;
        }
        let repo_name = zmx::repo_name_from_path(&dr.config.path);
        if let Ok(Some(wt)) = worktree::find_worktree(&dr.config.path, branch, dr.remote.as_ref()) {
            return Ok((dr, repo_name, wt));
        }
    }

    bail!("worktree '{}' not found in any configured repo", branch);
}

fn resolve_repo(config: &Config, repo_filter: Option<&str>) -> Result<DiscoveredRepo> {
    let all = discovery::discover_repos(config)?;
    if let Some(filter) = repo_filter {
        all.into_iter()
            .find(|dr| dr.config.path == filter || dr.config.path.ends_with(filter))
            .with_context(|| format!("repo not found: {}", filter))
    } else if all.len() == 1 {
        Ok(all.into_iter().next().unwrap())
    } else if all.is_empty() {
        bail!("no repos configured. Set scan_dirs or add repos explicitly.");
    } else {
        bail!(
            "multiple repos found, specify with --repo. Repos: {}",
            all.iter()
                .map(|dr| dr.config.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}
