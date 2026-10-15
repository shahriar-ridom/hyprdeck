# Hyprdeck

Hyprdeck is a keyboard-driven terminal UI (TUI) written in Rust for managing Hyprland dotfile profiles. It can snapshot the current configuration, import an existing dotfiles directory, switch profiles through symlinks, and reload supported desktop services.

[![Release](https://img.shields.io/github/v/release/shahriar-ridom/hyprdeck?style=for-the-badge&color=00f2fe)](https://github.com/shahriar-ridom/hyprdeck/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-orange.svg?style=for-the-badge&logo=rust)](https://www.rust-lang.org/)

---

## Downloads

Download pre-built packages from the [latest GitHub release](https://github.com/shahriar-ridom/hyprdeck/releases/latest):

<p align="center">
  <a href="https://github.com/shahriar-ridom/hyprdeck/releases/latest">
    <img src="https://img.shields.io/badge/Arch_Linux-.pkg.tar.zst-1793D1?style=for-the-badge&logo=arch-linux&logoColor=white" alt="Download Arch Package" />
  </a>
  &nbsp;
  <a href="https://github.com/shahriar-ridom/hyprdeck/releases/latest">
    <img src="https://img.shields.io/badge/Debian%20%2F%20Ubuntu-.deb-A81D33?style=for-the-badge&logo=debian&logoColor=white" alt="Download Debian Package" />
  </a>
  &nbsp;
  <a href="https://github.com/shahriar-ridom/hyprdeck/releases/latest">
    <img src="https://img.shields.io/badge/Universal-.AppImage-blueviolet?style=for-the-badge&logo=appimage&logoColor=white" alt="Download AppImage" />
  </a>
  &nbsp;
  <a href="https://github.com/shahriar-ridom/hyprdeck/releases/latest">
    <img src="https://img.shields.io/badge/Standalone_Binary-.tar.gz-333333?style=for-the-badge&logo=linux&logoColor=white" alt="Download Generic Tarball" />
  </a>
</p>

---

## Features

- **Profile switching:** Applies profiles through symlinks and backs up existing unmanaged configurations.
- **Ecosystem reloads:** Reloads Hyprland and starts or restarts supported services when they are present in the selected profile, including Noctalia, Caelestia, Quickshell, Waybar, AGS, Hyprpanel, Ironbar, EWW, SwayNC, Dunst, and Mako.
- **Terminal and theme support:** Handles Kitty, Alacritty, Foot, WezTerm, Ghostty, Pywal, Starship, Zsh, and more.
- **Profile lifecycle:** Snapshot the current configuration or import an existing dotfiles directory.
- **Custom hooks:** Add a `reload.sh` script to a profile to run custom commands after a profile transition.

Hyprdeck stores profiles in `~/.hyprdeck/profiles/` and backups in `~/.hyprdeck/backups/`.

---

## Installation

### Arch Linux

Download the Arch package from the [latest release](https://github.com/shahriar-ridom/hyprdeck/releases/latest), then install it with `pacman`:

```bash
sudo pacman -U ./hyprdeck-bin-<version>-<release>-x86_64.pkg.tar.zst
```

### Debian / Ubuntu

Download the Debian package from the [latest release](https://github.com/shahriar-ridom/hyprdeck/releases/latest), then install it with `apt`:

```bash
sudo apt install ./hyprdeck_<version>-<release>_amd64.deb
```

### AppImage (Universal)

Download the AppImage from the [latest release](https://github.com/shahriar-ridom/hyprdeck/releases/latest), make it executable, and run it:

```bash
chmod +x ./hyprdeck-<version>-x86_64.AppImage
./hyprdeck-<version>-x86_64.AppImage
```

### Build From Source (Cargo)

```bash
git clone https://github.com/shahriar-ridom/hyprdeck.git
cd hyprdeck
cargo build --release
sudo install -Dm755 target/release/hyprdeck /usr/local/bin/hyprdeck
```

---

## User Guide

Run `hyprdeck` from your terminal or launcher:

```bash
hyprdeck
```

### Keybindings

| Key | Mode | Action |
| :--- | :--- | :--- |
| `j` / `Down` | Normal | Navigate down the profile list |
| `k` / `Up` | Normal | Navigate up the profile list |
| `Enter` | Normal | Apply selected profile, swap symlinks, and reload daemons |
| `s` | Normal | **Snapshot:** Save live setup into a new profile |
| `i` | Normal | **Import:** Load an external dotfiles folder into a managed profile |
| `d` | Normal | **Delete:** Remove selected profile and unlink active symlinks |
| `r` | Normal | **Force Reload:** Re-trigger daemon hand-off for selected profile |
| `q` / `Esc` | Normal | Exit Hyprdeck |
| `Enter` | Input | Confirm a snapshot name, import path, or imported profile name |
| `y` / `n` | Delete | Confirm or cancel profile deletion |
| `Esc` | Input | Cancel the current prompt |

---

## Profile Directory Structure

Hyprdeck stores all profiles inside `~/.hyprdeck/profiles/`.

```text
~/.hyprdeck/
├── backups/                    # Auto-generated backups of replaced configs
└── profiles/
    ├── minimal-dark/
    │   ├── hypr/               # Linked to ~/.config/hypr when active
    │   ├── waybar/             # Linked to ~/.config/waybar when active
    │   ├── kitty/              # Linked to ~/.config/kitty when active
    │   └── reload.sh           # Optional profile hook
    └── modern-noctalia/
        ├── hypr/
        ├── noctalia/           # Linked to ~/.config/noctalia when active
        └── starship.toml       # Linked to ~/.config/starship.toml when active
```

### Custom Profile Hooks (`reload.sh`)

If a profile folder contains a file named `reload.sh`, Hyprdeck runs it asynchronously with `sh` after symlinking targets and reloading services:

```bash
#!/usr/bin/env bash
# ~/.hyprdeck/profiles/modern-noctalia/reload.sh

# Set wallpaper via swww
swww img ~/Pictures/Wallpapers/cyberpunk.png --transition-type wipe

# Generate colors with pywal or matugen
wal -i ~/Pictures/Wallpapers/cyberpunk.png -q
```

Make the hook executable:

```bash
chmod +x ~/.hyprdeck/profiles/modern-noctalia/reload.sh
```

---

## Troubleshooting

### My bar didn't launch after reloading
1. Check if the binary exists in your `$PATH` (e.g., `which waybar` or `which noctalia`).
2. If using Noctalia v5+, verify that the daemon didn't log an error by running:
   ```bash
   cat ~/.cache/noctalia/noctalia.log
   ```
3. Hit `[r]` inside `hyprdeck` to trigger a manual daemon restart cycle.

---

## License

Distributed under the MIT License. See [LICENSE](LICENSE) for more information.
