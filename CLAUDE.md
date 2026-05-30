# CLAUDE.md — perch

## Project Overview

perch is a real-time terminal dashboard that watches git working tree changes and displays them as a live-updating TUI. Think of it as a persistent, interactive `git status` + `git diff --numstat` that runs in a terminal pane.

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
    └── mod.rs        # TOML parsing, XDG paths, CLI-flag overrides
```

The repo is a Cargo workspace. The only non-root member is `xtask/` — dev tooling (`publish = false`, `dist = false`, never shipped in releases) that provides the `cargo dev-install` / `cargo run -p xtask -- uninstall` commands.

## Core Concepts

### File List (zero-config)

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

### View Modes

perch has three view modes, cycled with `m` (`Normal → Condensed → Tree → Normal`):

- **Normal** (default) — files split into collapsible status groups:
  **Changes** (staged/unstaged edits), **New files** (untracked), and
  **Committed** (committed on the branch, no pending edits). Empty groups are
  hidden. `Enter`/`Space` on a group header collapses it. The Committed group
  needs a resolved base branch; with none, Changes and New still render and
  Committed is silently hidden.
- **Condensed** — a single flat list of changed file paths.
- **Tree** — files arranged as a directory tree.

Set the startup mode with `display.default_view = "normal" | "condensed" | "tree"`
in `config.toml` (default `"normal"`).

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

`perch` uses `git` (the CLI) for the hot-path status walk and `gix` (gitoxide) for cheap reads.

- **File status**: `git status --porcelain=v2 -z` parsed natively — much faster than gix's walk on large repos thanks to git's untracked cache + fsmonitor.
- **Diff numstat**: `git diff --numstat -z <merge-base>` for branch view, `git diff --numstat -z` for the working-tree view.
- **Cheap reads**: branch name, HEAD commit, merge-base, stash count, ahead/behind still use `gix` — sub-millisecond.
- **Diff content**: rendered in an in-app overlay (see `src/ui/diff_overlay.rs`) — centered 85% panel with colored `+`/`-`/context lines and line numbers, scrollable with `j`/`k`.
- **Base branch resolution**: the diff range's base is resolved through four
  tiers, each reading a recorded git fact (not a name guess):
  1. Explicit `--base` flag or top-level `base_branch` config
  2. Branch reflog fork point — the start-point recorded by `git branch <name>
     <start>` or `git worktree add -b <name> <start>` (`logs/refs/heads/<branch>`
     in the common git dir). Implicit `git checkout -b foo` writes `Created from
     HEAD` and falls through.
  3. Main worktree HEAD branch — the trunk checked out in the primary worktree,
     read from `<common-git-dir>/HEAD`. Self-skips when it equals the current
     branch.
  4. `origin/HEAD` symbolic-ref target.

  Stacked branches resolve to their literal fork point — a branch created from
  `feature1` diffs against `feature1`, not trunk. Pass `--base` to override.
  If no tier resolves, branch-scoped data is unavailable: Condensed and Tree
  fall back to working-tree status, while Normal renders the Changes and New
  groups and hides the Committed group.

### Filesystem Watching

- Uses `notify` with `notify-debouncer-full` for cross-platform support (inotify/FSEvents/kqueue)
- Watches the entire working tree, filters out `.git/` directory changes (except `.git/index` for staged changes)
- Debounce window: 500ms default, configurable
- On debounce fire: full git status recomputation via worker thread
- A perch instance is pinned to one worktree for its lifetime. Background filesystem activity in *other* worktrees does not move the watched path. Use the `s`-key dialog to switch deliberately, or relaunch perch against a different path.

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
perch
```

## CLI Flags

```
perch [OPTIONS] [PATH]

Arguments:
  [PATH]  Path to git repository or worktree (defaults to current directory)

Options:
  -c, --config <FILE>         Path to config file
  -d, --debounce <MS>         Debounce interval in milliseconds [default: 500]
      --log <LEVEL>           Enable logging (trace, debug, info, warn, error)
      --base <BRANCH>         Base branch for the branch-scoped diff range
      --no-pr                 Disable the GitHub PR status strip
      --view <MODE>           Startup view (normal, condensed, tree)
      --no-flash              Disable the row flash on change
      --flash-duration <MS>   Flash duration in milliseconds
      --scroll-padding <N>    Rows kept visible above/below the selection
      --edit-command <CMD>    Editor command for the `e` key
  -h, --help                  Print help
  -V, --version               Print version
```

Flags that mirror config keys override the config file (precedence: flag > config > default).

## Development Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run tests
cargo clippy                   # Lint
cargo fmt                      # Format
RUST_LOG=debug cargo run       # Run with debug logging
cargo dev-install              # symlink the debug build into ~/.local/bin (xtask crate)
```

## Current Status

Core feature set is complete: live file-list with numstat, status-grouped Normal view (plus Condensed and Tree view modes), PR status strip, in-app diff overlay, filesystem watching, config file (with mirroring CLI flags), multi-worktree support, and branch-scoped diff range. Colors follow the terminal's ANSI palette (no theming).

Remaining open items:

- [ ] Handle edge cases (index.lock, mid-rebase, empty repo)
- [ ] Mouse support (click to select)
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
