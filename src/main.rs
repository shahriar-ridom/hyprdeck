mod logo;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
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
// Theme & Palette
//
// Refined, high-contrast palette harmonized with modern Wayland & Hyprland themes.
// ---------------------------------------------------------------------------

const C_PURPLE: Color = Color::Rgb(0xb4, 0x8f, 0xf6);
const C_BLUE: Color = Color::Rgb(0x82, 0xb5, 0xfd);
const C_CYAN: Color = Color::Rgb(0x7d, 0xcf, 0xff);
const C_SLATE: Color = Color::Rgb(0x24, 0x2b, 0x44);
const C_BG_CARD: Color = Color::Rgb(0x1a, 0x1f, 0x33);
const C_BG_SEL: Color = Color::Rgb(0x2f, 0x39, 0x5e);
const C_BORDER_DIM: Color = Color::Rgb(0x3e, 0x47, 0x6e);

const C_TEXT_BRIGHT: Color = Color::Rgb(0xf2, 0xf5, 0xff);
const C_TEXT: Color = Color::Rgb(0xd0, 0xd6, 0xea);
const C_MUTED: Color = Color::Rgb(0x8d, 0x97, 0xb8);
const C_DIM: Color = Color::Rgb(0x54, 0x5d, 0x80);

const C_OK: Color = Color::Rgb(0x73, 0xda, 0xca);
const C_WARN: Color = Color::Rgb(0xf6, 0xc1, 0x77);
const C_ERR: Color = Color::Rgb(0xf7, 0x76, 0x8e);

const PURPLE_RGB: (u8, u8, u8) = (0xb4, 0x8f, 0xf6);
const BLUE_RGB: (u8, u8, u8) = (0x82, 0xb5, 0xfd);

/// Replace home path prefix with `~/` for compact, clean display.
/// Browser data directories (profiles, settings, sessions). Never snapshotted,
/// imported or linked: switching dotfile profiles must not touch browser settings.
const BROWSER_DIRS: &[&str] = &[
    "zen",
    "mozilla",
    "librewolf",
    "floorp",
    "waterfox",
    "firefox",
    "google-chrome",
    "google-chrome-beta",
    "google-chrome-unstable",
    "chromium",
    "BraveSoftware",
    "vivaldi",
    "microsoft-edge",
    "opera",
    "thorium",
    "epiphany",
    ".zen",
    ".mozilla",
];

fn is_browser_dir(name: &str) -> bool {
    BROWSER_DIRS.contains(&name)
}

fn compact_path(path: &Path, home: &Path) -> String {
    if let Ok(stripped) = path.strip_prefix(home) {
        format!("~/{}", stripped.display())
    } else {
        path.display().to_string()
    }
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

/// High-contrast pill keychip used in the footer and help modals.
fn key_hint(key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {} ", key),
            Style::default()
                .fg(C_TEXT_BRIGHT)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {}   ", label), Style::default().fg(C_MUTED)),
    ]
}

