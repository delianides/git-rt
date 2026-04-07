# Simplify Action Config

## Summary

Replace the multiplexer-aware action system with a simple key + command model. Remove `Multiplexer` detection entirely. Actions are purely user-defined with no defaults.

## Config Shape

Before:
```toml
[actions.open_editor]
key = "e"
tmux = "tmux split-window -h 'nvim {file}'"
zellij = "zellij run --direction right -- nvim {file}"
wezterm = "wezterm cli split-pane --right -- nvim {file}"
fallback = "nvim {file}"
```

After:
```toml
[actions.open_editor]
key = "e"
command = "nvim {file}"
```

- Template variables: `{file}` (relative path), `{abs_file}` (absolute path)
- No default actions shipped — `actions` map starts empty
- Built-in navigation keys (`keys` section) remain separate and unchanged

## Removals

- `Multiplexer` enum and `Multiplexer::detect()` from `config/mod.rs`
- `tmux`, `zellij`, `wezterm`, `fallback` fields on `ActionConfig`
- `default_actions()` function
- `action_is_blocking()` from `actions/mod.rs`
- Mux parameter from `execute_action()` and `resolve_action_command()`
- All tests covering multiplexer detection, mux-specific resolution, and default actions

## Changes

### `src/config/mod.rs`

- `ActionConfig` becomes `{ key: String, command: String }`
- `AppConfig::default()` sets `actions` to empty `HashMap`
- `resolve_action_command()` simplifies to template substitution only — no mux parameter, returns `command.replace("{file}", ...).replace("{abs_file}", ...)`

### `src/actions/mod.rs`

- `execute_action()` drops `mux` parameter, runs command via `sh -c`
- Remove `action_is_blocking()`

### `src/app.rs`

- Update call sites that pass `Multiplexer` to action functions

### `CLAUDE.md`

- Update action system docs, config example, and Phase 3 checklist (remove multiplexer detection item)

### `README.md`

- Update config example to reflect new action format
