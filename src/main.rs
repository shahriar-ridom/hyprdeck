use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, stdout},
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[derive(PartialEq)]
enum InputMode {
    Normal,
    SnapshotName,
    ImportPath,
    ImportName,
    ConfirmDelete,
}

#[derive(Clone)]
struct Target {
    /// Name used in the profile directory.
    store_name: String,
    /// Destination relative to the user's home directory.
    home_rel_path: String,
    /// Category shown in the profile preview.
    category: &'static str,
}

struct App {
    profiles: Vec<String>,
    state: ListState,
    active_profile: String,
    status_msg: String,
    last_reload_summary: String,
    base_dir: PathBuf,
    home_dir: PathBuf,
    known_targets: Vec<Target>,
    input_mode: InputMode,
    input_buffer: String,
    pending_import_path: PathBuf,
}

impl App {
    fn new() -> Result<Self> {
        let home_dir = dirs::home_dir().context("Could not find home directory")?;
        let base_dir = home_dir.join(".hyprdeck");
        let profiles_dir = base_dir.join("profiles");

        fs::create_dir_all(&profiles_dir)?;

        let known_targets = Self::build_ecosystem_registry();

        let mut app = Self {
            profiles: Vec::new(),
            state: ListState::default(),
            active_profile: String::from("External / Unmanaged"),
            status_msg: String::from(
                "[Enter] Switch | [s] Snapshot | [i] Import | [d] Delete | [r] Reload Daemons | [q] Quit",
            ),
            last_reload_summary: String::from("No daemon reload triggered yet in this session."),
            base_dir,
            home_dir,
            known_targets,
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            pending_import_path: PathBuf::new(),
        };

        app.reload_profiles()?;
        app.detect_active_profile();
        Ok(app)
    }

    /// Map known profile entries to their live paths and display categories.
    fn build_ecosystem_registry() -> Vec<Target> {
        let mut reg = Vec::new();

        let mut add_config = |name: &str, cat: &'static str| {
            reg.push(Target {
                store_name: name.to_string(),
                home_rel_path: format!(".config/{}", name),
                category: cat,
            });
        };

        add_config("hypr", "Core WM");

        for shell in [
            "noctalia",
            "noctalia-shell",
            "caelestia",
            "calestia",
            "quickshell",
            "waybar",
            "ags",
            "eww",
            "ironbar",
            "hyprpanel",
        ] {
            add_config(shell, "Shell / Bar");
        }

        for term in ["kitty", "alacritty", "foot", "wezterm", "ghostty"] {
            add_config(term, "Terminal");
        }

        for launcher in [
            "rofi", "wofi", "fuzzel", "tofi", "anyrun", "walker", "wlogout",
        ] {
            add_config(launcher, "Launcher / Menu");
        }

        for util in [
            "wal",
            "dunst",
            "mako",
            "swaync",
            "swww",
            "wpaperd",
            "fastfetch",
            "btop",
            "cava",
            "matugen",
            "wallust",
        ] {
            add_config(util, "Theme / Utility");
        }

        reg.push(Target {
            store_name: "starship.toml".to_string(),
            home_rel_path: ".config/starship.toml".to_string(),
            category: "Prompt / Shell",
        });
        reg.push(Target {
            store_name: ".zshrc".to_string(),
            home_rel_path: ".zshrc".to_string(),
            category: "Prompt / Shell",
        });