/// Formats a status badge pill and corresponding message style based on content.
fn status_badge(msg: &str) -> (Span<'static>, Style) {
    let m = msg.to_ascii_lowercase();
    if ["error", "failed", "invalid", "cannot"]
        .iter()
        .any(|k| m.contains(k))
    {
        (
            Span::styled(
                " ✖ ERROR ",
                Style::default()
                    .fg(Color::Rgb(0x18, 0x18, 0x24))
                    .bg(C_ERR)
                    .add_modifier(Modifier::BOLD),
            ),
            Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
        )
    } else if ["cancelled", "cancel"].iter().any(|k| m.contains(k)) {
        (
            Span::styled(" ○ CANCEL ", Style::default().fg(C_MUTED).bg(C_SLATE)),
            Style::default().fg(C_MUTED),
        )
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
        (
            Span::styled(
                " ● SUCCESS ",
                Style::default()
                    .fg(Color::Rgb(0x18, 0x18, 0x24))
                    .bg(C_OK)
                    .add_modifier(Modifier::BOLD),
            ),
            Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
        )
    } else if m.contains("filter") {
        (
            Span::styled(
                " 🔍 FILTER ",
                Style::default()
                    .fg(Color::Rgb(0x18, 0x18, 0x24))
                    .bg(C_BLUE)
                    .add_modifier(Modifier::BOLD),
            ),
            Style::default().fg(C_BLUE),
        )
    } else {
        (
            Span::styled(
                " ℹ READY ",
                Style::default()
                    .fg(Color::Rgb(0x18, 0x18, 0x24))
                    .bg(C_CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
            Style::default().fg(C_TEXT),
        )
    }
}

/// Active focus pane.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum ActivePane {
    Profiles,
    Details,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum InputMode {
    Normal,
    Filter,
    SnapshotName,
    ImportPath,
    ImportName,
    ConfirmDelete,
    Help,
}

/// Live symlink connection status for audit & inspector.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LinkStatus {
    Active,
    Other,
    Local,
    Missing,
    Broken,
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
    detail_scroll: u16,
    detail_lines_count: usize,
    active_pane: ActivePane,
    filter_query: String,
    filtered_indices: Vec<usize>,
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
            detail_scroll: 0,
            detail_lines_count: 0,
            active_pane: ActivePane::Profiles,
            filter_query: String::new(),
            filtered_indices: Vec::new(),
        };

        app.reload_profiles()?;
        app.detect_active_profile();

        // Auto-select active profile if present in list
        if let Some(pos) = app.profiles.iter().position(|p| p == &app.active_profile) {
            if let Some(f_pos) = app.filtered_indices.iter().position(|&idx| idx == pos) {
                app.state.select(Some(f_pos));
            }
        }

        Ok(app)
    }

    /// Map known profile entries to their live paths and display categories.
    fn build_ecosystem_registry() -> Vec<Target> {
        let mut reg = Vec::new();

        fn add_config(reg: &mut Vec<Target>, name: &str, cat: &'static str) {
            reg.push(Target {
                store_name: name.to_string(),
                home_rel_path: format!(".config/{}", name),
                category: cat,
            });
        }

        fn add_home(reg: &mut Vec<Target>, name: &str, rel_path: &str, cat: &'static str) {
            reg.push(Target {
                store_name: name.to_string(),
                home_rel_path: rel_path.to_string(),
                category: cat,
            });
        }

        // Core Window Manager
        add_config(&mut reg, "hypr", "Core WM");

        // Desktop Shells, Bars & Panels
        for shell in [
            "noctalia",
            "noctalia-shell",
            "caelestia",
            "calestia",
            "quickshell",
            "qs",
            "waybar",
            "ags",
            "eww",
            "ironbar",
            "hyprpanel",
            "sfwbar",
            "yambar",
            "polybar",
            "tint2",
        ] {
            add_config(&mut reg, shell, "Shell / Bar");
        }

        // Hyprland Ecosystem Components & Lock/Idle
        for hypr_tool in [
            "hyprlock",
            "hypridle",
            "hyprpaper",
            "hyprsunset",
            "hyprshade",
            "hyprpicker",
            "hyprcursor",
            "swayosd",
            "swaylock",
            "swayidle",
            "gtklock",
            "wldash",
        ] {
            add_config(&mut reg, hypr_tool, "Hypr Ecosystem");
        }

        // Terminal Emulators
        for term in [
            "kitty",
            "alacritty",
            "foot",
            "wezterm",
            "ghostty",
            "rio",
            "konsole",
            "xfce4-terminal",
            "terminator",
        ] {
            add_config(&mut reg, term, "Terminal");
        }

        // App Launchers & Menus
        for launcher in [
            "rofi", "wofi", "fuzzel", "tofi", "anyrun", "walker", "wlogout", "bemenu",
        ] {
            add_config(&mut reg, launcher, "Launcher / Menu");
        }

        // Notification Daemons
        for notif in ["swaync", "dunst", "mako", "fnott", "wired", "deadd"] {
            add_config(&mut reg, notif, "Notification");
        }

        // Wallpaper Daemons & Display
        for wp in [
            "swww",
            "wpaperd",
            "mpvpaper",
            "swaybg",
            "waypaper",
            "wallpapers",
            "wlsunset",
            "gammastep",
        ] {
            add_config(&mut reg, wp, "Wallpaper / Display");
        }

        // Shells & Prompts
        for shell_config in ["fish", "nushell", "zsh", "ohmyposh"] {
            add_config(&mut reg, shell_config, "Shell / Prompt");
        }
        add_config(&mut reg, "starship.toml", "Shell / Prompt");
        add_home(&mut reg, ".zshrc", ".zshrc", "Shell / Prompt");
        add_home(&mut reg, ".zshenv", ".zshenv", "Shell / Prompt");
        add_home(&mut reg, ".zprofile", ".zprofile", "Shell / Prompt");
        add_home(&mut reg, ".bashrc", ".bashrc", "Shell / Prompt");
        add_home(&mut reg, ".bash_profile", ".bash_profile", "Shell / Prompt");
        add_home(&mut reg, ".bash_logout", ".bash_logout", "Shell / Prompt");

        // Code Editors & IDEs
        for editor in ["nvim", "helix", "micro", "zed", "emacs", "doom", "Code"] {
            add_config(&mut reg, editor, "Editor / IDE");
        }

        // File Managers
        for fm in ["yazi", "ranger", "lf", "Thunar"] {
            add_config(&mut reg, fm, "File Manager");
        }

        // Desktop Theme, Styling & Fonts
        for theme in [
            "wal",
            "wallust",
            "matugen",
            "theme",
            "fontconfig",
            "gtk-2.0",
            "gtk-3.0",
            "gtk-4.0",
            "nwg-look",
            "qt5ct",
            "qt6ct",
            "xsettingsd",
        ] {
            add_config(&mut reg, theme, "Theme / Style");
        }

        // System Utilities, Audio & Tooling
        for util in [
            "fastfetch",
            "neofetch",
            "btop",
            "htop",
            "bottom",
            "nvtop",
            "cava",
            "mpv",
            "mpd",
            "ncmpcpp",
            "spicetify",
            "pipewire",
            "wireplumber",
            "scripts",
            "tmux",
            "lazygit",
            "bat",
            "cliphist",
        ] {
            add_config(&mut reg, util, "System / Utility");
        }
        add_home(&mut reg, ".tmux.conf", ".tmux.conf", "System / Utility");
        add_home(&mut reg, ".gitconfig", ".gitconfig", "System / Utility");

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

        let (home_rel_path, category) = if store_name.starts_with('.') {
            let lower = store_name.to_lowercase();
            let cat = if lower.contains("sh") || lower.contains("profile") {
                "Shell / Prompt"
            } else if lower.contains("git") || lower.contains("tmux") {
                "System / Utility"
            } else {
                "User Dotfile"
            };
            (store_name.to_string(), cat)
        } else {
            let lower = store_name.to_lowercase();
            let cat = if lower.contains("bar")
                || lower.contains("panel")
                || lower.contains("dock")
                || lower.contains("shell")
            {
                "Shell / Bar"
            } else if lower.contains("hypr") {
                "Hypr Ecosystem"
            } else if lower.contains("theme")
                || lower.contains("color")
                || lower.contains("gtk")
                || lower.contains("font")
            {
                "Theme / Style"
            } else if lower.contains("term") {
                "Terminal"
            } else if lower.contains("lock") || lower.contains("idle") {
                "Idle / Lock"
            } else if lower.contains("paper") || lower.contains("wall") {
                "Wallpaper / Display"
            } else if lower.contains("menu") || lower.contains("rofi") || lower.contains("launcher")
            {
                "Launcher / Menu"
            } else if lower.contains("notif") || lower.contains("dunst") || lower.contains("mako") {
                "Notification"
            } else if lower.contains("edit") || lower.contains("vim") || lower.contains("code") {
                "Editor / IDE"
            } else {
                "App Config"
            };
            (format!(".config/{}", store_name), cat)
        };

        Target {
            store_name: store_name.to_string(),
            home_rel_path,
            category,
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

    /// Return the currently selected profile name, considering the active filter.
    fn selected_profile(&self) -> Option<&String> {
        let sel = self.state.selected()?;
        let idx = *self.filtered_indices.get(sel)?;
        self.profiles.get(idx)
    }

    /// Open selected profile directory in system default file manager.
    fn open_profile_folder(&mut self) {
        if let Some(name) = self.selected_profile() {
            let path = self.base_dir.join("profiles").join(name);
            if path.exists() {
                let res = Command::new("xdg-open")
                    .arg(&path)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                if res.is_ok() {
                    self.status_msg = format!(
                        "Opened profile directory: {}",
                        compact_path(&path, &self.home_dir)
                    );
                } else {
                    self.status_msg =
                        format!("Profile directory: {}", compact_path(&path, &self.home_dir));
                }
            }
        }
    }

    /// Update `filtered_indices` based on `filter_query`.
    fn update_filter(&mut self) {
        let query = self.filter_query.to_lowercase();
        self.filtered_indices = self
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, p)| query.is_empty() || p.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();

        if self.filtered_indices.is_empty() {
            self.state.select(None);
        } else {
            let cur = self.state.selected().unwrap_or(0);
            if cur >= self.filtered_indices.len() {
                self.state.select(Some(self.filtered_indices.len() - 1));
            } else {
                self.state.select(Some(cur));
            }
        }
        self.detail_scroll = 0;
    }

    /// Audit live symlink status for a target within a profile.
    fn check_target_status(&self, profile_name: &str, target: &Target) -> LinkStatus {
        let live_path = self.home_dir.join(&target.home_rel_path);
        let expected_source = self
            .base_dir
            .join("profiles")
            .join(profile_name)
            .join(&target.store_name);

        match fs::symlink_metadata(&live_path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    match fs::read_link(&live_path) {
                        Ok(dest) => {
                            if dest == expected_source {
                                LinkStatus::Active
                            } else {
                                LinkStatus::Other
                            }
                        }
                        Err(_) => LinkStatus::Broken,
                    }
                } else {
                    LinkStatus::Local
                }
            }
            Err(_) => LinkStatus::Missing,
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

        self.update_filter();
        Ok(())
    }

    fn next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => (i + 1) % self.filtered_indices.len(),
            None => 0,
        };
        self.state.select(Some(i));
        self.detail_scroll = 0;
    }

    fn previous(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered_indices.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.detail_scroll = 0;
    }

    /// Remove any symlinks in ~/.config or ~ that point into hyprdeck profiles.
    /// Returns the removed `(link, destination)` pairs so a failed switch can restore them.
    fn clean_profile_symlinks(&self) -> Vec<(PathBuf, PathBuf)> {
        let profiles_dir = self.base_dir.join("profiles");
        let mut removed = Vec::new();

        for dir in [self.home_dir.join(".config"), self.home_dir.clone()] {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_link = fs::symlink_metadata(&path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if !is_link {
                    continue;
                }
                if let Ok(dest) = fs::read_link(&path) {
                    if dest.starts_with(&profiles_dir) && fs::remove_file(&path).is_ok() {
                        removed.push((path, dest));
                    }
                }
            }
        }
        removed
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

        // 1. If import source has a .config subfolder, import all its entries
        let dot_config = self.pending_import_path.join(".config");
        if dot_config.is_dir() {
            if let Ok(entries) = fs::read_dir(&dot_config) {
                for entry in entries.flatten() {
                    if let Some(folder_name) = entry.file_name().to_str() {
                        if !folder_name.starts_with('.') && !is_browser_dir(folder_name) {
                            let target_dest = new_profile_dir.join(folder_name);
                            if Self::copy_entry(&entry.path(), &target_dest, &new_profile_dir)? {
                                imported_items.insert(folder_name.to_string());
                            }
                        }
                    }
                }
            }
        }

        // 2. Import all items from root of the external folder
        let ignored_root = [
            ".git",
            ".github",
            ".gitignore",
            ".config",
            "docs",
            "assets",
            "packaging",
            "target",
            "LICENSE",
            "LICENSE.md",
            "README",
            "README.md",
        ];

        if let Ok(entries) = fs::read_dir(&self.pending_import_path) {
            for entry in entries.flatten() {
                if let Some(item_name) = entry.file_name().to_str() {
                    if ignored_root.contains(&item_name) || is_browser_dir(item_name) {
                        continue;
                    }
                    if item_name == "reload.sh" {
                        let _ = fs::copy(entry.path(), new_profile_dir.join("reload.sh"));
                        continue;
                    }
                    // Skip media/archives unless they are dotfiles
                    if !item_name.starts_with('.') {
                        let lower = item_name.to_lowercase();
                        if lower.ends_with(".png")
                            || lower.ends_with(".jpg")
                            || lower.ends_with(".jpeg")
                            || lower.ends_with(".zip")
                            || lower.ends_with(".tar.gz")
                            || lower.ends_with(".tar.zst")
                            || lower.ends_with(".bak")
                        {
                            continue;
                        }
                    }

                    if !imported_items.contains(item_name) {
                        let target_dest = new_profile_dir.join(item_name);
                        if Self::copy_entry(&entry.path(), &target_dest, &new_profile_dir)? {
                            imported_items.insert(item_name.to_string());
                        }
                    }
                }
            }
        }

        if imported_items.is_empty() {
            let _ = fs::remove_dir_all(&new_profile_dir);
            self.status_msg = format!(
                "Import failed: No recognizable dotfiles or configs found in '{}'!",
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
        if target_dest.exists() || fs::symlink_metadata(target_dest).is_ok() {
            let _ = fs::remove_dir_all(target_dest);
            let _ = fs::remove_file(target_dest);
        }

        if let Ok(meta) = fs::symlink_metadata(src) {
            if meta.file_type().is_symlink() {
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
            } else if meta.is_dir() {
                if let Err(e) = copy_dir_all(src, target_dest, src) {
                    let _ = fs::remove_dir_all(rollback_dir);
                    return Err(e.into());
                }
                return Ok(true);
            } else if meta.is_file() {
                if let Err(e) = fs::copy(src, target_dest) {
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

        let mut copied_items = BTreeSet::new();

        // 1. Snapshot all known targets that exist on system
        for target in &self.known_targets {
            let live_path = self.home_dir.join(&target.home_rel_path);
            let target_dest = new_profile_dir.join(&target.store_name);

            if (live_path.exists() || fs::symlink_metadata(&live_path).is_ok())
                && Self::copy_entry(&live_path, &target_dest, &new_profile_dir)?
            {
                copied_items.insert(target.store_name.clone());
            }
        }

        // 2. Discover and snapshot all other configs in ~/.config
        let excluded_dirs = [
            "google-chrome",
            "BraveSoftware",
            "chromium",
            "mozilla",
            "librewolf",
            "epiphany",
            "dconf",
            "pulse",
            "evolution",
            "procps",
            "BetterDiscord",
            "Equicord",
            "Vencord",
            "legcord",
            "vesktop",
            "discord",
            "slack",
            "Claude",
            "obsidian",
            "zoom",
            "teams",
            "spotify",
            "systemd",
            "session",
            "user-dirs.dirs",
            "user-dirs.locale",
            "mimeapps.list",
            "monitors.xml",
            "kdeglobals",
            "kdenliverc",
            "kiorc",
            "trashrc",
            "filetypesrc",
            "baloofileinformationrc",
            "dolphinrc",
            "dolphin.kmessagebox",
            "QtProject",
            "QtProject.conf",
            "fcitx",
            "fcitx5",
            "ibus",
            "easyeffects",
            "libreoffice",
            "peazip",
            "GIMP",
            "audacity",
            "Blackmagic Design",
            "go",
            "Kitware",
            "rncbc.org",
            "Softdeluxe",
            "Unknown Organization",
            "gtk-backup",
            "pavucontrol.ini",
        ];

        let dot_config = self.home_dir.join(".config");
        if let Ok(entries) = fs::read_dir(&dot_config) {
            for entry in entries.flatten() {
                if let Some(folder_name) = entry.file_name().to_str() {
                    if copied_items.contains(folder_name)
                        || folder_name.starts_with('.')
                        || is_browser_dir(folder_name)
                    {
                        continue;
                    }
                    let path = entry.path();
                    let is_symlink = fs::symlink_metadata(&path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);

                    // Include if it's a symlink (user explicitly linked it as dotfile) OR not in exclusion list
                    if is_symlink || !excluded_dirs.contains(&folder_name) {
                        let target_dest = new_profile_dir.join(folder_name);
                        if Self::copy_entry(&path, &target_dest, &new_profile_dir)? {
                            copied_items.insert(folder_name.to_string());
                        }
                    }
                }
            }
        }

        // 3. User shell dotfiles in home
        for dotfile in [
            ".zshrc",
            ".zshenv",
            ".zprofile",
            ".bashrc",
            ".bash_profile",
            ".bash_logout",
            ".tmux.conf",
            ".gitconfig",
        ] {
            if !copied_items.contains(dotfile) {
                let live_path = self.home_dir.join(dotfile);
                if live_path.is_file() || fs::symlink_metadata(&live_path).is_ok() {
                    let target_dest = new_profile_dir.join(dotfile);
                    if Self::copy_entry(&live_path, &target_dest, &new_profile_dir)? {
                        copied_items.insert(dotfile.to_string());
                    }
                }
            }
        }

        if copied_items.is_empty() {
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
                name,
                copied_items.len()
            );
        }

        Ok(())
    }

    fn delete_selected_profile(&mut self) -> Result<()> {
        let Some(profile_name) = self.selected_profile().cloned() else {
            return Ok(());
        };
        let profile_path = self.base_dir.join("profiles").join(&profile_name);

        self.clean_profile_symlinks();

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

    /// Visit every non-comment line of the text files below `dir` (lowercased).
    fn for_each_config_line(dir: &Path, depth: usize, f: &mut dyn FnMut(&str)) {
        if depth > 8 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                Self::for_each_config_line(&path, depth + 1, f);
            } else if meta.is_file() && meta.len() < 2 * 1024 * 1024 {
                if let Ok(content) = fs::read_to_string(&path) {
                    for line in content.lines() {
                        let line = line.trim().to_lowercase();
                        if line.is_empty()
                            || line.starts_with('#')
                            || line.starts_with("//")
                            || line.starts_with("--")
                        {
                            continue;
                        }
                        f(&line);
                    }
                }
            }
        }
    }

    /// True when `needle` appears in `line` as a whole word (so `qs` does not match `tags`).
    fn line_has_word(line: &str, needle: &str) -> bool {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut from = 0;
        while let Some(pos) = line[from..].find(needle) {
            let start = from + pos;
            let end = start + needle.len();
            let before_ok = !line[..start].chars().next_back().is_some_and(is_word);
            let after_ok = !line[end..].chars().next().is_some_and(is_word);
            if before_ok && after_ok {
                return true;
            }
            from = start + needle.len().max(1);
        }
        false
    }

    /// Search active (non-comment) config lines for a keyword as a whole word.
    fn dir_mentions_recursive(dir: &Path, keyword: &str) -> bool {
        let needle = keyword.to_lowercase();
        let mut found = false;
        Self::for_each_config_line(dir, 0, &mut |line| {
            found = found || Self::line_has_word(line, &needle);
        });
        found
    }

    /// Like `dir_mentions_recursive`, but only for lines that launch something.
    fn dir_execs_recursive(dir: &Path, keyword: &str) -> bool {
        let needle = keyword.to_lowercase();
        let mut found = false;
        Self::for_each_config_line(dir, 0, &mut |line| {
            found = found
                || ((line.contains("exec") || line.contains("spawn"))
                    && Self::line_has_word(line, &needle));
        });
        found
    }

    /// Find `-c <name>` / `--config <name>` passed to quickshell (`qs`) in launch lines.
    fn quickshell_config_from_hypr(dir: &Path) -> Option<String> {
        let mut result = None;
        Self::for_each_config_line(dir, 0, &mut |line| {
            if result.is_some() || !(line.contains("exec") || line.contains("spawn")) {
                return;
            }
            let tokens: Vec<&str> = line
                .split(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | ',' | '(' | ')'))
                .filter(|t| !t.is_empty())
                .collect();
            for (i, tok) in tokens.iter().enumerate() {
                if matches!(*tok, "qs" | "quickshell") {
                    let mut j = i + 1;
                    while j + 1 < tokens.len() && tokens[j] != "-c" && tokens[j] != "--config" {
                        if !tokens[j].starts_with('-') {
                            break;
                        }
                        j += 1;
                    }
                    if j + 1 < tokens.len() && (tokens[j] == "-c" || tokens[j] == "--config") {
                        result = Some(tokens[j + 1].to_string());
                        return;
                    }
                }
            }
        });
        result
    }

    /// Decide how to launch quickshell for a profile: the config name for `-c`, if any.
    fn quickshell_config_name(profile_path: &Path) -> Option<String> {
        if let Some(name) = Self::quickshell_config_from_hypr(&profile_path.join("hypr")) {
            return Some(name);
        }
        for module in ["quickshell", "qs"] {
            let dir = profile_path.join(module);
            if dir.join("shell.qml").is_file() {
                return None; // default config; plain `qs` finds it
            }
            let named: Vec<String> = fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().join("shell.qml").is_file())
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .collect();
            if named.len() == 1 {
                return Some(named[0].clone());
            }
        }
        None
    }

    /// Launch a command detached and wait briefly to confirm one of `procs` is running.
    fn start_checked(cmd: &str, procs: &[&str]) -> bool {
        Self::spawn_detached(cmd);
        for _ in 0..10 {
            thread::sleep(Duration::from_millis(200));
            if procs.iter().any(|p| Self::is_proc_alive(p)) {
                return true;
            }
        }
        false
    }

    /// Live-reload Noctalia when possible, then try progressively simpler launches.
    fn reload_or_spawn_noctalia(&self) -> String {
        if Self::is_proc_alive("noctalia")
            && Self::command_exists("noctalia")
            && Self::run_silent("noctalia", &["msg", "config-reload"])
        {
            return "noctalia msg config-reload".to_string();
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

    /// Stop conflicting daemons and start/reload the services selected by a profile.
    fn reload_ecosystem_daemons(&mut self, profile_name: &str) {
        let profile_path = self.base_dir.join("profiles").join(profile_name);
        let targets = self.get_profile_targets(profile_name);
        let has_module = |name: &str| targets.iter().any(|t| t.store_name == name);

        let mut actions = Vec::new();

        // 1. Reload Hyprland Core WM
        Self::run_silent("hyprctl", &["reload"]);
        actions.push("hyprctl reload".to_string());

        let hypr_dir = profile_path.join("hypr");
        let hypr_mentions = |keyword: &str| Self::dir_mentions_recursive(&hypr_dir, keyword);

        // 2. Identify Shell / Bar
        // Quickshell is last: Noctalia and Caelestia are quickshell configs, and
        // their launch lines mention `qs`/`quickshell` too.
        let candidate_shells = [
            "noctalia",
            "caelestia",
            "hyprpanel",
            "ags",
            "ironbar",
            "eww",
            "waybar",
            "sfwbar",
            "yambar",
            "polybar",
            "tint2",
            "quickshell",
        ];
        let aliases = |shell: &str| -> Vec<&'static str> {
            match shell {
                "noctalia" => vec!["noctalia", "noctalia-shell"],
                "caelestia" => vec!["caelestia", "calestia"],
                "quickshell" => vec!["quickshell", "qs"],
                "ags" => vec!["ags", "astal"],
                "waybar" => vec!["waybar"],
                "hyprpanel" => vec!["hyprpanel"],
                "ironbar" => vec!["ironbar"],
                "eww" => vec!["eww"],
                "sfwbar" => vec!["sfwbar"],
                "yambar" => vec!["yambar"],
                "polybar" => vec!["polybar"],
                _ => vec!["tint2"],
            }
        };

        // Prefer what the Hyprland config actually launches, then profile modules,
        // then any mention at all.
        let wanted_shell = candidate_shells
            .iter()
            .copied()
            .find(|&sh| {
                aliases(sh)
                    .iter()
                    .any(|a| Self::dir_execs_recursive(&hypr_dir, a))
            })
            .or_else(|| {
                candidate_shells
                    .iter()
                    .copied()
                    .find(|&sh| aliases(sh).iter().any(|a| has_module(a)))
            })
            .or_else(|| {
                candidate_shells
                    .iter()
                    .copied()
                    .find(|&sh| aliases(sh).iter().any(|a| hypr_mentions(a)))
            });

        // Every process any shell/bar may run as. Quickshell-based shells show up as
        // `quickshell` or `qs` depending on how they were launched.
        let all_bar_procs = [
            "waybar",
            "ags",
            "gjs",
            "ironbar",
            "hyprpanel",
            "caelestia",
            "noctalia",
            "noctalia-shell",
            "quickshell",
            "qs",
            "sfwbar",
            "yambar",
            "polybar",
            "tint2",
        ];

        // Processes of the wanted shell that can be reloaded in place; all others die.
        // Quickshell-family shells are always restarted so the new config is loaded.
        let keep: &[&str] = match wanted_shell {
            Some("noctalia") => &["noctalia", "noctalia-shell"],
            Some("waybar") => &["waybar"],
            _ => &[],
        };
        let procs_to_kill: Vec<&str> = all_bar_procs
            .iter()
            .copied()
            .filter(|p| !keep.contains(p))
            .collect();
        Self::kill_and_wait(&procs_to_kill);

        if wanted_shell != Some("eww") && Self::command_exists("eww") {
            Self::run_silent("eww", &["kill"]);
        }

        // Launch or reload wanted shell
        match wanted_shell {
            Some("noctalia") => {
                let msg = self.reload_or_spawn_noctalia();
                actions.push(msg);
            }
            Some("waybar") => {
                if Self::command_exists("waybar") {
                    if Self::is_proc_alive("waybar") {
                        Self::run_silent("pkill", &["-USR2", "waybar"]);
                        actions.push("reloaded waybar".to_string());
                    } else {
                        Self::spawn_detached("waybar");
                        actions.push("spawned waybar".to_string());
                    }
                }
            }
            Some("ags") => {
                if Self::command_exists("ags") {
                    Self::run_silent("ags", &["quit"]);
                    Self::spawn_detached("ags");
                    actions.push("spawned ags".to_string());
                }
            }
            Some("caelestia") => {
                let ok = if Self::command_exists("caelestia") {
                    Self::start_checked("caelestia shell -d", &["quickshell", "qs"])
                } else if Self::command_exists("qs") {
                    Self::start_checked("qs -c caelestia", &["quickshell", "qs"])
                } else {
                    false
                };
                actions.push(if ok {
                    "restarted caelestia".to_string()
                } else {
                    "failed to start caelestia".to_string()
                });
            }
            Some("quickshell") => {
                let bin = ["qs", "quickshell"]
                    .into_iter()
                    .find(|b| Self::command_exists(b));
                match bin {
                    Some(bin) => {
                        let cmd = match Self::quickshell_config_name(&profile_path) {
                            Some(name) => format!("{} -c {}", bin, name),
                            None => bin.to_string(),
                        };
                        if Self::start_checked(&cmd, &["quickshell", "qs"]) {
                            actions.push(format!("restarted {}", cmd));
                        } else {
                            actions.push(format!("failed to start `{}`", cmd));
                        }
                    }
                    None => actions.push("quickshell not installed".to_string()),
                }
            }
            Some("eww") => {
                if Self::command_exists("eww") {
                    Self::spawn_detached("eww daemon");
                    actions.push("spawned eww daemon".to_string());
                }
            }
            Some(other) if Self::command_exists(other) => {
                Self::spawn_detached(other);
                actions.push(format!("spawned {}", other));
            }
            Some(_) => {}
            None => {}
        }

        // 3. Notification Daemons
        let notif_daemons = ["swaync", "dunst", "mako", "fnott", "wired"];
        let mut wanted_notif = None;
        for &daemon in &notif_daemons {
            if has_module(daemon) || hypr_mentions(daemon) {
                wanted_notif = Some(daemon);
                break;
            }
        }

        if let Some(wanted) = wanted_notif {
            let notifs_to_kill: Vec<&str> = notif_daemons
                .iter()
                .copied()
                .filter(|&d| d != wanted)
                .collect();
            Self::kill_and_wait(&notifs_to_kill);

            match wanted {
                "swaync" => {
                    if Self::command_exists("swaync") {
                        if Self::is_proc_alive("swaync") {
                            Self::run_silent("swaync-client", &["-R"]);
                            Self::run_silent("swaync-client", &["-rs"]);
                            actions.push("reloaded swaync".to_string());
                        } else {
                            Self::spawn_detached("swaync");
                            actions.push("spawned swaync".to_string());
                        }
                    }
                }
                "dunst" => {
                    if Self::command_exists("dunst") {
                        if Self::is_proc_alive("dunst") {
                            Self::run_silent("dunstctl", &["reload"]);
                            actions.push("reloaded dunst".to_string());
                        } else {
                            Self::spawn_detached("dunst");
                            actions.push("spawned dunst".to_string());
                        }
                    }
                }
                "mako" => {
                    if Self::command_exists("mako") {
                        if Self::is_proc_alive("mako") {
                            Self::run_silent("makoctl", &["reload"]);
                            actions.push("reloaded mako".to_string());
                        } else {
                            Self::spawn_detached("mako");
                            actions.push("spawned mako".to_string());
                        }
                    }
                }
                "fnott" => {
                    if Self::command_exists("fnott") {
                        Self::spawn_detached("fnott");
                        actions.push("spawned fnott".to_string());
                    }
                }
                "wired" if Self::command_exists("wired") => {
                    Self::spawn_detached("wired");
                    actions.push("spawned wired".to_string());
                }
                _ => {}
            }
        }

        // 4. Wallpaper Daemons
        let wallpaper_daemons = ["swww", "hyprpaper", "wpaperd", "mpvpaper", "swaybg"];
        let mut wanted_wallpaper = None;
        for &daemon in &wallpaper_daemons {
            if has_module(daemon) || hypr_mentions(daemon) {
                wanted_wallpaper = Some(daemon);
                break;
            }
        }

        if let Some(wp) = wanted_wallpaper {
            match wp {
                "swww" => {
                    if Self::command_exists("swww") {
                        if Self::is_proc_alive("swww-daemon") {
                            Self::run_silent("swww", &["restore"]);
                            actions.push("swww restore".to_string());
                        } else {
                            Self::spawn_detached("swww-daemon");
                            actions.push("spawned swww-daemon".to_string());
                        }
                    }
                }
                "hyprpaper" => {
                    if Self::command_exists("hyprpaper") {
                        if Self::is_proc_alive("hyprpaper") {
                            Self::run_silent("hyprctl", &["hyprpaper", "reload"]);
                            actions.push("reloaded hyprpaper".to_string());
                        } else {
                            Self::spawn_detached("hyprpaper");
                            actions.push("spawned hyprpaper".to_string());
                        }
                    }
                }
                "wpaperd" => {
                    if Self::command_exists("wpaperd") {
                        Self::kill_and_wait(&["wpaperd"]);
                        Self::spawn_detached("wpaperd");
                        actions.push("spawned wpaperd".to_string());
                    }
                }
                "mpvpaper" => {
                    if Self::command_exists("mpvpaper") {
                        Self::kill_and_wait(&["mpvpaper"]);
                        actions.push("stopped mpvpaper".to_string());
                    }
                }
                "swaybg" if Self::command_exists("swaybg") => {
                    Self::kill_and_wait(&["swaybg"]);
                    Self::spawn_detached("swaybg");
                    actions.push("spawned swaybg".to_string());
                }
                _ => {}
            }
        }

        // 5. OSD & Nightlight
        if (has_module("swayosd") || hypr_mentions("swayosd"))
            && Self::command_exists("swayosd-server")
            && !Self::is_proc_alive("swayosd-server")
        {
            Self::spawn_detached("swayosd-server");
            actions.push("spawned swayosd-server".to_string());
        }

        // 6. Terminals Hot-Reload
        if has_module("kitty") {
            Self::run_silent("killall", &["-SIGUSR1", "kitty"]);
            actions.push("reloaded kitty".to_string());
        }
        if has_module("foot") {
            Self::run_silent("killall", &["-SIGUSR1", "foot"]);
            actions.push("reloaded foot".to_string());
        }
        if has_module("alacritty")
            && Self::command_exists("alacritty")
            && Self::is_proc_alive("alacritty")
        {
            Self::run_silent("alacritty", &["msg", "config", "reload"]);
            actions.push("reloaded alacritty".to_string());
        }

        // 7. Theme Engines
        if (has_module("wal") || hypr_mentions("wal")) && Self::command_exists("wal") {
            Self::run_silent("wal", &["-R", "-q"]);
            actions.push("restored pywal".to_string());
        }
        if (has_module("wallust") || hypr_mentions("wallust")) && Self::command_exists("wallust") {
            Self::run_silent("wallust", &["run"]);
            actions.push("reloaded wallust".to_string());
        }

        // 8. Custom Hook
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
        let Some(profile_name) = self.selected_profile().cloned() else {
            return Ok(());
        };
        self.apply_profile(&profile_name)
    }

    /// Pick a backup path that never overwrites an earlier backup.
    fn unique_backup_path(backup_dir: &Path, store_name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut candidate = backup_dir.join(format!("{}_backup-{}", store_name, stamp));
        let mut n = 1;
        while fs::symlink_metadata(&candidate).is_ok() {
            candidate = backup_dir.join(format!("{}_backup-{}-{}", store_name, stamp, n));
            n += 1;
        }
        candidate
    }

    /// Re-link a profile's targets into the home directory and reload the desktop.
    /// If linking fails part way, every change is rolled back.
    fn apply_profile(&mut self, profile_name: &str) -> Result<()> {
        let profile_path = self.base_dir.join("profiles").join(profile_name);
        if !profile_path.is_dir() {
            anyhow::bail!("profile '{}' does not exist", profile_name);
        }
        let backup_dir = self.base_dir.join("backups");
        fs::create_dir_all(&backup_dir)?;

        let removed_links = self.clean_profile_symlinks();
        let mut created: Vec<PathBuf> = Vec::new();
        // (original location, where it was moved to)
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut replaced_links: Vec<(PathBuf, PathBuf)> = Vec::new();

        let result: Result<()> = (|| {
            for target in self.get_profile_targets(profile_name) {
                if is_browser_dir(&target.store_name) {
                    continue;
                }
                let source = profile_path.join(&target.store_name);
                let live_target = self.home_dir.join(&target.home_rel_path);

                if fs::symlink_metadata(&source).is_err() {
                    continue;
                }
                if let Some(parent) = live_target.parent() {
                    fs::create_dir_all(parent)?;
                }

                if let Ok(meta) = fs::symlink_metadata(&live_target) {
                    if meta.file_type().is_symlink() {
                        let dest = fs::read_link(&live_target)?;
                        fs::remove_file(&live_target)?;
                        replaced_links.push((live_target.clone(), dest));
                    } else {
                        let backup = Self::unique_backup_path(&backup_dir, &target.store_name);
                        fs::rename(&live_target, &backup)?;
                        moved.push((live_target.clone(), backup));
                    }
                }

                symlink(&source, &live_target)?;
                created.push(live_target);
            }
            Ok(())
        })();

        if let Err(err) = result {
            for link in created.iter().rev() {
                let _ = fs::remove_file(link);
            }
            for (orig, backup) in moved.iter().rev() {
                let _ = fs::rename(backup, orig);
            }
            for (link, dest) in replaced_links.iter().chain(removed_links.iter()) {
                let _ = symlink(dest, link);
            }
            return Err(err.context(format!(
                "switching to '{}' failed; previous setup restored",
                profile_name
            )));
        }

        // Unit tests must never signal or spawn real desktop processes.
        if !cfg!(test) {
            self.reload_ecosystem_daemons(profile_name);
        }

        self.active_profile = profile_name.to_string();
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

/// Return a centered rectangle for a modal with exact dimensions.
fn centered_rect_exact(width: u16, height: u16, r: Rect) -> Rect {
    let x = r.x + r.width.saturating_sub(width) / 2;
    let y = r.y + r.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(r.width),
        height: height.min(r.height),
    }
}

const USAGE: &str = "hyprdeck - Hyprland profile and dotfile manager

USAGE:
    hyprdeck                 Open the interactive TUI
    hyprdeck list            List profiles (active one is marked with *)
    hyprdeck current         Print the active profile
    hyprdeck switch <name>   Activate a profile and reload services
    hyprdeck --version       Print the version
    hyprdeck --help          Print this help";

/// Handle non-interactive subcommands. Returns `Ok(true)` if one was handled.
fn run_cli(args: &[String]) -> Result<bool> {
    let Some(cmd) = args.first() else {
        return Ok(false);
    };
    match cmd.as_str() {
        "-h" | "--help" | "help" => println!("{}", USAGE),
        "-V" | "--version" | "version" => println!("hyprdeck {}", env!("CARGO_PKG_VERSION")),
        "list" => {
            let app = App::new()?;
            for p in &app.profiles {
                let mark = if *p == app.active_profile { "*" } else { " " };
                println!("{} {}", mark, p);
            }
        }
        "current" => println!("{}", App::new()?.active_profile),
        "switch" => {
            let name = args.get(1).context("usage: hyprdeck switch <name>")?;
            let mut app = App::new()?;
            app.apply_profile(name)?;
            println!("{}", app.status_msg);
        }
        other => anyhow::bail!("unknown command '{}'\n\n{}", other, USAGE),
    }
    Ok(true)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if run_cli(&args)? {
        return Ok(());
    }

    // Release builds abort on panic; put the terminal back first.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = App::new()?;
    let res = run_app(&mut terminal, &mut app);

    restore_terminal();
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
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Esc => {
                        if !app.filter_query.is_empty() {
                            app.filter_query.clear();
                            app.update_filter();
                            app.status_msg = String::from("Filter cleared.");
                        } else {
                            return Ok(());
                        }
                    }
                    KeyCode::Tab => {
                        app.active_pane = match app.active_pane {
                            ActivePane::Profiles => ActivePane::Details,
                            ActivePane::Details => ActivePane::Profiles,
                        };
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        app.active_pane = ActivePane::Profiles;
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        app.active_pane = ActivePane::Details;
                    }
                    KeyCode::Down | KeyCode::Char('j') => match app.active_pane {
                        ActivePane::Profiles => app.next(),
                        ActivePane::Details => {
                            let max_scroll = app.detail_lines_count.saturating_sub(5) as u16;
                            app.detail_scroll = (app.detail_scroll + 1).min(max_scroll);
                        }
                    },
                    KeyCode::Up | KeyCode::Char('k') => match app.active_pane {
                        ActivePane::Profiles => app.previous(),
                        ActivePane::Details => {
                            app.detail_scroll = app.detail_scroll.saturating_sub(1);
                        }
                    },
                    KeyCode::PageDown | KeyCode::Char('J') => {
                        let max_scroll = app.detail_lines_count.saturating_sub(5) as u16;
                        app.detail_scroll = (app.detail_scroll + 5).min(max_scroll);
                    }
                    KeyCode::PageUp | KeyCode::Char('K') => {
                        app.detail_scroll = app.detail_scroll.saturating_sub(5);
                    }
                    KeyCode::Home | KeyCode::Char('g') => match app.active_pane {
                        ActivePane::Profiles => {
                            if !app.filtered_indices.is_empty() {
                                app.state.select(Some(0));
                                app.detail_scroll = 0;
                            }
                        }
                        ActivePane::Details => {
                            app.detail_scroll = 0;
                        }
                    },
                    KeyCode::End | KeyCode::Char('G') => match app.active_pane {
                        ActivePane::Profiles => {
                            if !app.filtered_indices.is_empty() {
                                app.state.select(Some(app.filtered_indices.len() - 1));
                                app.detail_scroll = 0;
                            }
                        }
                        ActivePane::Details => {
                            let max_scroll = app.detail_lines_count.saturating_sub(5) as u16;
                            app.detail_scroll = max_scroll;
                        }
                    },
                    KeyCode::Char('/') => {
                        app.input_mode = InputMode::Filter;
                        app.status_msg =
                            String::from("Type to filter profiles... [Enter] confirm, [Esc] clear");
                    }
                    KeyCode::Char('?') => {
                        app.input_mode = InputMode::Help;
                    }
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
                        if let Some(name) = app.selected_profile().cloned() {
                            app.input_mode = InputMode::ConfirmDelete;
                            app.status_msg = format!(
                                "Confirm deleting profile '{}'? Press [y] to delete or [n]/[Esc] to cancel.",
                                name
                            );
                        }
                    }
                    KeyCode::Char('r') => {
                        if let Some(name) = app.selected_profile().cloned() {
                            app.reload_ecosystem_daemons(&name);
                            app.status_msg = format!(
                                "Force-reloaded daemons for '{}': {}",
                                name, app.last_reload_summary
                            );
                        }
                    }
                    KeyCode::Char('o') => {
                        app.open_profile_folder();
                    }
                    KeyCode::Enter => {
                        if let Err(e) = app.apply_selected_profile() {
                            app.status_msg = format!("Error: {}", e);
                        }
                    }
                    _ => {}
                },
                InputMode::Filter => match key.code {
                    KeyCode::Enter => {
                        app.input_mode = InputMode::Normal;
                        app.status_msg = format!(
                            "Filter applied ({} match{}). Press [Esc] to clear.",
                            app.filtered_indices.len(),
                            if app.filtered_indices.len() == 1 {
                                ""
                            } else {
                                "es"
                            }
                        );
                    }
                    KeyCode::Esc => {
                        app.filter_query.clear();
                        app.update_filter();
                        app.input_mode = InputMode::Normal;
                        app.status_msg = String::from("Filter cleared.");
                    }
                    KeyCode::Backspace => {
                        app.filter_query.pop();
                        app.update_filter();
                    }
                    KeyCode::Char(c) => {
                        app.filter_query.push(c);
                        app.update_filter();
                    }
                    KeyCode::Down => app.next(),
                    KeyCode::Up => app.previous(),
                    _ => {}
                },
                InputMode::Help => match key.code {
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {
                        app.input_mode = InputMode::Normal;
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

/// Top header bar: gradient brand mark, active profile pill, and live system status.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_BORDER_DIM));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 10 || inner.height == 0 {
        return;
    }

    let mut left = vec![
        Span::raw(" "),
        Span::styled(
            "◈ ",
            Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
        ),
    ];
    left.extend(gradient("HYPRDECK", PURPLE_RGB, BLUE_RGB, true));
    left.push(Span::styled(" · ", Style::default().fg(C_DIM)));
    left.push(Span::styled(
        "Wayland Profile & Dotfile Engine",
        Style::default().fg(C_MUTED),
    ));

    let managed = !app.active_profile.contains("External")
        && !app.active_profile.contains("None")
        && !app.active_profile.is_empty();
    let mut right = Vec::new();

    if managed {
        right.push(Span::styled(
            " ● ACTIVE: ",
            Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
        ));
        right.push(Span::styled(
            format!("{} ", app.active_profile),
            Style::default()
                .fg(C_TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ));
        right.push(Span::styled("│ ", Style::default().fg(C_DIM)));
    } else {
        right.push(Span::styled(" ○ UNMANAGED ", Style::default().fg(C_WARN)));
        right.push(Span::styled("│ ", Style::default().fg(C_DIM)));
    }

    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        right.push(Span::styled("󱞪 Hyprland ", Style::default().fg(C_CYAN)));
        right.push(Span::styled("│ ", Style::default().fg(C_DIM)));
    }

    let count = app.profiles.len();
    right.push(Span::styled(
        format!("{} profile{} ", count, if count == 1 { "" } else { "s" }),
        Style::default().fg(C_MUTED),
    ));

    let right_width = right.iter().map(|s| s.width()).sum::<usize>() as u16 + 2;
    let cols = Layout::horizontal([
        Constraint::Min(20),
        Constraint::Length(right_width.min(inner.width)),
    ])
    .split(inner);

    frame.render_widget(Paragraph::new(Line::from(left)), cols[0]);
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        cols[1],
    );
}

