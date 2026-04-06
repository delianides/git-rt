# git-rt

A real-time terminal dashboard for git changes. Watch your working tree update live as you edit files, with inline diffs and configurable actions.

![status: early development](https://img.shields.io/badge/status-early%20development-orange)

## What it does

Run `git-rt` in a terminal pane alongside your editor. It shows a live-updating view of all changed files with insertion/deletion counts, and lets you expand any file to see its diff inline.

```
  M src/main.rs          -3   +12
  M src/watcher.rs       -0   +45
▼ M src/config.rs        -10  +2
│  @@ -14,10 +14,2 @@
│  -  let old_config = parse(raw);
│  +  let config = Config::from_toml(raw);
  ? tests/integration.rs -1   +1

 4 files changed  -14  +60  │  j/k:nav  enter:expand  q:quit
```

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

| Key | Action |
|---|---|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `Enter` / `l` | Expand file diff |
| `h` | Collapse file diff |
| `r` | Refresh |
| `q` | Quit |

## Configuration

Create `~/.config/git-rt/config.toml` to customize behavior:

```toml
debounce_ms = 200

[display]
show_status = true
context_lines = 3

[actions.open_editor]
key = "e"
tmux = "tmux split-window -h 'nvim {file}'"
zellij = "zellij run --direction right -- nvim {file}"
fallback = "nvim {file}"

[actions.diff_view]
key = "d"
tmux = "tmux popup -w 80% -h 80% 'git diff -- {file} | delta'"
fallback = "git diff -- {file} | delta"
```

## License

MIT
