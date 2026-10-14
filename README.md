# Hyprdeck

A blazing-fast, keyboard-driven Terminal User Interface (TUI) written in Rust to manage, snapshot, import, and hot-reload Hyprland dotfile profiles and ecosystem daemons on the fly.

[![Release](https://img.shields.io/github/v/release/shahriar-ridom/hyprdeck?style=for-the-badge&color=00f2fe)](https://github.com/shahriar-ridom/hyprdeck/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75+-orange.svg?style=for-the-badge&logo=rust)](https://www.rust-lang.org)

---

## Downloads

Download pre-built packages directly for your distribution:

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

* **Zero-Downtime Hot-Reloading:** Switches bars, shells, notification daemons, and terminals cleanly without stacking duplicate surfaces or crashing Wayland layers.
* **Native Ecosystem Awareness:** Auto-detects and orchestrates:
  * **Shells & Bars:** Noctalia (v5+ IPC & legacy), Caelestia, Quickshell, Waybar, AGS, Hyprpanel, Ironbar, EWW.
  * **Terminals:** Kitty (live `SIGUSR1` palette updates), Alacritty, Foot, WezTerm, Ghostty.
  * **Theming & Utilities:** Pywal (`wal -R`), SwayNC, Dunst, Mako, Starship, Zsh.
* **Non-Destructive Symlinking:** Automatically backs up existing unmanaged configurations to `~/.hyprdots/backups/` before linking new profiles.
* **Full Profile Lifecycle:** Snapshot your live configuration into a reusable profile in one keystroke, or import community dotfile repositories effortlessly.
* **Atomic Hooks:** Drop an optional `reload.sh` script into any profile to execute custom scripts, shaders, or color alterations during profile transitions.

---

## Installation

### Arch Linux

Download and install the native package using `pacman`:

```bash
# Replace <version> with the current release tag (e.g., 0.1.0)
curl -LO [https://github.com/shahriar-ridom/hyprdeck/releases/download/v0.1.0/hyprdeck-bin-0.1.0-1-x86_64.pkg.tar.zst](https://github.com/shahriar-ridom/hyprdeck/releases/download/v0.1.0/hyprdeck-bin-0.1.0-1-x86_64.pkg.tar.zst)
sudo pacman -U hyprdeck-bin-0.1.0-1-x86_64.pkg.tar.zst
```

### Debian / Ubuntu

Download and install using `apt`:

```bash
curl -LO [https://github.com/shahriar-ridom/hyprdeck/releases/download/v0.1.0/hyprdeck_0.1.0-1_amd64.deb](https://github.com/shahriar-ridom/hyprdeck/releases/download/v0.1.0/hyprdeck_0.1.0-1_amd64.deb)
sudo apt install ./hyprdeck_0.1.0-1_amd64.deb
```

### AppImage (Universal)

No package manager required:

```bash
curl -LO [https://github.com/shahriar-ridom/hyprdeck/releases/download/v0.1.0/hyprdeck-v0.1.0-x86_64.AppImage](https://github.com/shahriar-ridom/hyprdeck/releases/download/v0.1.0/hyprdeck-v0.1.0-x86_64.AppImage)
chmod +x hyprdeck-v0.1.0-x86_64.AppImage
./hyprdeck-v0.1.0-x86_64.AppImage
```

### Build From Source (Cargo)

```bash
git clone [https://github.com/shahriar-ridom/hyprdeck.git](https://github.com/shahriar-ridom/hyprdeck.git)
cd hyprdeck
cargo build --release
sudo install -m 755 target/release/hyprdeck /usr/local/bin/
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
| `j` / `↓` | Normal | Navigate down the profile list |
| `k` / `↑` | Normal | Navigate up the profile list |
| `Enter` | Normal | Apply selected profile, swap symlinks, and reload daemons |
| `s` | Normal | **Snapshot:** Save live setup into a new profile |
| `i` | Normal | **Import:** Load an external dotfiles folder into a managed profile |
| `d` | Normal | **Delete:** Remove selected profile and unlink active symlinks |
| `r` | Normal | **Force Reload:** Re-trigger daemon hand-off for selected profile |
| `q` / `Esc` | Normal | Exit Hyprdeck |
| `Enter` | Input | Confirm input (Snapshot name, Import path, Delete) |
| `Esc` | Input | Cancel modal prompt |

---

## Profile Directory Structure

Hyprdeck stores all profiles inside `~/.hyprdots/profiles/`. 

```text
~/.hyprdots/
├── backups/                    # Auto-generated backups of replaced configs
└── profiles/
    ├── minimal-dark/
    │   ├── hypr/               # Links to ~/.config/hypr
    │   ├── waybar/             # Links to ~/.config/waybar
    │   ├── kitty/              # Links to ~/.config/kitty
    │   └── reload.sh           # (Optional) Profile-specific startup script
    └── modern-noctalia/
        ├── hypr/
        ├── noctalia/           # Links to ~/.config/noctalia
        └── starship.toml       # Links to ~/.config/starship.toml
```

### Custom Profile Hooks (`reload.sh`)

If a profile folder contains an executable script named `reload.sh`, Hyprdeck will run it asynchronously after symlinking targets and restarting core bars:

```bash
#!/usr/bin/env bash
# ~/.hyprdots/profiles/modern-noctalia/reload.sh

# Set wallpaper via swww
swww img ~/Pictures/Wallpapers/cyberpunk.png --transition-type wipe

# Generate colors with pywal or matugen
wal -i ~/Pictures/Wallpapers/cyberpunk.png -q
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