        reg
    }

    fn resolve_target(&self, store_name: &str) -> Target {
        if let Some(found) = self
            .known_targets
            .iter()
            .find(|t| t.store_name == store_name)
        {
            return found.clone();
        }
        Target {
            store_name: store_name.to_string(),
            home_rel_path: format!(".config/{}", store_name),
            category: "Custom .config",
        }
    }

    fn get_profile_targets(&self, profile_name: &str) -> Vec<Target> {
        let profile_path = self.base_dir.join("profiles").join(profile_name);
        let mut names = BTreeSet::new();

        if let Ok(entries) = fs::read_dir(&profile_path) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name != "reload.sh" {
                        names.insert(name.to_string());
                    }
                }
            }
        }

        names.into_iter().map(|n| self.resolve_target(&n)).collect()
    }

    fn detect_active_profile(&mut self) {
        let hypr_target = self.home_dir.join(".config/hypr");
        let profiles_dir = self.base_dir.join("profiles");

        if let Ok(link_target) = fs::read_link(&hypr_target) {
            if let Ok(stripped) = link_target.strip_prefix(&profiles_dir) {
                if let Some(first_component) = stripped.components().next() {
                    if let Some(name) = first_component.as_os_str().to_str() {
                        self.active_profile = name.to_string();
                    }
                }
            }
        }
    }

    fn reload_profiles(&mut self) -> Result<()> {
        let profiles_dir = self.base_dir.join("profiles");
        let mut profiles = Vec::new();
        for entry in fs::read_dir(&profiles_dir)? {
            let entry = entry?;
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    profiles.push(name.to_string());
                }
            }
        }
        profiles.sort();
        self.profiles = profiles;

        if self.profiles.is_empty() {
            self.state.select(None);
        } else if let Some(selected) = self.state.selected() {
            if selected >= self.profiles.len() {
                self.state.select(Some(self.profiles.len() - 1));
            }
        } else {
            self.state.select(Some(0));
        }
        Ok(())
    }

    fn next(&mut self) {
        if self.profiles.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => (i + 1) % self.profiles.len(),
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.profiles.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.profiles.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn expand_tilde(&self, raw_path: &str) -> PathBuf {
        let trimmed = raw_path.trim();
        if trimmed == "~" {
            self.home_dir.clone()
        } else if let Some(stripped) = trimmed.strip_prefix("~/") {
            self.home_dir.join(stripped)
        } else {
            PathBuf::from(trimmed)
        }
    }

    fn prepare_import_path(&mut self) {
        let expanded = self.expand_tilde(&self.input_buffer);
        if !expanded.exists() || !expanded.is_dir() {
            self.input_mode = InputMode::Normal;
            self.status_msg = format!(
                "Import Error: Directory '{}' does not exist!",
                expanded.display()
            );
            return;
        }

        let default_name = expanded
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("imported-dots")
            .to_string();

        self.pending_import_path = expanded;
        self.input_buffer = default_name;
        self.input_mode = InputMode::ImportName;
        self.status_msg = String::from("Confirm or edit the profile name and press [Enter].");
    }

    fn execute_import(&mut self) -> Result<()> {
        let name = self.input_buffer.trim().to_string();
        if name.is_empty() {
            self.status_msg = String::from("Import cancelled: Profile name cannot be empty.");
            return Ok(());
        }

        if name.contains('/') || name.contains('\\') || name.contains("..") {
            self.status_msg = String::from("Error: Invalid characters in profile name.");
            return Ok(());
        }

        let new_profile_dir = self.base_dir.join("profiles").join(&name);
        if new_profile_dir.exists() {
            self.status_msg = format!("Error: Profile '{}' already exists!", name);
            return Ok(());
        }

        fs::create_dir_all(&new_profile_dir)?;

        let mut imported_items = BTreeSet::new();

        for target in &self.known_targets {
            let candidate_nested = self.pending_import_path.join(&target.home_rel_path);
            let candidate_flat = self.pending_import_path.join(&target.store_name);

            let found_source = if candidate_nested.exists() {
                Some(candidate_nested)
            } else if candidate_flat.exists() {
                Some(candidate_flat)
            } else {
                None
            };

            if let Some(src) = found_source {
                let target_dest = new_profile_dir.join(&target.store_name);
                if Self::copy_entry(&src, &target_dest, &new_profile_dir)? {
                    imported_items.insert(target.store_name.clone());
                }
            }
        }

        let dot_config = self.pending_import_path.join(".config");
        if dot_config.is_dir() {
            if let Ok(entries) = fs::read_dir(&dot_config) {
                for entry in entries.flatten() {
                    if let Some(folder_name) = entry.file_name().to_str() {
                        if !imported_items.contains(folder_name) && !folder_name.starts_with('.') {
                            let target_dest = new_profile_dir.join(folder_name);
                            if Self::copy_entry(&entry.path(), &target_dest, &new_profile_dir)? {
                                imported_items.insert(folder_name.to_string());
                            }
                        }
                    }
                }
            }
        }

        if imported_items.is_empty() {
            let _ = fs::remove_dir_all(&new_profile_dir);
            self.status_msg = format!(
                "Import failed: No recognizable dotfiles or .config items found in '{}'!",
                self.pending_import_path.display()
            );
            return Ok(());
        }

        self.reload_profiles()?;
        if let Some(idx) = self.profiles.iter().position(|p| *p == name) {
            self.state.select(Some(idx));
        }

        self.status_msg = format!(
            "Imported '{}' ({} modules detected)! Press [Enter] to activate.",
            name,
            imported_items.len()
        );
        Ok(())
    }

    fn copy_entry(src: &Path, target_dest: &Path, rollback_dir: &Path) -> Result<bool> {
        if let Ok(real_source) = fs::canonicalize(src) {
            if real_source.is_dir() {
                if let Err(e) = copy_dir_all(&real_source, target_dest, &real_source) {
                    let _ = fs::remove_dir_all(rollback_dir);
                    return Err(e.into());
                }
                return Ok(true);
            } else if real_source.is_file() {
                if let Err(e) = fs::copy(&real_source, target_dest) {
                    let _ = fs::remove_dir_all(rollback_dir);
                    return Err(e.into());
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn create_snapshot(&mut self) -> Result<()> {
        let name = self.input_buffer.trim().to_string();
        if name.is_empty() {
            self.status_msg = String::from("Snapshot cancelled: Name cannot be empty.");
            return Ok(());
        }

        if name.contains('/') || name.contains('\\') || name.contains("..") {
            self.status_msg = String::from("Error: Invalid characters in profile name.");
            return Ok(());
        }

        let new_profile_dir = self.base_dir.join("profiles").join(&name);
        if new_profile_dir.exists() {
            self.status_msg = format!("Error: Profile '{}' already exists!", name);
            return Ok(());
        }

        fs::create_dir_all(&new_profile_dir)?;

        let mut copied_count = 0;
        for target in &self.known_targets {
            let live_path = self.home_dir.join(&target.home_rel_path);
            let target_dest = new_profile_dir.join(&target.store_name);

            if Self::copy_entry(&live_path, &target_dest, &new_profile_dir)? {
                copied_count += 1;
            }
        }

        if copied_count == 0 {
            let _ = fs::remove_dir_all(&new_profile_dir);
            self.status_msg = String::from("Error: No valid configs found to snapshot!");
            return Ok(());
        }

        self.reload_profiles()?;
        if let Some(idx) = self.profiles.iter().position(|p| *p == name) {
            self.state.select(Some(idx));
            self.apply_selected_profile()?;
            self.status_msg = format!(
                "Snapshot '{}' created ({} modules) & activated!",
                name, copied_count
            );
        }

        Ok(())
    }

    fn delete_selected_profile(&mut self) -> Result<()> {
        let Some(selected) = self.state.selected() else {
            return Ok(());
        };
        let profile_name = self.profiles[selected].clone();
        let profile_path = self.base_dir.join("profiles").join(&profile_name);

        for target in self.get_profile_targets(&profile_name) {
            let live_target = self.home_dir.join(&target.home_rel_path);
            if let Ok(link_dest) = fs::read_link(&live_target) {
                if link_dest.starts_with(&profile_path) {
                    let _ = fs::remove_file(&live_target);
                }
            }
        }

        if profile_path.exists() {
            fs::remove_dir_all(&profile_path)?;
        }

        if self.active_profile == profile_name {
            self.active_profile = String::from("None (Deleted)");
        }

        self.reload_profiles()?;
        self.status_msg = format!("Deleted profile '{}'.", profile_name);
        Ok(())
    }

    /// Run a helper command without corrupting the alternate-screen UI.
    fn run_silent(cmd: &str, args: &[&str]) -> bool {
        Command::new(cmd)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check for an executable directly in the user's `PATH`.
    fn command_exists(cmd: &str) -> bool {
        if let Ok(path) = std::env::var("PATH") {
            for p in std::env::split_paths(&path) {
                let full_path = p.join(cmd);
                if full_path.is_file() {
                    return true;
                }
            }
        }
        false
    }

    /// Check whether a process with the exact binary name is running.
    fn is_proc_alive(name: &str) -> bool {
        Self::run_silent("pgrep", &["-x", name])
    }

    /// Stop competing daemons and give them time to release sockets and locks.
    fn kill_and_wait(exact_names: &[&str]) {
        for &name in exact_names {
            Self::run_silent("pkill", &["-x", name]);
        }

        for _ in 0..20 {
            let any_alive = exact_names.iter().any(|&name| Self::is_proc_alive(name));
            if !any_alive {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        thread::sleep(Duration::from_millis(150));
    }

    /// Start a daemon without attaching its stdio to the TUI.
    fn spawn_detached(cmd: &str) {
        let shell_cmd = format!("nohup {} >/dev/null 2>&1 &", cmd);
        let _ = Command::new("sh")
            .args(["-c", &shell_cmd])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    /// Detect shell references in the profile's Hyprland configuration files.
    fn hypr_dir_mentions(hypr_dir: &Path, keyword: &str) -> bool {
        let Ok(entries) = fs::read_dir(hypr_dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if content.contains(keyword) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Live-reload Noctalia when possible, then try progressively simpler launches.
    fn reload_or_spawn_noctalia(&self) -> String {
        if Self::is_proc_alive("noctalia") && Self::command_exists("noctalia") {
            if Self::run_silent("noctalia", &["msg", "config-reload"]) {
                return "noctalia msg config-reload".to_string();
            }
        }

        if Self::command_exists("noctalia") {
            if Self::run_silent("noctalia", &["--daemon"]) && Self::is_proc_alive("noctalia") {
                return "spawned noctalia --daemon".to_string();
            }

            Self::spawn_detached("noctalia");
            thread::sleep(Duration::from_millis(300));
            if Self::is_proc_alive("noctalia") {
                return "spawned noctalia".to_string();
            }

            let _ = Command::new("hyprctl")
                .args(["dispatch", "exec", "noctalia"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            thread::sleep(Duration::from_millis(300));
            if Self::is_proc_alive("noctalia") {
                return "hyprctl exec noctalia".to_string();
            }
        }

        if Self::command_exists("noctalia-shell") {
            Self::spawn_detached("noctalia-shell");
            return "spawned noctalia-shell".to_string();
        }

        "failed to start noctalia (check ~/.cache/noctalia/noctalia.log)".to_string()
    }

    /// Stop conflicting daemons and start the services selected by a profile.
    fn reload_ecosystem_daemons(&mut self, profile_name: &str) {
        let profile_path = self.base_dir.join("profiles").join(profile_name);
        let targets = self.get_profile_targets(profile_name);
        let has_module = |name: &str| targets.iter().any(|t| t.store_name == name);

        let mut actions = Vec::new();

        Self::run_silent("hyprctl", &["reload"]);
        actions.push("hyprctl reload".to_string());

        let hypr_dir = profile_path.join("hypr");
        let wants_noctalia = has_module("noctalia")
            || has_module("noctalia-shell")
            || Self::hypr_dir_mentions(&hypr_dir, "noctalia")
            || Self::command_exists("noctalia");

        let mut procs_to_kill = vec!["waybar", "ags", "gjs", "ironbar", "hyprpanel", "caelestia"];
        if !wants_noctalia {
            procs_to_kill.push("noctalia");
            procs_to_kill.push("noctalia-shell");
            procs_to_kill.push("quickshell");
            procs_to_kill.push("qs");
        }
        Self::kill_and_wait(&procs_to_kill);

        if Self::command_exists("eww") {
            Self::run_silent("eww", &["kill"]);
        }

        if wants_noctalia
            && !has_module("waybar")
            && !has_module("caelestia")
            && !has_module("ags")
            && !has_module("hyprpanel")
        {
            let msg = self.reload_or_spawn_noctalia();
            actions.push(msg);
        } else if has_module("caelestia") || has_module("calestia") {
            Self::kill_and_wait(&["noctalia", "noctalia-shell", "quickshell", "qs"]);
            if Self::command_exists("caelestia") {
                Self::spawn_detached("caelestia shell -d");
                actions.push("spawned caelestia".to_string());
            } else if Self::command_exists("qs") {
                Self::spawn_detached("qs -c caelestia");
                actions.push("spawned qs -c caelestia".to_string());
            }
        } else if has_module("waybar") && Self::command_exists("waybar") {
            Self::kill_and_wait(&["noctalia", "noctalia-shell", "quickshell", "qs"]);
            Self::spawn_detached("waybar");
            actions.push("spawned waybar".to_string());
        } else if has_module("ags") && Self::command_exists("ags") {
            Self::kill_and_wait(&["noctalia", "noctalia-shell", "quickshell", "qs"]);
            Self::spawn_detached("ags");
            actions.push("spawned ags".to_string());
        } else if has_module("hyprpanel") && Self::command_exists("hyprpanel") {
            Self::kill_and_wait(&["noctalia", "noctalia-shell", "quickshell", "qs"]);
            Self::spawn_detached("hyprpanel");
            actions.push("spawned hyprpanel".to_string());
        } else if has_module("ironbar") && Self::command_exists("ironbar") {
            Self::kill_and_wait(&["noctalia", "noctalia-shell", "quickshell", "qs"]);
            Self::spawn_detached("ironbar");
            actions.push("spawned ironbar".to_string());
        } else if has_module("eww") && Self::command_exists("eww") {
            Self::kill_and_wait(&["noctalia", "noctalia-shell", "quickshell", "qs"]);
            Self::spawn_detached("eww daemon");
            actions.push("spawned eww daemon".to_string());
        } else if has_module("quickshell") && Self::command_exists("quickshell") {
            Self::spawn_detached("quickshell");
            actions.push("spawned quickshell".to_string());
        }

        if has_module("swaync") || has_module("dunst") || has_module("mako") {
            Self::kill_and_wait(&["swaync", "dunst", "mako"]);

            if has_module("swaync") && Self::command_exists("swaync") {
                Self::spawn_detached("swaync");
                actions.push("restarted swaync".to_string());
            } else if has_module("dunst") && Self::command_exists("dunst") {
                Self::spawn_detached("dunst");
                actions.push("restarted dunst".to_string());
            } else if has_module("mako") && Self::command_exists("mako") {
                Self::spawn_detached("mako");
                actions.push("restarted mako".to_string());
            }
        }

        if has_module("kitty") {
            Self::run_silent("killall", &["-SIGUSR1", "kitty"]);
            actions.push("reloaded kitty".to_string());
        }
        if has_module("wal") && Self::command_exists("wal") {
            Self::run_silent("wal", &["-R", "-q"]);
            actions.push("restored pywal".to_string());
        }

        let custom_hook = profile_path.join("reload.sh");
        if custom_hook.is_file() {
            if let Some(hook_str) = custom_hook.to_str() {
                Self::spawn_detached(&format!("sh '{}'", hook_str));
                actions.push("ran reload.sh".to_string());
            }
        }

        self.last_reload_summary = actions.join(" -> ");
    }

    fn apply_selected_profile(&mut self) -> Result<()> {
        let Some(selected) = self.state.selected() else {
            return Ok(());
        };
        let profile_name = self.profiles[selected].clone();
        let profile_path = self.base_dir.join("profiles").join(&profile_name);
        let profiles_dir = self.base_dir.join("profiles");
        let backup_dir = self.base_dir.join("backups");
        fs::create_dir_all(&backup_dir)?;

        for target in &self.known_targets {
            let live_target = self.home_dir.join(&target.home_rel_path);
            if let Ok(link_dest) = fs::read_link(&live_target) {
                if link_dest.starts_with(&profiles_dir) {
                    let _ = fs::remove_file(&live_target);
                }
            }
        }

        let profile_targets = self.get_profile_targets(&profile_name);
        for target in &profile_targets {
            let source = profile_path.join(&target.store_name);
            let live_target = self.home_dir.join(&target.home_rel_path);

            if !source.exists() {
                continue;
            }

            if let Some(parent) = live_target.parent() {
                fs::create_dir_all(parent)?;
            }

            if let Ok(meta) = fs::symlink_metadata(&live_target) {
                if meta.file_type().is_symlink() {
                    fs::remove_file(&live_target)?;
                } else if meta.is_dir() {
                    let backup_target = backup_dir.join(format!("{}_backup", target.store_name));
                    if backup_target.exists() {
                        fs::remove_dir_all(&backup_target)?;
                    }
                    fs::rename(&live_target, &backup_target)?;
                } else if meta.is_file() {
                    let backup_target = backup_dir.join(format!("{}_backup", target.store_name));
                    if backup_target.exists() {
                        fs::remove_file(&backup_target)?;
                    }
                    fs::rename(&live_target, &backup_target)?;
                }
            }

            symlink(&source, &live_target)?;
        }

        self.reload_ecosystem_daemons(&profile_name);

        self.active_profile = profile_name.clone();
        self.status_msg = format!(
            "Switched to '{}' ({})!",
            profile_name, self.last_reload_summary
        );
        Ok(())
    }
}

/// Copy a directory while preserving internal symlinks and skipping cycles.
fn copy_dir_all(src: &Path, dst: &Path, root_src: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        let meta = fs::symlink_metadata(&src_path)?;
        let file_type = meta.file_type();

        if file_type.is_symlink() {
            let link_target = fs::read_link(&src_path)?;

            if let Ok(resolved) = fs::canonicalize(&src_path) {
                if resolved == root_src || root_src.starts_with(&resolved) {
                    continue;
                }
            }

            if dst_path.exists() || fs::symlink_metadata(&dst_path).is_ok() {
                let _ = fs::remove_file(&dst_path);
            }
            symlink(&link_target, &dst_path)?;
        } else if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path, root_src)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Return a centered rectangle for a modal with a fixed height.
fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = App::new()?;
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }
    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match app.input_mode {
                InputMode::Normal => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Char('s') => {
                        app.input_mode = InputMode::SnapshotName;
                        app.input_buffer.clear();
                        app.status_msg = String::from(
                            "Type snapshot name and press [Enter], or [Esc] to cancel.",
                        );
                    }
                    KeyCode::Char('i') => {
                        app.input_mode = InputMode::ImportPath;
                        app.input_buffer = String::from("~/");
                        app.status_msg = String::from(
                            "Enter path to external dotfiles folder (e.g. ~/dotfiles) and press [Enter].",
                        );
                    }
                    KeyCode::Char('d') => {
                        if let Some(selected) = app.state.selected() {
                            let name = &app.profiles[selected];
                            app.input_mode = InputMode::ConfirmDelete;
                            app.status_msg = format!(
                                "Confirm deleting profile '{}'? Press [y] to delete or [n]/[Esc] to cancel.",
                                name
                            );
                        }
                    }
                    KeyCode::Char('r') => {
                        if let Some(selected) = app.state.selected() {
                            let name = app.profiles[selected].clone();
                            app.reload_ecosystem_daemons(&name);
                            app.status_msg = format!(
                                "Force-reloaded daemons for '{}': {}",
                                name, app.last_reload_summary
                            );
                        }
                    }
                    KeyCode::Enter => {
                        if let Err(e) = app.apply_selected_profile() {
                            app.status_msg = format!("Error: {}", e);
                        }
                    }
                    _ => {}
                },
                InputMode::SnapshotName => match key.code {
                    KeyCode::Enter => {
                        app.input_mode = InputMode::Normal;
                        if let Err(e) = app.create_snapshot() {
                            app.status_msg = format!("Snapshot Error: {}", e);
                        }
                    }
                    KeyCode::Esc => {
                        app.input_mode = InputMode::Normal;
                        app.status_msg = String::from("Snapshot cancelled.");
                    }
                    KeyCode::Backspace => {
                        app.input_buffer.pop();
                    }
                    KeyCode::Char(c) => {
                        app.input_buffer.push(c);
                    }
                    _ => {}
                },
                InputMode::ImportPath => match key.code {
                    KeyCode::Enter => {
                        app.prepare_import_path();
                    }
                    KeyCode::Esc => {
                        app.input_mode = InputMode::Normal;
                        app.status_msg = String::from("Import cancelled.");
                    }
                    KeyCode::Backspace => {
                        app.input_buffer.pop();
                    }
                    KeyCode::Char(c) => {
                        app.input_buffer.push(c);
                    }
                    _ => {}
                },
                InputMode::ImportName => match key.code {
                    KeyCode::Enter => {
                        app.input_mode = InputMode::Normal;
                        if let Err(e) = app.execute_import() {
                            app.status_msg = format!("Import Error: {}", e);
                        }
                    }
                    KeyCode::Esc => {
                        app.input_mode = InputMode::Normal;
                        app.status_msg = String::from("Import cancelled.");
                    }
                    KeyCode::Backspace => {
                        app.input_buffer.pop();
                    }
                    KeyCode::Char(c) => {
                        app.input_buffer.push(c);
                    }
                    _ => {}
                },
                InputMode::ConfirmDelete => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        app.input_mode = InputMode::Normal;
                        if let Err(e) = app.delete_selected_profile() {
                            app.status_msg = format!("Delete Error: {}", e);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        app.input_mode = InputMode::Normal;
                        app.status_msg = String::from("Delete cancelled.");
                    }
                    _ => {}
                },
            }
        }
    }
}

fn ui(frame: &mut Frame, app: &mut App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(frame.area());

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(main_layout[0]);

    let items: Vec<ListItem> = app
        .profiles
        .iter()
        .map(|p| {
            let prefix = if *p == app.active_profile {
                "● "
            } else {
                "  "
            };
            ListItem::new(format!("{}{}", prefix, p))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Hyprland Profiles ")
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black).bold())
        .highlight_symbol(">> ");

    frame.render_stateful_widget(list, top_chunks[0], &mut app.state);

    let preview_text = if let Some(selected) = app.state.selected() {
        let profile = &app.profiles[selected];
        let profile_path = app.base_dir.join("profiles").join(profile);
        let detected_targets = app.get_profile_targets(profile);

        let mut target_lines = String::new();
        if detected_targets.is_empty() {
            target_lines.push_str("  (Empty profile folder)\n");
        } else {
            for t in &detected_targets {
                target_lines.push_str(&format!(
                    "  [✓] {:<15} -> ~/{:<24} ({})\n",
                    t.store_name, t.home_rel_path, t.category
                ));
            }
        }

        let has_custom_hook = if profile_path.join("reload.sh").exists() {
            "Yes (reload.sh found)"
        } else {
            "None (Optional: add reload.sh to profile folder)"
        };

        format!(
            "Active Profile:   {}\nSelected Profile: {}\nLocation:         {}\nCustom Hook:      {}\nLast Reload:      {}\n\nDetected Ecosystem Modules ({}):\n{}\nKeybinds:\n  [Enter] : Activate profile & switch running daemons\n  [r]     : Force-reload daemons for selected profile\n  [s]     : Snapshot current live setup\n  [i]     : Import external dotfiles folder\n  [d]     : Delete selected profile\n  [q]     : Quit",
            app.active_profile,
            profile,
            profile_path.display(),
            has_custom_hook,
            app.last_reload_summary,
            detected_targets.len(),
            target_lines
        )
    } else {
        format!(
            "Active Profile: {}\n\nNo saved profiles found in ~/.hyprdeck/profiles.\n\nAuto-Detects & Reloads Ecosystem Modules:\n  - WM & Shells: hypr, noctalia, caelestia, quickshell, waybar, ags, eww\n  - Terminals:   kitty (live SIGUSR1 reload), alacritty, foot, wezterm, ghostty\n  - Launchers:   rofi, wofi, fuzzel, tofi, anyrun, walker\n  - Theming:     wal (live wal -R), starship.toml, .zshrc, dunst, swaync, mako\n\nKeybinds:\n  [s] : Snapshot your current live setup\n  [i] : Import an external dotfiles folder\n  [q] : Quit",
            app.active_profile
        )
    };

    let preview = Paragraph::new(preview_text).block(
        Block::default()
            .title(" Profile Details ")
            .borders(Borders::ALL),
    );
    frame.render_widget(preview, top_chunks[1]);

    let status = Paragraph::new(app.status_msg.as_str())
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(status, main_layout[1]);

    match app.input_mode {
        InputMode::Normal => {}
        InputMode::SnapshotName => {
            let area = centered_rect(55, 3, frame.area());
            frame.render_widget(Clear, area);
            let input_box = Paragraph::new(format!("{}_", app.input_buffer))
                .style(Style::default().fg(Color::Cyan))
                .block(
                    Block::default()
                        .title(" New Snapshot Name (Enter to Save, Esc to Cancel) ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            frame.render_widget(input_box, area);
        }
        InputMode::ImportPath => {
            let area = centered_rect(65, 3, frame.area());
            frame.render_widget(Clear, area);
            let input_box = Paragraph::new(format!("{}_", app.input_buffer))
                .style(Style::default().fg(Color::Green))
                .block(
                    Block::default()
                        .title(" Import Step 1/2: Source Folder Path (e.g. ~/dotfiles) ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                );
            frame.render_widget(input_box, area);
        }
        InputMode::ImportName => {
            let area = centered_rect(65, 3, frame.area());
            frame.render_widget(Clear, area);
            let input_box = Paragraph::new(format!("{}_", app.input_buffer))
                .style(Style::default().fg(Color::Green))
                .block(
                    Block::default()
                        .title(" Import Step 2/2: Name For Imported Profile ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                );
            frame.render_widget(input_box, area);
        }
        InputMode::ConfirmDelete => {
            if let Some(selected) = app.state.selected() {
                let name = &app.profiles[selected];
                let area = centered_rect(55, 4, frame.area());
                frame.render_widget(Clear, area);
                let confirm_box = Paragraph::new(format!(
                    "Are you sure you want to permanently delete '{}'?\nPress [y] to Confirm, or [n] / [Esc] to Cancel.",
                    name
                ))
                .style(Style::default().fg(Color::Red).bold())
                .block(
                    Block::default()
                        .title(" ⚠ Confirm Delete Profile ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red)),
                );
                frame.render_widget(confirm_box, area);
            }
        }
    }
}
