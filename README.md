# hyprswitch-tui

`hyprswitch-tui` is a terminal UI for saving, importing, and switching Hyprland desktop configurations. It stores each setup as a named profile, activates profile entries with symlinks, and coordinates common bars, shells, notification daemons, terminal reloads, and optional profile hooks.

## Requirements

- Linux with a Unix-like filesystem
- Rust and Cargo (Rust 2021 edition)
- A Hyprland session for compositor reloads and daemon integration
- `hyprctl` available in `PATH`

The profile manager itself can run without every supported desktop component installed. Missing commands are skipped and reported in the UI. Supported integrations include Hyprland, Noctalia, Caelestia, QuickShell, Waybar, AGS, EWW, Ironbar, HyprPanel, SwayNC, Dunst, Mako, Kitty, and pywal, along with several configuration-only tools.

## Install and run

Clone the repository and start the application with Cargo:

```sh
git clone <repository-url>
cd hyprswitch-tui
cargo run --release
```

To install the compiled binary somewhere on your `PATH`:

```sh
cargo install --path .
hyprswitch-tui
```

Run it from inside your graphical Hyprland session so the environment needed by Wayland applications is available.

## Key bindings

| Key | Action |
| --- | --- |
| `j` / `Down` | Select the next profile |
| `k` / `Up` | Select the previous profile |
| `Enter` | Activate the selected profile |
| `s` | Snapshot the current live configuration |
| `i` | Import a directory as a new profile |
| `d` | Delete the selected profile after confirmation |
| `r` | Force-reload daemons for the selected profile |
| `Esc` | Cancel the current prompt or quit from the main view |
| `q` | Quit |

## Profiles and storage

Profiles are stored under:

```text
~/.hyprdeck/profiles/<profile-name>/
```

Each entry in a profile is stored by module name. For example, the `hypr` entry maps to `~/.config/hypr`, while `.zshrc` maps to `~/.zshrc`. When a profile is activated, the application removes symlinks previously created for managed targets, backs up conflicting regular files or directories under `~/.hyprdeck/backups/`, and creates new symlinks into the selected profile.

The built-in registry recognizes common Hyprland ecosystem paths. Any additional directory found in a profile is treated as a custom `~/.config/<name>` entry. Standalone files such as `starship.toml` and `.zshrc` are recognized explicitly.

## Creating a profile

Press `s`, enter a profile name, and press `Enter`. The application copies recognized configuration entries from your home directory into the new profile and activates it. Existing files are copied into the profile; the live files are not changed until activation.

Profile names must not contain `/`, `\\`, or `..`.

## Importing a profile

Press `i` and enter a directory containing dotfiles. The importer accepts either the normal home-relative layout, such as:

```text
dotfiles/.config/hypr
dotfiles/.config/waybar
dotfiles/.zshrc
```

or a flat module layout, such as `dotfiles/hypr` and `dotfiles/waybar`. It imports recognized entries first, then copies additional non-hidden directories from `.config/` as custom modules. After naming the imported profile, press `Enter` again to activate it.

## Daemon hand-off

Activating a profile reloads Hyprland and starts or reloads services represented by that profile. The application avoids restarting a running Noctalia instance when its IPC reload command is available, stops competing bars and notification daemons before starting the selected one, sends Kitty its live-reload signal, and restores pywal colors when `wal` is present.

If a profile contains an executable `reload.sh`, it is launched after the built-in reload actions. The hook runs detached with its output redirected to `/dev/null`, so use your own logging in the script when troubleshooting.

## Safety notes

- Activating a profile changes paths in your home directory by replacing managed entries with symlinks.
- Existing regular files and directories at managed destinations are moved to `~/.hyprdeck/backups/` before replacement.
- Deleting a profile removes only symlinks that point into that profile and then removes the profile directory. It does not restore backups automatically.
- The daemon reload feature invokes commands such as `pkill`, `pgrep`, `killall`, `hyprctl`, and shell commands from profile hooks. Review profiles and hooks before activating them.

## Development

Format and check the project with:

```sh
cargo fmt --check
cargo check
```

The application currently lives in `src/main.rs`; the profile registry and daemon integration logic are kept there so supported modules can be audited in one place.