/// Main workspace split: responsive sidebar and rich inspector.
fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.width < 50 || area.height < 5 {
        render_profile_list(frame, area, app);
        return;
    }

    let sidebar_w = (area.width / 100 * 32).clamp(28, 42).min(area.width - 24);
    let cols = Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(24)]).split(area);

    render_sidebar(frame, cols[0], app);
    render_details(frame, cols[1], app);
}

/// Sidebar wrapper: profile navigation list and search.
fn render_sidebar(frame: &mut Frame, area: Rect, app: &mut App) {
    render_profile_list(frame, area, app);
}

/// Profile list with real-time search filtering, status indicators, and module count badges.
fn render_profile_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.active_pane == ActivePane::Profiles;
    let (border_color, title_style) = if focused {
        (
            C_BLUE,
            Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
        )
    } else {
        (C_BORDER_DIM, Style::default().fg(C_MUTED))
    };

    let count_str = if app.filter_query.is_empty() {
        format!("· {} ", app.profiles.len())
    } else {
        format!("· {}/{} ", app.filtered_indices.len(), app.profiles.len())
    };

    let title = Line::from(vec![
        Span::styled(" 󰁤 Profiles ", title_style),
        Span::styled(count_str, Style::default().fg(C_DIM)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let show_filter_box = app.input_mode == InputMode::Filter || !app.filter_query.is_empty();

    let list_area = if show_filter_box {
        let chunks = Layout::vertical([
            Constraint::Length(1), // Filter bar
            Constraint::Length(1), // Divider
            Constraint::Min(1),    // List
        ])
        .split(inner);

        let filter_line = Line::from(vec![
            Span::styled(" 🔍 ", Style::default().fg(C_BLUE)),
            Span::styled(
                if app.filter_query.is_empty() {
                    "type to filter..."
                } else {
                    &app.filter_query
                },
                if app.filter_query.is_empty() {
                    Style::default().fg(C_DIM)
                } else {
                    Style::default()
                        .fg(C_TEXT_BRIGHT)
                        .add_modifier(Modifier::BOLD)
                },
            ),
            if app.input_mode == InputMode::Filter {
                Span::styled("█", Style::default().fg(C_CYAN))
            } else {
                Span::raw("")
            },
        ]);
        frame.render_widget(Paragraph::new(filter_line), chunks[0]);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(chunks[1].width as usize),
                Style::default().fg(C_BORDER_DIM),
            ))),
            chunks[1],
        );

        chunks[2]
    } else {
        inner
    };

    if app.filtered_indices.is_empty() {
        let empty_msg = if !app.filter_query.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No matching profiles.",
                    Style::default().fg(C_WARN),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("[Esc]", Style::default().fg(C_BLUE)),
                    Span::styled(" clear search filter", Style::default().fg(C_MUTED)),
                ]),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No profiles saved yet.",
                    Style::default().fg(C_TEXT),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("[s]", Style::default().fg(C_BLUE)),
                    Span::styled(" snapshot live setup", Style::default().fg(C_MUTED)),
                ]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("[i]", Style::default().fg(C_BLUE)),
                    Span::styled(" import dotfiles folder", Style::default().fg(C_MUTED)),
                ]),
            ]
        };
        frame.render_widget(Paragraph::new(empty_msg), list_area);
        return;
    }

    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .map(|&idx| {
            let name = &app.profiles[idx];
            let active = *name == app.active_profile;
            let target_count = app.get_profile_targets(name).len();

            let (marker, marker_style) = if active {
                (
                    " ● ",
                    Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
                )
            } else {
                (" ○ ", Style::default().fg(C_DIM))
            };

            let name_style = if active {
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(C_TEXT)
            };

            let count_pill = Span::styled(
                format!(" [{} cfgs] ", target_count),
                Style::default().fg(C_DIM),
            );

            ListItem::new(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(name.clone(), name_style),
                Span::raw(" "),
                count_pill,
            ]))
        })
        .collect();

    let highlight_style = if focused {
        Style::default()
            .bg(C_BG_SEL)
            .fg(C_TEXT_BRIGHT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(C_SLATE).fg(C_TEXT)
    };

    let list = List::new(items)
        .highlight_style(highlight_style)
        .highlight_symbol("▎");

    frame.render_stateful_widget(list, list_area, &mut app.state);
}

