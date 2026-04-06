# CLAUDE.md — git-rt

## Project Overview

git-rt is a real-time terminal dashboard that watches git working tree changes and displays them as a live-updating TUI. Think of it as a persistent, interactive `git status` + `git diff --numstat` that runs in a terminal pane.

## Architecture

The project follows a layered architecture with clear separation of concerns:

```
src/
├── main.rs           # Entry point: CLI parsing, init, main event loop
├── app.rs            # Application state machine (handles events, coordinates layers)
├── watcher/          # Filesystem watching via notify crate
│   └── mod.rs        # Debounced FS events → channel messages
├── git/              # Git operations via gitoxide (gix)
│   └── mod.rs        # Diff computation, file status, numstat
├── state/            # Application state / view model
│   └── mod.rs        # FileEntry list, selection, expanded state, diff cache
├── ui/               # Rendering via ratatui
│   └── mod.rs        # Layout, file list, diff panel, status bar
├── actions/          # Configurable external actions (open editor, diff viewer, etc.)
│   └── mod.rs        # Template resolution, multiplexer detection, process spawning
└── config/           # Configuration loading and defaults
    └── mod.rs        # TOML parsing, XDG paths, default keybindings
```

## Core Concepts

### Default View (zero-config)
```
 src/main.rs          -3  +12
 src/watcher.rs       -0  +45
▼ src/config.rs       -10  +2
│  @@ -14,10 +14,2 @@
│  -  let old_config = parse(raw);
│  +  let config = Config::from_toml(raw);
 tests/integration.rs -1  +1
```

- Each line shows a changed file path with red deletion count and green addition count
- Navigation via `j/k` or arrow keys
- `Enter` or `l` expands inline diff for the selected file (accordion — only one open at a time)
- `h` or `Enter` on expanded file collapses it
- `q` quits

### Event Loop
The main loop multiplexes three event sources via crossbeam channels:
1. **Terminal events** (key presses, mouse, resize) from crossterm
2. **Filesystem events** from notify (debounced ~200ms)
3. **Tick events** for periodic UI refresh (~250ms)

When a filesystem event fires:
1. Watcher sends `Event::FsChange` to the app
2. App triggers git status recomputation
3. State updates the file list and invalidates stale diff caches
4. UI re-renders on the next tick

### Git Integration
Using `gix` (gitoxide) for all git operations — no shelling out to `git`.
- **File status**: Equivalent to `git status --porcelain` — untracked, modified, staged, deleted, renamed, conflicted
- **Diff numstat**: Insertions/deletions per file for the compact view
- **Diff hunks**: Full unified diff for the expanded view, computed lazily and cached by file content hash

### Diff Caching Strategy
- Diffs are computed lazily — only when a file is expanded
- Cache key is `(file_path, content_hash)` where content_hash is a fast hash of the working tree version
- Cache is invalidated when a new FS event touches that file path
- This keeps large repos responsive since we only compute diffs for visible content

### Filesystem Watching
- Uses `notify` with `notify-debouncer-full` for cross-platform support (inotify/FSEvents/kqueue)
- Watches the entire working tree, filters out `.git/` directory changes (except `.git/index` for staged changes)
- Debounce window: 200ms default, configurable
- On debounce fire: full git status recomputation (fast with gix)

### Action System (configurable, future phase)
Actions are shell command templates triggered by keybindings on a selected file. The tool detects the runtime environment and resolves the appropriate command variant.

Environment detection order: `$TMUX` → `$ZELLIJ` → `$WEZTERM_PANE` → fallback (plain terminal)

Config file location: `~/.config/git-rt/config.toml`

```toml
[actions.open_editor]
key = "e"
tmux = "tmux split-window -h 'nvim {file}'"
zellij = "zellij run --direction right -- nvim {file}"
fallback = "nvim {file}"

[actions.diff_view]
key = "d"
tmux = "tmux popup -w 80% -h 80% 'delta {file}'"
fallback = "delta {file}"
```

Template variables: `{file}` (relative path), `{abs_file}` (absolute path), `{diff}` (temp file with diff output)

## Key Design Decisions

- **No polling**: All updates are event-driven via filesystem notifications
- **Single expanded file**: Accordion pattern keeps the UI predictable and avoids layout complexity
- **Lazy diffs**: Only compute what's visible to stay responsive in large repos
- **Zero-config useful**: Works immediately with sensible defaults, config only needed for actions/customization
- **Multiplexer-agnostic**: Works in plain terminal, enhances when tmux/zellij/wezterm detected
- **Pure Rust git**: gitoxide over shelling out to git CLI for speed and reliability

## Build & Run

```bash
cargo build --release
# Run in any git repository:
cd /path/to/your/repo
git-rt
```

## CLI Flags (planned)

```
git-rt [OPTIONS] [PATH]

Arguments:
  [PATH]  Path to git repository (defaults to current directory)

Options:
  -c, --config <FILE>     Path to config file
  -d, --debounce <MS>     Debounce interval in milliseconds [default: 200]
      --no-color          Disable colored output
      --log <LEVEL>       Enable logging (trace, debug, info, warn, error)
  -h, --help              Print help
  -V, --version           Print version
```

## Development Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run tests
cargo clippy                   # Lint
cargo fmt                      # Format
RUST_LOG=debug cargo run       # Run with debug logging
```

## Implementation Priority

### Phase 1 — MVP (current)
- [x] Project scaffold and module structure
- [ ] Git status computation via gix (file list with status)
- [ ] Diff numstat computation (insertions/deletions per file)
- [ ] Basic ratatui rendering (file list with diff stats)
- [ ] Keyboard navigation (j/k, arrows, q to quit)
- [ ] Expand/collapse single file diff (Enter/l/h)
- [ ] Filesystem watching with debounce
- [ ] Wire up event loop (terminal + fs + tick)

### Phase 2 — Polish
- [ ] Syntax-colored diff output (red/green/cyan)
- [ ] Scrollable diff within expanded region
- [ ] Handle edge cases (index.lock, mid-rebase, empty repo)
- [ ] Mouse support (click to select/expand)
- [ ] Status bar (branch name, total changes, last update time)
- [ ] Respect .gitignore for watch filtering

### Phase 3 — Actions & Config
- [ ] Config file loading (TOML, XDG paths)
- [ ] Multiplexer detection
- [ ] Action system with template resolution
- [ ] Default action presets (editor, diff viewer, claude)
- [ ] Custom keybinding configuration

### Phase 4 — Advanced
- [ ] Tree view mode (directory structure)
- [ ] Staged vs unstaged split view
- [ ] Multiple display modes (compact, expanded, tree)
- [ ] Virtual scrolling for large repos
- [ ] Watch multiple repos

## Crate Versions (as of project creation)

- ratatui: 0.29
- crossterm: 0.28
- gix: 0.68
- notify: 7.0
- notify-debouncer-full: 0.4
- clap: 4
- serde: 1
- toml: 0.8

## Code Style

- Use `thiserror` for library-style errors in each module, `anyhow` in main/app for ergonomic error propagation
- Prefer channels (crossbeam) over async — this is a synchronous TUI app
- Keep git operations off the main thread — run in a background thread, send results via channel
- All public functions should have doc comments
- Module-level `mod.rs` files should re-export the public API cleanly
