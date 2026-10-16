mod logo;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
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

// ---------------------------------------------------------------------------
// Theme
//
// The palette is lifted straight from the artwork (logo.png): the purple and
// blue of the stacked windows plus the dark slate of the front card.
// ---------------------------------------------------------------------------

const C_PURPLE: Color = Color::Rgb(0xb4, 0x8f, 0xf6);
const C_BLUE: Color = Color::Rgb(0x82, 0xb5, 0xfd);
const C_SLATE: Color = Color::Rgb(0x2d, 0x37, 0x5b);
const C_TEXT: Color = Color::Rgb(0xd8, 0xdd, 0xea);
const C_MUTED: Color = Color::Rgb(0x9a, 0xa2, 0xba);
const C_DIM: Color = Color::Rgb(0x5f, 0x68, 0x84);
const C_OK: Color = Color::Rgb(0x8b, 0xd4, 0xa0);
const C_WARN: Color = Color::Rgb(0xf6, 0xc1, 0x77);
const C_ERR: Color = Color::Rgb(0xf2, 0x7e, 0x8a);

const PURPLE_RGB: (u8, u8, u8) = (0xb4, 0x8f, 0xf6);
const BLUE_RGB: (u8, u8, u8) = (0x82, 0xb5, 0xfd);

/// A rounded panel with a styled title, matching the app's visual language.
fn panel<'a>(title: Line<'a>, accent: Color) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title(title)
}

/// Colour a word left-to-right with a linear gradient between two RGB values.
fn gradient(text: &str, from: (u8, u8, u8), to: (u8, u8, u8), bold: bool) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let last = chars.len().saturating_sub(1).max(1) as f32;

    chars
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let t = i as f32 / last;
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            let mut style = Style::default().fg(Color::Rgb(
                mix(from.0, to.0),
                mix(from.1, to.1),
                mix(from.2, to.2),
            ));
            if bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            Span::styled(c.to_string(), style)
        })
        .collect()
}

/// `[key] label` chip used across the footer and empty states.
fn key_hint(key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {} ", key),
            Style::default()
                .fg(C_BLUE)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {}   ", label), Style::default().fg(C_MUTED)),
    ]
}

/// Pick a status colour from the message text.
fn status_style(msg: &str) -> Style {
    let m = msg.to_ascii_lowercase();
    if [
        "error",
        "failed",
        "invalid",
        "cannot",
        "cancelled",
        "cancel",
    ]
    .iter()
    .any(|k| m.contains(k))
    {
        if m.contains("cancel") {
            Style::default().fg(C_MUTED)
        } else {
            Style::default().fg(C_ERR).add_modifier(Modifier::BOLD)
        }
    } else if [
        "switched",
        "created",
        "imported",
        "deleted",
        "reloaded",
        "activated",
    ]
    .iter()
    .any(|k| m.contains(k))
    {
        Style::default().fg(C_OK).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(C_WARN)
    }
}

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
            status_msg: String::from("Ready — pick a profile and press [Enter] to activate it."),
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
    let area = frame.area();
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);

    render_header(frame, rows[0], app);
    render_body(frame, rows[1], app);
    render_footer(frame, rows[2], app);
    render_overlays(frame, app);
}

/// Top bar: gradient wordmark in the border, live state on the right.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let mut title = vec![Span::raw(" ")];
    title.extend(gradient("HYPRDECK", PURPLE_RGB, BLUE_RGB, true));
    title.push(Span::styled(
        "  ·  Hyprland profile & dotfile manager ",
        Style::default().fg(C_DIM),
    ));

    let block = panel(Line::from(title), C_SLATE);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 8 || inner.height == 0 {
        return;
    }

    let managed = !app.active_profile.contains("External") && !app.active_profile.contains("None");
    let mut left = vec![Span::raw("  ")];
    if managed {
        left.push(Span::styled("● ", Style::default().fg(C_OK)));
        left.push(Span::styled("active ", Style::default().fg(C_DIM)));
        left.push(Span::styled(
            app.active_profile.clone(),
            Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
        ));
    } else {
        left.push(Span::styled("○ ", Style::default().fg(C_DIM)));
        left.push(Span::styled(
            "no profile active ",
            Style::default().fg(C_DIM),
        ));
    }

    let stats = format!(
        "{} profile{}  ·  {} targets tracked  ",
        app.profiles.len(),
        if app.profiles.len() == 1 { "" } else { "s" },
        app.known_targets.len()
    );

    let halves = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(stats.chars().count() as u16),
    ])
    .split(inner);

    frame.render_widget(Paragraph::new(Line::from(left)), halves[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(stats, Style::default().fg(C_DIM))))
            .alignment(Alignment::Right),
        halves[1],
    );
}