/// Rich details inspector panel with live symlink audit, daemon pipelines, and scrollbar.
fn render_details(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.active_pane == ActivePane::Details;
    let label = match app.selected_profile() {
        Some(name) => name.clone(),
        None => String::from("welcome"),
    };

    let (border_color, title_style) = if focused {
        (
            C_PURPLE,
            Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
        )
    } else {
        (C_BORDER_DIM, Style::default().fg(C_MUTED))
    };

    let title = Line::from(vec![
        Span::styled(" 󰋼 Details ", title_style),
        Span::styled(format!("· {} ", label), Style::default().fg(C_DIM)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = match app.selected_profile().cloned() {
        Some(selected_name) => profile_details(app, &selected_name),
        None => welcome_details(app, inner.width as usize),
    };

    app.detail_lines_count = lines.len();

    frame.render_widget(
        Paragraph::new(lines)
            .scroll((app.detail_scroll, 0))
            .wrap(Wrap { trim: false }),
        inner,
    );

    if app.detail_lines_count > inner.height as usize {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state =
            ScrollbarState::new(app.detail_lines_count.saturating_sub(inner.height as usize))
                .position(app.detail_scroll as usize);

        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn profile_details(app: &App, profile: &str) -> Vec<Line<'static>> {
    let profile_path = app.base_dir.join("profiles").join(profile);
    let targets = app.get_profile_targets(profile);
    let is_active = *profile == app.active_profile;
    let has_hook = profile_path.join("reload.sh").exists();

    let mut lines = Vec::new();

    // 1. Profile Title & Badges
    lines.push(Line::from(""));
    let mut header_spans = vec![
        Span::raw("  "),
        Span::styled(
            format!(" {} ", profile),
            Style::default()
                .fg(C_TEXT_BRIGHT)
                .bg(C_BG_CARD)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    if is_active {
        header_spans.push(Span::styled(
            " ● ACTIVE ",
            Style::default()
                .fg(Color::Rgb(0x18, 0x18, 0x24))
                .bg(C_OK)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        header_spans.push(Span::styled(
            " ○ STANDBY ",
            Style::default().fg(C_MUTED).bg(C_SLATE),
        ));
    }
    header_spans.push(Span::raw(" "));
    header_spans.push(Span::styled(
        format!(" {} MODULES ", targets.len()),
        Style::default().fg(C_PURPLE).bg(C_SLATE),
    ));
    if has_hook {
        header_spans.push(Span::raw(" "));
        header_spans.push(Span::styled(
            " ⚡ HOOK ",
            Style::default().fg(C_CYAN).bg(C_SLATE),
        ));
    }
    lines.push(Line::from(header_spans));

    // Path
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled("📁 Location: ", Style::default().fg(C_MUTED)),
        Span::styled(
            compact_path(&profile_path, &app.home_dir),
            Style::default().fg(C_BLUE),
        ),
    ]));

    lines.push(Line::from(Span::styled(
        "  ────────────────────────────────────────────────────────────────────────",
        Style::default().fg(C_BORDER_DIM),
    )));
    lines.push(Line::from(""));

    // 2. Tracked Configurations
    lines.push(Line::from(vec![
        Span::styled(
            "  TRACKED CONFIGURATIONS & HEALTH",
            Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {} components", targets.len()),
            Style::default().fg(C_DIM),
        ),
    ]));

    if targets.is_empty() {
        lines.push(Line::from(Span::styled(
            "    (Empty profile — no configuration modules found)",
            Style::default().fg(C_DIM),
        )));
    } else {
        for target in &targets {
            let status = app.check_target_status(profile, target);
            let (status_badge, status_style) = match status {
                LinkStatus::Active => (
                    " ✓ Active Link ",
                    Style::default()
                        .fg(C_OK)
                        .bg(C_SLATE)
                        .add_modifier(Modifier::BOLD),
                ),
                LinkStatus::Other => (" ⇄ Other Link ", Style::default().fg(C_WARN).bg(C_SLATE)),
                LinkStatus::Local => (" ○ Local Copy ", Style::default().fg(C_CYAN).bg(C_SLATE)),
                LinkStatus::Broken => (
                    " ✖ Broken Link ",
                    Style::default()
                        .fg(C_ERR)
                        .bg(C_SLATE)
                        .add_modifier(Modifier::BOLD),
                ),
                LinkStatus::Missing => (" ○ Detached ", Style::default().fg(C_DIM).bg(C_SLATE)),
            };

            let cat_style = match target.category {
                "Core WM" => Style::default().fg(C_PURPLE),
                "Shell / Bar" => Style::default().fg(C_BLUE),
                "Terminal" => Style::default().fg(C_CYAN),
                "Notification" => Style::default().fg(C_WARN),
                "Theme / Style" => Style::default().fg(C_OK),
                "Shell / Prompt" => Style::default().fg(C_MUTED),
                _ => Style::default().fg(C_DIM),
            };

            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled("◈ ", Style::default().fg(C_BLUE)),
                Span::styled(
                    format!("{:<15}", target.store_name),
                    Style::default()
                        .fg(C_TEXT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<26}", format!("~/{}", target.home_rel_path)),
                    Style::default().fg(C_MUTED),
                ),
                Span::styled(format!("{:<17}", target.category), cat_style),
                Span::styled(status_badge, status_style),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ────────────────────────────────────────────────────────────────────────",
        Style::default().fg(C_BORDER_DIM),
    )));
    lines.push(Line::from(""));

    // 3. Daemon Orchestration & Hot Reload
    lines.push(Line::from(vec![
        Span::styled(
            "  DAEMON ORCHESTRATION PIPELINE",
            Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  automatic reload triggers", Style::default().fg(C_DIM)),
    ]));

    let target_names: BTreeSet<String> = targets.iter().map(|t| t.store_name.clone()).collect();
    let mut daemons = Vec::new();

    if target_names.contains("hypr") {
        daemons.push((
            "Hyprland",
            "hyprctl reload (smooth hot-reload without closing windows)",
        ));
    }
    if target_names.iter().any(|n| {
        n.contains("bar") || n.contains("shell") || n == "waybar" || n == "ags" || n == "quickshell"
    }) {
        daemons.push((
            "Desktop Shell",
            "Live daemon signal reload / automatic respawn",
        ));
    }
    if target_names.iter().any(|n| n == "kitty" || n == "foot") {
        daemons.push(("Terminal", "SIGUSR1 real-time config & color-scheme reload"));
    }
    if target_names
        .iter()
        .any(|n| n == "swaync" || n == "dunst" || n == "mako")
    {
        daemons.push((
            "Notifications",
            "Reload daemon style and notification server",
        ));
    }
    if target_names
        .iter()
        .any(|n| n == "wal" || n == "wallust" || n == "matugen")
    {
        daemons.push((
            "Palette / Theme",
            "Apply dynamically generated color schemes",
        ));
    }

    if daemons.is_empty() {
        lines.push(Line::from(Span::styled(
            "    Standard symlinks updated upon activation.",
            Style::default().fg(C_MUTED),
        )));
    } else {
        for (daemon, action) in daemons {
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled("❯ ", Style::default().fg(C_BLUE)),
                Span::styled(
                    format!("{:<16}", daemon),
                    Style::default().fg(C_TEXT_BRIGHT),
                ),
                Span::styled(action, Style::default().fg(C_MUTED)),
            ]));
        }
    }

    // Hook status
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled(
            "❯ ",
            Style::default().fg(if has_hook { C_OK } else { C_DIM }),
        ),
        Span::styled(
            format!("{:<16}", "Custom Hook"),
            Style::default().fg(if has_hook { C_OK } else { C_MUTED }),
        ),
        if has_hook {
            Span::styled(
                "reload.sh detected (will execute automatically after daemons)",
                Style::default().fg(C_OK),
            )
        } else {
            Span::styled(
                "none (optional: create reload.sh in profile to run custom scripts)",
                Style::default().fg(C_DIM),
            )
        },
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ────────────────────────────────────────────────────────────────────────",
        Style::default().fg(C_BORDER_DIM),
    )));
    lines.push(Line::from(""));

    // 4. Session Activity / Last Reload
    lines.push(Line::from(vec![
        Span::styled(
            "  SESSION ACTIVITY & AUDIT",
            Style::default().fg(C_CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  last executed operation", Style::default().fg(C_DIM)),
    ]));

    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled("❯ ", Style::default().fg(C_CYAN)),
        Span::styled(app.last_reload_summary.clone(), Style::default().fg(C_TEXT)),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            " [ Enter ] ",
            Style::default()
                .fg(C_BLUE)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Activate & hot-reload", Style::default().fg(C_MUTED)),
        Span::raw("    "),
        Span::styled(
            " [ o ] ",
            Style::default()
                .fg(C_PURPLE)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Open folder", Style::default().fg(C_MUTED)),
        Span::raw("    "),
        Span::styled(
            " [ r ] ",
            Style::default()
                .fg(C_CYAN)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Force reload", Style::default().fg(C_MUTED)),
    ]));
    lines.push(Line::from(""));

    lines
}

