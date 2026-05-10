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
│   └── mod.rs        # Status, numstat, CLI shell-out helpers
├── state/            # Application state / view model
│   └── mod.rs        # FileEntry list, selection, flash state
├── ui/               # Rendering via ratatui
│   └── mod.rs        # Layout, file list, status line, help overlay
└── config/           # Configuration loading and defaults
    └── mod.rs        # TOML parsing, XDG paths, default keybindings
```

## Core Concepts

### Default View (zero-config)

```
 src/main.rs          -3  +12
 src/watcher.rs       -0  +45
 src/config.rs        -10  +2
 tests/integration.rs -1  +1
```

- Each line shows a changed file path with red deletion count and green addition count
- Navigation via `j/k` or arrow keys
- `Enter`, `l`, `Right`, `Space`, or `d` opens the selected file's diff in an in-app overlay (centered 85% panel, scrollable with `j`/`k`, dismissible with `Esc`/`q`/`h`/`Left`)
- `q` quits
- The viewport keeps a configurable `scroll_padding` of rows (default 3) visible above and below the selected row — set `display.scroll_padding` in `config.toml` to change it (`0` disables).

### Event Loop

The main loop multiplexes three event sources via crossbeam channels:

1. **Terminal events** (key presses, mouse, resize) from crossterm
2. **Filesystem events** from notify (debounced ~500ms by default)
3. **Tick events** for periodic work (~1s)

When a filesystem event fires:

1. Watcher sends `FsChange` to the app
2. App enqueues a `Recompute` request on the worker thread
3. Worker runs `git status --porcelain=v2` + `git diff --numstat`, returns a `StatusBundle`
4. State updates the file list
5. UI re-renders on the next iteration

### Git Integration

`git-rt` uses `git` (the CLI) for the hot-path status walk and `gix` (gitoxide) for cheap reads.

- **File status**: `git status --porcelain=v2 -z` parsed natively — much faster than gix's walk on large repos thanks to git's untracked cache + fsmonitor.
- **Diff numstat**: `git diff --numstat -z <merge-base>` for branch view, `git diff --numstat -z` for the working-tree view.
- **Cheap reads**: branch name, HEAD commit, merge-base, stash count, ahead/behind still use `gix` — sub-millisecond.
- **Diff content**: rendered in an in-app overlay (see `src/ui/diff_overlay.rs`) — centered 85% panel with colored `+`/`-`/context lines and line numbers, scrollable with `j`/`k`.
- **Base branch resolution**: the diff range's "base" is the repo trunk. Priority: explicit `--base` flag → `display.base_branch` config → `origin/HEAD` symbolic-ref target → `origin/main` → `origin/master`. Sibling local branches are never chosen.

### Filesystem Watching

- Uses `notify` with `notify-debouncer-full` for cross-platform support (inotify/FSEvents/kqueue)
- Watches the entire working tree, filters out `.git/` directory changes (except `.git/index` for staged changes)
- Debounce window: 500ms default, configurable
- On debounce fire: full git status recomputation via worker thread
- A git-rt instance is pinned to one worktree for its lifetime. Background filesystem activity in *other* worktrees does not move the watched path. Use the `s`-key dialog to switch deliberately, or relaunch git-rt against a different path.

## Key Design Decisions

- **No polling**: All updates are event-driven via filesystem notifications
- **Off-thread git**: all status work runs on a dedicated worker thread
- **Zero-config useful**: Works immediately with sensible defaults
- **Hybrid git**: `git` CLI for the hot status path, `gix` for cheap reads — pragmatic over purity

## Build & Run

```bash
cargo build --release
# Run in any git repository:
cd /path/to/your/repo
git-rt
```

## CLI Flags

```
git-rt [OPTIONS] [PATH]

Arguments:
  [PATH]  Path to git repository or worktree (defaults to current directory)

Options:
  -c, --config <FILE>     Path to config file
  -d, --debounce <MS>     Debounce interval in milliseconds [default: 500]
      --log <LEVEL>       Enable logging (trace, debug, info, warn, error)
      --theme <NAME|PATH> Theme override (built-in name or path to .toml theme file)
      --base <BRANCH>     Base branch for the branch-scoped diff range
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

## Current Status

Core feature set is complete: live file-list with numstat, PR status strip, in-app diff overlay, filesystem watching, config file + keybindings, themes, multi-worktree support, and branch-scoped diff range.

Remaining open items:

- [ ] Handle edge cases (index.lock, mid-rebase, empty repo)
- [ ] Mouse support (click to select)
- [ ] Tree view mode (directory structure)
- [ ] Staged vs unstaged split view
- [ ] Virtual scrolling for large repos
- [ ] Watch multiple repos

## Crate Versions

Source of truth is `Cargo.toml`. Current pins:

- ratatui: 0.29
- crossterm: 0.28
- gix: 0.81
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

## Commit Guidelines

Use conventional commits:

- feat: new features
- fix: bug fixes
- docs: documentation
- refactor: code refactoring
- chore: maintenance

## Testing

- **All new work MUST include test cases that cover the new functionality.** No exceptions.
- Tests live in `#[cfg(test)] mod tests` blocks at the bottom of each module
- Use `cargo test` to run the full suite
- Unit tests should cover: state transitions, config parsing/defaults, git output parsing, and any pure logic
- Integration tests requiring a real git repo should use `tempfile` to create disposable repos
- TUI/rendering code is exempt from unit tests but should be validated manually