/// Two-column workspace: the branded sidebar and the details pane.
fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.width < 44 || area.height < 5 {
        // Too cramped to split — degrade to the profile list alone.
        render_profile_list(frame, area, app);
        return;
    }

    let sidebar_w = (area.width / 100 * 34).clamp(30, 44).min(area.width - 20);
    let cols = Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(20)]).split(area);

    render_sidebar(frame, cols[0], app);
    render_details(frame, cols[1], app);
}

/// Sidebar: the logo splash on top of the profile list.
fn render_sidebar(frame: &mut Frame, area: Rect, app: &mut App) {
    let (mut rows, mut show_logo) = (0usize, area.height >= 18 && area.width >= 18);
    if show_logo {
        // Give the artwork roughly half the column, but never starve the list.
        rows = (area.height.saturating_sub(10) as usize).clamp(4, 13);
        // Half-block cells are twice as tall as they are wide, so a square
        // mark needs twice as many columns as rows. Trim to fit the sidebar.
        let max_cols = area.width.saturating_sub(6) as usize;
        while rows > 4 && rows * 2 > max_cols {
            rows -= 1;
        }
        // Below this the artwork turns to mush, so drop it entirely.
        show_logo = rows >= 8;
    }

    if !show_logo {
        render_profile_list(frame, area, app);
        return;
    }

    let logo_h = rows as u16 + 2;
    let parts = Layout::vertical([Constraint::Length(logo_h), Constraint::Min(4)]).split(area);

    render_logo(frame, parts[0], rows * 2, rows);
    render_profile_list(frame, parts[1], app);
}

/// The artwork, centered inside its own rounded card.
fn render_logo(frame: &mut Frame, area: Rect, cols: usize, rows: usize) {
    let title = Line::from(vec![
        Span::styled(" ◈ ", Style::default().fg(C_PURPLE)),
        Span::styled("hyprdeck", Style::default().fg(C_BLUE)),
        Span::raw(" "),
    ]);
    let block = panel(title, C_SLATE);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let lines = logo::render(cols, rows);
    let height = (rows as u16).min(inner.height);
    let art = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(height) / 2,
        width: inner.width,
        height,
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        art,
    );
}