fn welcome_details(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(""));

    if width >= 26 {
        let logo_lines = logo::render(20, 8);
        for l in logo_lines {
            lines.push(l);
        }
    }

    lines.push(Line::from(""));
    let mut title_spans = vec![Span::raw("  ")];
    title_spans.extend(gradient("HYPRDECK", PURPLE_RGB, BLUE_RGB, true));
    title_spans.push(Span::styled(
        "  ·  Native Wayland Profile & Dotfile Engine",
        Style::default().fg(C_MUTED),
    ));
    lines.push(Line::from(title_spans));

    lines.push(Line::from(Span::styled(
        "  No profiles found yet in ~/.hyprdeck/profiles. Get started below:",
        Style::default().fg(C_TEXT),
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ────────────────────────────────────────────────────────────────────────",
        Style::default().fg(C_BORDER_DIM),
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "  QUICK START ACTIONS",
        Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
    )));

    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled(
            " [ s ] ",
            Style::default()
                .fg(C_BLUE)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Snapshot Live Setup  ",
            Style::default()
                .fg(C_TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Capture active ~/.config and dotfiles into a new profile",
            Style::default().fg(C_MUTED),
        ),
    ]));

    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled(
            " [ i ] ",
            Style::default()
                .fg(C_PURPLE)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Import Dotfiles       ",
            Style::default()
                .fg(C_TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Import existing repository or folder (e.g. ~/dotfiles)",
            Style::default().fg(C_MUTED),
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ────────────────────────────────────────────────────────────────────────",
        Style::default().fg(C_BORDER_DIM),
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled(
            "  DETECTED ECOSYSTEM ON HOST",
            Style::default().fg(C_CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  ·  configurations ready to snapshot",
            Style::default().fg(C_DIM),
        ),
    ]));

    let common_checks = [
        ("hypr", ".config/hypr", "Core WM"),
        ("waybar", ".config/waybar", "Shell / Bar"),
        ("kitty", ".config/kitty", "Terminal"),
        ("alacritty", ".config/alacritty", "Terminal"),
        ("swaync", ".config/swaync", "Notification"),
        ("dunst", ".config/dunst", "Notification"),
        ("wal", ".config/wal", "Theme / Style"),
        ("starship.toml", ".config/starship.toml", "Shell / Prompt"),
        (".zshrc", ".zshrc", "Shell / Prompt"),
        (".bashrc", ".bashrc", "Shell / Prompt"),
        ("rofi", ".config/rofi", "Launcher / Menu"),
        ("wofi", ".config/wofi", "Launcher / Menu"),
    ];

    for (name, rel_path, cat) in common_checks {
        let p = app.home_dir.join(rel_path);
        if p.exists() {
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled("✓ ", Style::default().fg(C_OK)),
                Span::styled(format!("{:<15}", name), Style::default().fg(C_TEXT_BRIGHT)),
                Span::styled(
                    format!("{:<26}", format!("~/{}", rel_path)),
                    Style::default().fg(C_MUTED),
                ),
                Span::styled(cat, Style::default().fg(C_DIM)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let (badge, style) = status_badge(&app.status_msg);

    let status_line = Line::from(vec![
        Span::raw("  "),
        badge,
        Span::raw(" "),
        Span::styled(app.status_msg.clone(), style),
    ]);

    let status_area = Rect { height: 1, ..area };
    frame.render_widget(Paragraph::new(status_line), status_area);

    if area.height < 2 {
        return;
    }

    let hints_area = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };

    let mut spans = vec![Span::raw("  ")];
    for (key, label) in [
        ("↵", "Apply"),
        ("s", "Snapshot"),
        ("i", "Import"),
        ("d", "Delete"),
        ("r", "Reload"),
        ("o", "Open"),
        ("/", "Filter"),
        ("Tab", "Pane"),
        ("?", "Help"),
        ("q", "Quit"),
    ] {
        spans.extend(key_hint(key, label));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), hints_area);
}

