# git-rt

A real-time terminal dashboard for git changes. Watch your working tree update live as you edit files, with PR status and a one-keystroke in-app diff overlay.

![status: early development](https://img.shields.io/badge/status-early%20development-orange)

## Overview

Run `git-rt` in a terminal pane alongside your editor. It shows a live-updating list of changed files with insertion/deletion counts and — when the current branch has a PR open on GitHub — a compact PR status strip with review, check, and mergeability state. Press `m` to cycle between the flat file list and a tree view that groups changes by directory. Press `Enter` (or `d` / `Space` / `l` / `→`) on a file to open its full diff in a centered in-app overlay, or use those same keys to toggle directories in tree mode. Updates are event-driven via filesystem watches; there is no polling of the working tree.

## Install

```bash
cargo install --path .
```

Or see [Development](#development) for a Nix-based setup.

## Usage

```bash
# Current directory
git-rt

# Specific repo
git-rt /path/to/repo

# Custom debounce
git-rt --debounce 500
```

git-rt can be launched from any directory inside a git working tree — the repository root is discovered automatically.

### CLI flags

| Flag                      | Purpose                                                                 |
| ------------------------- | ----------------------------------------------------------------------- |
| `[PATH]`                  | Repository path (default `.`)                                           |
| `-c, --config <FILE>`     | Path to config file                                                     |
| `-d, --debounce <MS>`     | Filesystem debounce in ms (default `200`)                               |
| `--log <LEVEL>`           | Logging level: `trace`, `debug`, `info`, `warn`, `error`                |
| `--branch <BRANCH>`       | Pin to the worktree (main or linked) with this branch checked out; auto-follow remains active |
| `--no-follow`             | Disable auto-follow to other worktrees                                  |
| `--theme <NAME\|PATH>`    | Theme override (built-in name or path to a `.toml` theme file)          |
| `--base <BRANCH>`         | Base branch for the branch-scoped diff range                            |

## Keybindings

| Key                   | Action                                                |
| --------------------- | ----------------------------------------------------- |
| `j` / `↓`             | Select next file                                      |
| `k` / `↑`             | Select previous file                                  |
| `m`                   | Cycle view mode (`flat` / `tree`)                     |
| `Enter` / `l` / `→` / `Space` / `d` | Open diff for files, toggle directories in tree mode |
| `r`                   | Refresh                                               |
| `?`                   | Show the help popup                                   |
| `q` / `Ctrl+C`        | Quit                                                  |

Inside the diff overlay:

| Key                                     | Action          |
| --------------------------------------- | --------------- |
| `j` / `↓`                               | Scroll down     |
| `k` / `↑`                               | Scroll up       |
| `Esc` / `q` / `h` / `←`                 | Close overlay   |
| `d` / `Space`                           | Toggle overlay  |

Pressing a diff key opens a centered panel (~85% of the terminal) that renders the file's diff inline — colored `+`/`-`/context lines with line numbers, scrollable with `j` / `k`. The overlay lives inside the TUI (no external tool is invoked) and dismisses with `Esc` / `q` / `h` / `←` (or toggles with `d` / `Space`).

## Configuration

Config lives at `~/.config/git-rt/config.toml`. All sections are optional; defaults are used for anything you omit.

### Top-level

```toml
theme = "catppuccin-mocha"   # built-in theme name or user theme file name
debounce_ms = 200            # filesystem event debounce in ms
base_branch = "main"         # base for branch-scoped diff (merge-base..worktree)
```

### `[display]`

```toml
[display]
context_lines = 3            # diff context lines
flash_on_change = true       # flash a file row when it changes
flash_duration_ms = 600      # flash duration
```

### `[keys]`

Rebindable single-character keys.

```toml
[keys]
quit = "q"
up = "k"
down = "j"
expand = "l"
collapse = "h"
refresh = "r"
```

### `[pr]`

Controls the compact PR status strip that appears when the current branch has an open PR on GitHub.

```toml
[pr]
enabled = true
show_labels = false
```

The GitHub token is discovered from the `GITHUB_TOKEN` environment variable or your `git config`. If no token is available the PR strip silently stays hidden.

### Full example

```toml
theme = "catppuccin-mocha"
debounce_ms = 200
base_branch = "main"

[display]
context_lines = 3
flash_on_change = true
flash_duration_ms = 600

[pr]
enabled = true
show_labels = false
```

## Themes

git-rt ships with 11 built-in themes:

- `catppuccin-mocha` (default)
- `catppuccin-latte`
- `one-dark`
- `dracula`
- `gruvbox-dark`
- `nord`
- `tokyo-night`
- `solarized-dark`
- `rose-pine`
- `kanagawa`
- `everforest-dark`

Select one via the `theme` config key or the `--theme` CLI flag.

User themes live in `~/.config/git-rt/themes/` — drop a `<name>.toml` file there and reference it as `theme = "<name>"`. A theme can inherit from another via `extends = "<other-theme>"` and override only the colors it wants to change.

Want a theme that isn't in the list? [PRs welcome](https://github.com/delianides/git-rt/pulls) — add a `src/theme/builtin/<name>.toml` and register it in `src/theme/mod.rs`.

## Development

### Prerequisites

- [Rust](https://rustup.rs/) (pinned via `rust-toolchain.toml`)
- Or [Nix](https://nixos.org/) + [direnv](https://direnv.net/) for a reproducible dev environment

### With Nix (recommended)

```bash
direnv allow  # one-time setup, auto-activates on cd
cargo run     # run the app
cargo test    # run tests
```

### Without Nix

```bash
rustup show           # installs toolchain from rust-toolchain.toml
cargo run             # run the app
cargo test            # run tests
cargo clippy          # lint
cargo fmt             # format
```

### Nix commands

```bash
nix flake check       # build + clippy + fmt
nix build             # build the package (output in ./result)
nix run               # build and run
nix flake update      # update pinned dependencies
```

## License

MIT
