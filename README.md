# git-rt

A real-time terminal dashboard for git changes. Watch your working tree update live as you edit files, with inline diffs and configurable actions.

![status: early development](https://img.shields.io/badge/status-early%20development-orange)

## What it does

Run `git-rt` in a terminal pane alongside your editor. It shows a live-updating view of all changed files with insertion/deletion counts, and lets you expand any file to see its diff inline.

![git-rt screenshot](assets/screenshot.png)

## Install

```bash
cargo install --path .
```

## Usage

```bash
# Run in the current git repository
git-rt

# Run for a specific repo
git-rt /path/to/repo

# Custom debounce interval
git-rt --debounce 500
```

### Default keybindings

| Key           | Action             |
| ------------- | ------------------ |
| `j` / `↓`     | Move down          |
| `k` / `↑`     | Move up            |
| `Enter` / `l` | Expand file diff   |
| `h`           | Collapse file diff |
| `r`           | Refresh            |
| `q`           | Quit               |

## Configuration

Create `~/.config/git-rt/config.toml` to customize behavior.

### Color Palette

Define custom named colors in the top-level `[colors]` section. These can be referenced in statusbar format strings using `{name}...{/}` tags. The 16 built-in terminal color names (red, green, blue, yellow, cyan, magenta, white, black, gray, darkgray, lightred, lightgreen, lightyellow, lightblue, lightmagenta, lightcyan) are always available without defining them. Palette entries can override built-in names.

```toml
[colors]
ins = "#50FA7B"
del = "#FF5555"
branch = "#BD93F9"
muted = "#6272A4"
```

### Statusbar

Top and bottom statusbars are independently configurable with format strings. The top bar is hidden by default. Pass an empty string to hide either bar.

Format tokens: `%b` (branch), `%c` (file count), `%+` (total insertions), `%-` (total deletions), `%R` (refresh counter), `%h` (HEAD short SHA), `%H` (HEAD message), `%w` (worktree name), `%n` (repo name), `%a` (ahead/behind), `%m` (modified count), `%u` (untracked count), `%s` (staged count), `%S` (stash count), `%G` (git state), `%?` (help), `%=` (right-align marker).

Style tags: `{color}...{/}`, `{bold}...{/}`, `{dim}...{/}`. Colors can be palette names, built-in names, or hex (`{#FF5555}...{/}`).

```toml
[display.statusbar.top]
status_line = "{dim}%n{/}  {muted}%h{/}"
foreground_color = "white"
background_color = "#1E1E1E"

[display.statusbar.bottom]
status_line = "{branch}%b{/}  %c files  {del}%-{/} {ins}%+{/}  %=%R"
foreground_color = "white"
background_color = "#1E1E1E"
```

### File Line Format

Customize how each file row is displayed with a format string.

Tokens: `%s` (status char), `%S` (staged char), `%f` (path), `%n` (filename), `%d` (directory), `%e` (extension), `%-` (deletions), `%+` (insertions), `%t` (total changes), `%g` (change graph), `%b` (branch), `%=` (right-align).

```toml
[display]
file_line = "%s %f %= %- %+"
show_expand_marker = true
```

### UI Colors

```toml
[display.colors.ui]
selection_bg = "darkgray"
selection_fg = "white"
flash_bg = "#64641E"
empty_text = "darkgray"
```

### Actions

```toml
[actions.open_editor]
key = "e"
command = "nvim {file}"

[actions.diff_view]
key = "d"
command = "git diff -- {file} | delta"
```

### Sample Config

```toml
debounce_ms = 200

[colors]
ins = "#50FA7B"
del = "#FF5555"
branch = "#BD93F9"
muted = "#6272A4"

[display]
context_lines = 3
flash_on_change = true
flash_duration_ms = 600
file_line = "%s %f %= %- %+"
show_expand_marker = true

[display.statusbar.top]
status_line = "{dim}%n{/}"
foreground_color = "white"
background_color = "#282A36"

[display.statusbar.bottom]
status_line = "{branch}%b{/}  %c files  {del}%-{/} {ins}%+{/}  %=%R"
foreground_color = "white"
background_color = "#282A36"

[display.colors.ui]
selection_bg = "#44475A"
selection_fg = "#F8F8F2"
flash_bg = "#64641E"
empty_text = "#6272A4"

[display.padding]
top = 1
bottom = 0
left = 0
right = 2

[actions.edit]
key = "e"
command = "nvim {file}"
```

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