fn render_overlays(frame: &mut Frame, app: &App) {
    match app.input_mode {
        InputMode::Normal | InputMode::Filter => {}
        InputMode::SnapshotName => render_snapshot_modal(frame, app),
        InputMode::ImportPath => render_import_path_modal(frame, app),
        InputMode::ImportName => render_import_name_modal(frame, app),
        InputMode::ConfirmDelete => render_delete_modal(frame, app),
        InputMode::Help => render_help_modal(frame, app),
    }
}

fn render_snapshot_modal(frame: &mut Frame, app: &App) {
    let area = centered_rect_exact(62, 9, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD))
        .title(Line::from(vec![
            Span::styled(" 󰁤 ", Style::default().fg(C_BLUE)),
            Span::styled(
                "Create Profile Snapshot ",
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter a unique name for this profile snapshot:",
            Style::default().fg(C_MUTED),
        ))),
        chunks[0],
    );

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_BLUE));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ❯ ", Style::default().fg(C_BLUE)),
            Span::styled(
                app.input_buffer.clone(),
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(C_CYAN)),
        ])),
        input_inner,
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "Captures active ~/.config and user dotfiles into ~/.hyprdeck/profiles.",
            Style::default().fg(C_DIM),
        )])),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [↵] Save Snapshot ",
                Style::default()
                    .fg(C_BLUE)
                    .bg(C_SLATE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(" [Esc] Cancel ", Style::default().fg(C_MUTED).bg(C_SLATE)),
        ]))
        .alignment(Alignment::Right),
        chunks[3],
    );
}