fn render_profile_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let title = Line::from(vec![Span::styled(
        format!("  Profiles · {} ", app.profiles.len()),
        Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
    )]);
    let block = panel(title, C_BLUE);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.profiles.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Nothing here yet.",
                    Style::default().fg(C_TEXT),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("[s]", Style::default().fg(C_BLUE)),
                    Span::styled(" snapshot your live setup", Style::default().fg(C_DIM)),
                ]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("[i]", Style::default().fg(C_BLUE)),
                    Span::styled(" import a dotfiles folder", Style::default().fg(C_DIM)),
                ]),
            ]),
            inner,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .profiles
        .iter()
        .map(|name| {
            let active = *name == app.active_profile;
            let (marker, marker_style) = if active {
                ("● ", Style::default().fg(C_OK))
            } else {
                ("○ ", Style::default().fg(C_DIM))
            };
            let name_style = if active {
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(C_TEXT)
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(name.clone(), name_style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(C_SLATE)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌ ");

    frame.render_stateful_widget(list, inner, &mut app.state);
}

fn render_details(frame: &mut Frame, area: Rect, app: &App) {
    let label = match app.state.selected() {
        Some(i) => app.profiles[i].clone(),
        None => String::from("welcome"),
    };

    let title = Line::from(vec![
        Span::styled(
            "  Details ",
            Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("· {} ", label), Style::default().fg(C_DIM)),
    ]);
    let block = panel(title, C_SLATE);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = match app.state.selected() {
        Some(selected) => profile_details(app, selected),
        None => welcome_details(app, inner.width as usize),
    };

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn profile_details(app: &App, selected: usize) -> Vec<Line<'static>> {
    let profile = &app.profiles[selected];
    let profile_path = app.base_dir.join("profiles").join(profile);
    let targets = app.get_profile_targets(profile);
    let active = *profile == app.active_profile;

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                profile.clone(),
                Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
            ),
            if active {
                Span::styled("   ● active", Style::default().fg(C_OK))
            } else {
                Span::styled("   ○ inactive", Style::default().fg(C_DIM))
            },
        ]),
        Line::from(Span::styled(
            format!("  {}", profile_path.display()),
            Style::default().fg(C_DIM),
        )),
        Line::from(""),
    ];

    lines.push(Line::from(vec![
        Span::styled(
            "  Modules ",
            Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("({})", targets.len()), Style::default().fg(C_DIM)),
    ]));

    if targets.is_empty() {
        lines.push(Line::from(Span::styled(
            "    empty profile directory",
            Style::default().fg(C_DIM),
        )));
    } else {
        for target in &targets {
            lines.push(Line::from(vec![
                Span::styled("    ✓ ", Style::default().fg(C_OK)),
                Span::styled(
                    format!("{:<16}", target.store_name),
                    Style::default().fg(C_TEXT),
                ),
                Span::styled(
                    format!("{:<26}", target.home_rel_path),
                    Style::default().fg(C_MUTED),
                ),
                Span::styled(target.category.to_string(), Style::default().fg(C_DIM)),
            ]));
        }
    }

    let hook = if profile_path.join("reload.sh").exists() {
        Span::styled("reload.sh found", Style::default().fg(C_OK))
    } else {
        Span::styled("none (optional)", Style::default().fg(C_DIM))
    };

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Custom hook   ", Style::default().fg(C_MUTED)),
        hook,
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Last reload   ", Style::default().fg(C_MUTED)),
        Span::styled(app.last_reload_summary.clone(), Style::default().fg(C_TEXT)),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("[Enter]", Style::default().fg(C_BLUE)),
        Span::styled(
            " activates this profile and restarts its daemons",
            Style::default().fg(C_DIM),
        ),
    ]));

    lines
}

fn welcome_details(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Welcome to Hyprdeck",
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            "  No profiles yet. Save the setup you are running now, or import one:",
            Style::default().fg(C_MUTED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "[s]",
                Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  snapshot the current configuration",
                Style::default().fg(C_TEXT),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "[i]",
                Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  import an existing dotfiles folder",
                Style::default().fg(C_TEXT),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("  Targets Hyprdeck tracks ({})", app.known_targets.len()),
            Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
        )),
    ];

    let mut grouped: Vec<(&str, Vec<&str>)> = Vec::new();
    for target in &app.known_targets {
        match grouped.iter_mut().find(|(cat, _)| *cat == target.category) {
            Some((_, names)) => names.push(target.store_name.as_str()),
            None => grouped.push((target.category, vec![target.store_name.as_str()])),
        }
    }

    // Keep continuation lines aligned under the names column instead of
    // letting the paragraph wrap them back to the left edge.
    const NAMES_COL: usize = 19; // 3 leading spaces + 16-wide category column
    let available = width.saturating_sub(NAMES_COL).max(12);

    for (category, names) in grouped {
        for (i, chunk) in wrap_names(&names, available).into_iter().enumerate() {
            let lead = if i == 0 {
                format!("   {category:<16}")
            } else {
                " ".repeat(NAMES_COL)
            };
            lines.push(Line::from(vec![
                Span::styled(lead, Style::default().fg(C_MUTED)),
                Span::styled(chunk, Style::default().fg(C_DIM)),
            ]));
        }
    }

    lines
}