fn render_import_path_modal(frame: &mut Frame, app: &App) {
    let area = centered_rect_exact(70, 10, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD))
        .title(Line::from(vec![
            Span::styled(" 󰋚 ", Style::default().fg(C_PURPLE)),
            Span::styled(
                "Import Dotfiles · Step 1/2 ",
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter source path to external dotfiles (e.g. ~/dotfiles or /repo):",
            Style::default().fg(C_MUTED),
        ))),
        chunks[0],
    );

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_PURPLE));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ❯ ", Style::default().fg(C_PURPLE)),
            Span::styled(
                app.input_buffer.clone(),
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(C_CYAN)),
        ])),
        input_inner,
    );

    let expanded = app.expand_tilde(&app.input_buffer);
    let validation = if expanded.exists() && expanded.is_dir() {
        Line::from(vec![
            Span::styled(" ● Directory exists: ", Style::default().fg(C_OK)),
            Span::styled(
                compact_path(&expanded, &app.home_dir),
                Style::default().fg(C_TEXT),
            ),
        ])
    } else {
        Line::from(Span::styled(
            " ○ Directory does not exist yet.",
            Style::default().fg(C_DIM),
        ))
    };
    frame.render_widget(Paragraph::new(validation), chunks[2]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [↵] Next Step ",
                Style::default()
                    .fg(C_PURPLE)
                    .bg(C_SLATE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(" [Esc] Cancel ", Style::default().fg(C_MUTED).bg(C_SLATE)),
        ]))
        .alignment(Alignment::Right),
        chunks[3],
    );
}

fn render_import_name_modal(frame: &mut Frame, app: &App) {
    let area = centered_rect_exact(70, 9, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD))
        .title(Line::from(vec![
            Span::styled(" 󰋚 ", Style::default().fg(C_PURPLE)),
            Span::styled(
                "Import Dotfiles · Step 2/2 ",
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Choose a profile name for the imported configuration:",
            Style::default().fg(C_MUTED),
        ))),
        chunks[0],
    );

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_PURPLE));
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ❯ ", Style::default().fg(C_PURPLE)),
            Span::styled(
                app.input_buffer.clone(),
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(C_CYAN)),
        ])),
        input_inner,
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Source: ", Style::default().fg(C_DIM)),
            Span::styled(
                compact_path(&app.pending_import_path, &app.home_dir),
                Style::default().fg(C_MUTED),
            ),
        ])),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [↵] Finish Import ",
                Style::default()
                    .fg(C_PURPLE)
                    .bg(C_SLATE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(" [Esc] Cancel ", Style::default().fg(C_MUTED).bg(C_SLATE)),
        ]))
        .alignment(Alignment::Right),
        chunks[3],
    );
}

fn render_delete_modal(frame: &mut Frame, app: &App) {
    let area = centered_rect_exact(60, 9, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_ERR).add_modifier(Modifier::BOLD))
        .title(Line::from(vec![
            Span::styled(" 󰆴 ", Style::default().fg(C_ERR)),
            Span::styled(
                "Confirm Delete ",
                Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let name = app
        .selected_profile()
        .cloned()
        .unwrap_or_else(|| "profile".to_string());

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Are you sure you want to permanently delete profile:",
                Style::default().fg(C_TEXT),
            )),
            Line::from(vec![
                Span::raw("  ● "),
                Span::styled(
                    name,
                    Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
                ),
            ]),
        ]),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "This will remove ~/.hyprdeck/profiles and cannot be undone.",
            Style::default().fg(C_MUTED),
        ))),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [y] Confirm Delete ",
                Style::default()
                    .fg(Color::Rgb(0x18, 0x18, 0x24))
                    .bg(C_ERR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                " [n / Esc] Cancel ",
                Style::default().fg(C_TEXT_BRIGHT).bg(C_SLATE),
            ),
        ]))
        .alignment(Alignment::Right),
        chunks[2],
    );
}

fn render_help_modal(frame: &mut Frame, _app: &App) {
    let area = centered_rect_exact(76, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD))
        .title(Line::from(vec![
            Span::styled(" 󰋖 ", Style::default().fg(C_BLUE)),
            Span::styled(
                "Hyprdeck Cheatsheet & Keyboard Shortcuts ",
                Style::default()
                    .fg(C_TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

    let help_cols =
        Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(rows[0]);

    let nav_and_actions = vec![
        Line::from(Span::styled(
            "NAVIGATION",
            Style::default().fg(C_BLUE).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  ↑ / k           ", Style::default().fg(C_CYAN)),
            Span::styled("Previous profile / scroll up", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  ↓ / j           ", Style::default().fg(C_CYAN)),
            Span::styled("Next profile / scroll down", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  Tab / ← / →     ", Style::default().fg(C_CYAN)),
            Span::styled(
                "Switch pane (Profiles / Details)",
                Style::default().fg(C_TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled("  PgUp / PgDn     ", Style::default().fg(C_CYAN)),
            Span::styled("Page scroll details view", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  g / G           ", Style::default().fg(C_CYAN)),
            Span::styled("Jump to top / bottom", Style::default().fg(C_TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "PROFILE ACTIONS",
            Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  Enter           ", Style::default().fg(C_PURPLE)),
            Span::styled("Activate & hot-reload daemons", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  s               ", Style::default().fg(C_PURPLE)),
            Span::styled("Snapshot current environment", Style::default().fg(C_TEXT)),
        ]),
    ];

    let actions_and_tools = vec![
        Line::from(Span::styled(
            "MANAGEMENT",
            Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  i               ", Style::default().fg(C_PURPLE)),
            Span::styled(
                "Import external dotfiles directory",
                Style::default().fg(C_TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled("  d               ", Style::default().fg(C_PURPLE)),
            Span::styled("Delete selected profile", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  r               ", Style::default().fg(C_PURPLE)),
            Span::styled(
                "Force-reload ecosystem daemons",
                Style::default().fg(C_TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled("  o               ", Style::default().fg(C_PURPLE)),
            Span::styled("Open folder in file manager", Style::default().fg(C_TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "SEARCH & APPLICATION",
            Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  /               ", Style::default().fg(C_OK)),
            Span::styled(
                "Search & filter profiles in real-time",
                Style::default().fg(C_TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ?               ", Style::default().fg(C_OK)),
            Span::styled("Toggle this help popup", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc         ", Style::default().fg(C_OK)),
            Span::styled("Close modal / Quit Hyprdeck", Style::default().fg(C_TEXT)),
        ]),
    ];

    frame.render_widget(Paragraph::new(nav_and_actions), help_cols[0]);
    frame.render_widget(Paragraph::new(actions_and_tools), help_cols[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " Press [Esc] or [?] to close ",
            Style::default()
                .fg(C_TEXT_BRIGHT)
                .bg(C_SLATE)
                .add_modifier(Modifier::BOLD),
        )]))
        .alignment(Alignment::Center),
        rows[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_registry_contains_universal_shells_and_apps() {
        let targets = App::build_ecosystem_registry();
        let names: BTreeSet<String> = targets.iter().map(|t| t.store_name.clone()).collect();

        // Check desktop shells / bars
        assert!(names.contains("noctalia"));
        assert!(names.contains("waybar"));
        assert!(names.contains("ags"));
        assert!(names.contains("quickshell"));
        assert!(names.contains("caelestia"));
        assert!(names.contains("ironbar"));
        assert!(names.contains("hyprpanel"));
        assert!(names.contains("eww"));
        assert!(names.contains("sfwbar"));
        assert!(names.contains("yambar"));

        // Check Hyprland ecosystem
        assert!(names.contains("hypr"));
        assert!(names.contains("hyprlock"));
        assert!(names.contains("hypridle"));
        assert!(names.contains("hyprpaper"));
        assert!(names.contains("hyprsunset"));
        assert!(names.contains("swayosd"));

        // Check terminals
        assert!(names.contains("kitty"));
        assert!(names.contains("alacritty"));
        assert!(names.contains("foot"));
        assert!(names.contains("wezterm"));
        assert!(names.contains("ghostty"));

        // Check user shells & dotfiles
        assert!(names.contains(".zshrc"));
        assert!(names.contains(".bashrc"));
        assert!(names.contains("fish"));
        assert!(names.contains("nushell"));
        assert!(names.contains("starship.toml"));

        // Check editors & utilities
        assert!(names.contains("nvim"));
        assert!(names.contains("helix"));
        assert!(names.contains("yazi"));
        assert!(names.contains("swaync"));
        assert!(names.contains("dunst"));
        assert!(names.contains("swww"));
    }

    #[test]
    fn test_resolve_target_known_and_dynamic() {
        let app = App {
            profiles: Vec::new(),
            state: ListState::default(),
            active_profile: String::new(),
            status_msg: String::new(),
            last_reload_summary: String::new(),
            base_dir: PathBuf::new(),
            home_dir: PathBuf::new(),
            known_targets: App::build_ecosystem_registry(),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            pending_import_path: PathBuf::new(),
            detail_scroll: 0,
            detail_lines_count: 0,
            active_pane: ActivePane::Profiles,
            filter_query: String::new(),
            filtered_indices: Vec::new(),
        };

        // Known targets
        let waybar = app.resolve_target("waybar");
        assert_eq!(waybar.home_rel_path, ".config/waybar");
        assert_eq!(waybar.category, "Shell / Bar");

        let zshrc = app.resolve_target(".zshrc");
        assert_eq!(zshrc.home_rel_path, ".zshrc");
        assert_eq!(zshrc.category, "Shell / Prompt");

        let nvim = app.resolve_target("nvim");
        assert_eq!(nvim.home_rel_path, ".config/nvim");
        assert_eq!(nvim.category, "Editor / IDE");

        // Dynamic unknown targets
        let custom_bar = app.resolve_target("my-custom-bar");
        assert_eq!(custom_bar.home_rel_path, ".config/my-custom-bar");
        assert_eq!(custom_bar.category, "Shell / Bar");

        let custom_dot = app.resolve_target(".myprofile");
        assert_eq!(custom_dot.home_rel_path, ".myprofile");
        assert_eq!(custom_dot.category, "Shell / Prompt");

        let custom_app = app.resolve_target("some-new-app");
        assert_eq!(custom_app.home_rel_path, ".config/some-new-app");
        assert_eq!(custom_app.category, "App Config");
    }

    #[test]
    fn test_dir_mentions_recursive() {
        let temp_dir = std::env::temp_dir().join("hyprdeck_test_mentions");
        let sub_dir = temp_dir.join("subfolder").join("config");
        let _ = fs::create_dir_all(&sub_dir);

        let startup_file = sub_dir.join("startup.lua");
        let mut file = File::create(&startup_file).unwrap();
        writeln!(file, "hl.exec_cmd('waybar')").unwrap();

        assert!(App::dir_mentions_recursive(&temp_dir, "waybar"));
        assert!(App::dir_mentions_recursive(&temp_dir, "WayBar"));
        assert!(!App::dir_mentions_recursive(&temp_dir, "noctalia"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_filter_profiles() {
        let mut app = App {
            profiles: vec![
                "catppuccin-mocha".to_string(),
                "default".to_string(),
                "nordic-dark".to_string(),
                "tokyo-night".to_string(),
            ],
            state: ListState::default(),
            active_profile: String::new(),
            status_msg: String::new(),
            last_reload_summary: String::new(),
            base_dir: PathBuf::new(),
            home_dir: PathBuf::new(),
            known_targets: Vec::new(),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            pending_import_path: PathBuf::new(),
            detail_scroll: 0,
            detail_lines_count: 0,
            active_pane: ActivePane::Profiles,
            filter_query: String::new(),
            filtered_indices: Vec::new(),
        };

        app.update_filter();
        assert_eq!(app.filtered_indices.len(), 4);

        app.filter_query = "NIGHT".to_string();
        app.update_filter();
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.selected_profile(), Some(&"tokyo-night".to_string()));

        app.filter_query.clear();
        app.update_filter();
        assert_eq!(app.filtered_indices.len(), 4);
    }

    #[test]
    fn test_status_badge_variants() {
        let (err_badge, _) = status_badge("Database connection failed");
        assert_eq!(err_badge.content, " ✖ ERROR ");

        let (ok_badge, _) = status_badge("Profile switched successfully");
        assert_eq!(ok_badge.content, " ● SUCCESS ");

        let (cancel_badge, _) = status_badge("Operation cancelled by user");
        assert_eq!(cancel_badge.content, " ○ CANCEL ");

        let (ready_badge, _) = status_badge("Ready — pick a profile");
        assert_eq!(ready_badge.content, " ℹ READY ");
    }

    #[test]
    fn test_compact_path_formatting() {
        let home = PathBuf::from("/home/user");
        let sub = PathBuf::from("/home/user/.config/hypr");
        let external = PathBuf::from("/etc/systemd/system");

        assert_eq!(compact_path(&sub, &home), "~/.config/hypr");
        assert_eq!(compact_path(&external, &home), "/etc/systemd/system");
    }

    #[test]
    fn test_ui_render_all_modes_and_sizes() {
        use ratatui::backend::TestBackend;

        let mut app = App {
            profiles: vec!["default".to_string(), "minimal".to_string()],
            state: ListState::default(),
            active_profile: "default".to_string(),
            status_msg: "Ready".to_string(),
            last_reload_summary: "hyprctl reload".to_string(),
            base_dir: PathBuf::from("/tmp"),
            home_dir: PathBuf::from("/home/user"),
            known_targets: App::build_ecosystem_registry(),
            input_mode: InputMode::Normal,
            input_buffer: "my-snapshot".to_string(),
            pending_import_path: PathBuf::from("/tmp/import"),
            detail_scroll: 0,
            detail_lines_count: 0,
            active_pane: ActivePane::Profiles,
            filter_query: String::new(),
            filtered_indices: vec![0, 1],
        };
        app.state.select(Some(0));

        // Test normal mode in standard size
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| ui(f, &mut app)).unwrap();

        // Test filter mode
        app.input_mode = InputMode::Filter;
        terminal.draw(|f| ui(f, &mut app)).unwrap();

        // Test help modal
        app.input_mode = InputMode::Help;
        terminal.draw(|f| ui(f, &mut app)).unwrap();

        // Test snapshot modal
        app.input_mode = InputMode::SnapshotName;
        terminal.draw(|f| ui(f, &mut app)).unwrap();

        // Test import path modal
        app.input_mode = InputMode::ImportPath;
        terminal.draw(|f| ui(f, &mut app)).unwrap();

        // Test import name modal
        app.input_mode = InputMode::ImportName;
        terminal.draw(|f| ui(f, &mut app)).unwrap();

        // Test delete modal
        app.input_mode = InputMode::ConfirmDelete;
        terminal.draw(|f| ui(f, &mut app)).unwrap();

        // Test small cramped terminal
        let backend_small = TestBackend::new(45, 12);
        let mut terminal_small = Terminal::new(backend_small).unwrap();
        app.input_mode = InputMode::Normal;
        terminal_small.draw(|f| ui(f, &mut app)).unwrap();
    }

    fn temp_app(tag: &str) -> App {
        let root = std::env::temp_dir().join(format!("hyprdeck_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        fs::create_dir_all(home.join(".config")).unwrap();
        let base = home.join(".hyprdeck");
        fs::create_dir_all(base.join("profiles")).unwrap();
        App {
            profiles: Vec::new(),
            state: ListState::default(),
            active_profile: String::new(),
            status_msg: String::new(),
            last_reload_summary: String::new(),
            base_dir: base,
            home_dir: home,
            known_targets: App::build_ecosystem_registry(),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            pending_import_path: PathBuf::new(),
            detail_scroll: 0,
            detail_lines_count: 0,
            active_pane: ActivePane::Profiles,
            filter_query: String::new(),
            filtered_indices: Vec::new(),
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn test_line_has_word_is_boundary_aware() {
        assert!(App::line_has_word("exec-once = qs -c x", "qs"));
        assert!(!App::line_has_word("exec = tags --flags", "qs"));
        assert!(!App::line_has_word("wallust run", "wal"));
        assert!(App::line_has_word("exec-once = wal -r", "wal"));
        assert!(App::line_has_word("spawn noctalia-shell", "noctalia"));
    }

    #[test]
    fn test_comments_do_not_count_as_mentions() {
        let app = temp_app("comments");
        let dir = app.base_dir.join("hypr");
        write(
            &dir.join("a.conf"),
            "# exec-once = waybar\n-- hl.exec_cmd('eww')\n",
        );
        assert!(!App::dir_mentions_recursive(&dir, "waybar"));
        assert!(!App::dir_mentions_recursive(&dir, "eww"));
        let _ = fs::remove_dir_all(app.home_dir.parent().unwrap());
    }

    #[test]
    fn test_quickshell_config_name_resolution() {
        let app = temp_app("qsname");
        let prof = app.base_dir.join("profiles/p");

        // Named config referenced from the Hyprland config.
        write(
            &prof.join("hypr/hyprland.conf"),
            "exec-once = qs -c mybar\n",
        );
        assert_eq!(App::quickshell_config_name(&prof), Some("mybar".into()));

        // No -c: a single named config dir is used.
        write(&prof.join("hypr/hyprland.conf"), "exec-once = qs\n");
        write(&prof.join("quickshell/ii/shell.qml"), "");
        assert_eq!(App::quickshell_config_name(&prof), Some("ii".into()));

        // Default config at the module root: plain `qs`.
        write(&prof.join("quickshell/shell.qml"), "");
        assert_eq!(App::quickshell_config_name(&prof), None);
        let _ = fs::remove_dir_all(app.home_dir.parent().unwrap());
    }

    #[test]
    fn test_apply_profile_switches_links_and_keeps_every_backup() {
        let mut app = temp_app("apply");
        let profiles = app.base_dir.join("profiles");
        write(&profiles.join("a/waybar/config"), "a");
        write(&profiles.join("b/waybar/config"), "b");

        // Pre-existing unmanaged config.
        write(&app.home_dir.join(".config/waybar/config"), "mine");

        app.apply_profile("a").unwrap();
        let live = app.home_dir.join(".config/waybar");
        assert_eq!(fs::read_link(&live).unwrap(), profiles.join("a/waybar"));

        app.apply_profile("b").unwrap();
        assert_eq!(fs::read_link(&live).unwrap(), profiles.join("b/waybar"));

        // The original config must still be on disk after switching twice.
        let backups: Vec<_> = fs::read_dir(app.base_dir.join("backups"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(backups[0].path().join("config")).unwrap(),
            "mine"
        );

        // An unmanaged config appearing again is backed up without clobbering the first.
        fs::remove_file(&live).unwrap();
        write(&live.join("config"), "mine2");
        app.apply_profile("a").unwrap();
        assert_eq!(
            fs::read_dir(app.base_dir.join("backups")).unwrap().count(),
            2
        );
        let _ = fs::remove_dir_all(app.home_dir.parent().unwrap());
    }

    #[test]
    fn test_apply_profile_rolls_back_on_failure() {
        use std::os::unix::fs::PermissionsExt;

        let mut app = temp_app("rollback");
        let profiles = app.base_dir.join("profiles");
        write(&profiles.join("a/waybar/config"), "a");
        write(&profiles.join("b/alacritty/alacritty.toml"), "b");
        write(&profiles.join("b/waybar/config"), "b");
        app.apply_profile("a").unwrap();

        // Unmanaged waybar config that must be backed up, but backups/ is read-only.
        let live_waybar = app.home_dir.join(".config/waybar");
        fs::remove_file(&live_waybar).unwrap();
        write(&live_waybar.join("config"), "mine");
        let backups = app.base_dir.join("backups");
        fs::set_permissions(&backups, fs::Permissions::from_mode(0o555)).unwrap();

        let res = app.apply_profile("b");
        fs::set_permissions(&backups, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(res.is_err());
        // alacritty was linked before the failure and must be undone; waybar untouched.
        assert!(fs::symlink_metadata(app.home_dir.join(".config/alacritty")).is_err());
        assert_eq!(
            fs::read_to_string(live_waybar.join("config")).unwrap(),
            "mine"
        );
        let _ = fs::remove_dir_all(app.home_dir.parent().unwrap());
    }
}