/// Greedily pack `names` into comma-separated lines that fit `width` columns.
fn wrap_names(names: &[&str], width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for name in names {
        let candidate = if current.is_empty() {
            (*name).to_string()
        } else {
            format!("{current}, {name}")
        };

        if candidate.chars().count() > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current = (*name).to_string();
        } else {
            current = candidate;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let status_area = Rect { height: 1, ..area };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(app.status_msg.clone(), status_style(&app.status_msg)),
        ])),
        status_area,
    );

    if area.height < 2 {
        return;
    }

    let mut spans = vec![Span::raw("  ")];
    for (key, label) in [
        ("Enter", "Activate"),
        ("s", "Snapshot"),
        ("i", "Import"),
        ("d", "Delete"),
        ("r", "Reload"),
        ("↑↓", "Select"),
        ("q", "Quit"),
    ] {
        spans.extend(key_hint(key, label));
    }

    let hints_area = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };
    frame.render_widget(Paragraph::new(Line::from(spans)), hints_area);
}

fn render_overlays(frame: &mut Frame, app: &App) {
    match app.input_mode {
        InputMode::Normal => {}
        InputMode::SnapshotName => {
            let area = centered_rect(58, 3, frame.area());
            frame.render_widget(Clear, area);
            let input_box = Paragraph::new(Line::from(vec![
                Span::styled(app.input_buffer.clone(), Style::default().fg(C_TEXT)),
                Span::styled("_", Style::default().fg(C_BLUE)),
            ]))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(C_BLUE))
                    .title(Line::from(Span::styled(
                        " New snapshot name · Enter to save, Esc to cancel ",
                        Style::default().fg(C_BLUE),
                    ))),
            );
            frame.render_widget(input_box, area);
        }
        InputMode::ImportPath => {
            let area = centered_rect(68, 3, frame.area());
            frame.render_widget(Clear, area);
            let input_box = Paragraph::new(Line::from(vec![
                Span::styled(app.input_buffer.clone(), Style::default().fg(C_TEXT)),
                Span::styled("_", Style::default().fg(C_PURPLE)),
            ]))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(C_PURPLE))
                    .title(Line::from(Span::styled(
                        " Import 1/2 · source folder (e.g. ~/dotfiles) ",
                        Style::default().fg(C_PURPLE),
                    ))),
            );
            frame.render_widget(input_box, area);
        }
        InputMode::ImportName => {
            let area = centered_rect(68, 3, frame.area());
            frame.render_widget(Clear, area);
            let input_box = Paragraph::new(Line::from(vec![
                Span::styled(app.input_buffer.clone(), Style::default().fg(C_TEXT)),
                Span::styled("_", Style::default().fg(C_PURPLE)),
            ]))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(C_PURPLE))
                    .title(Line::from(Span::styled(
                        " Import 2/2 · name for the imported profile ",
                        Style::default().fg(C_PURPLE),
                    ))),
            );
            frame.render_widget(input_box, area);
        }
        InputMode::ConfirmDelete => {
            if let Some(selected) = app.state.selected() {
                let name = &app.profiles[selected];
                let area = centered_rect(58, 4, frame.area());
                frame.render_widget(Clear, area);
                let confirm_box = Paragraph::new(Line::from(vec![
                    Span::styled("Delete ", Style::default().fg(C_TEXT)),
                    Span::styled(
                        name.clone(),
                        Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("? This cannot be undone.", Style::default().fg(C_TEXT)),
                ]))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(C_ERR))
                        .title(Line::from(Span::styled(
                            " Confirm delete · y to confirm, n / Esc to cancel ",
                            Style::default().fg(C_ERR),
                        ))),
                );
                frame.render_widget(confirm_box, area);
            }
        }
    }
}
