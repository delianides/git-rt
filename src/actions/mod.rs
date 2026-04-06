use std::process::Command;

use anyhow::{Context, Result};

use crate::config::Multiplexer;

/// Execute a resolved action command string.
///
/// In a multiplexer context, the command spawns a new pane/popup,
/// so the TUI keeps running. In a plain terminal, we'd need to
/// suspend the TUI first (handled by the caller).
pub fn execute_action(command: &str, mux: &Multiplexer) -> Result<()> {
    tracing::info!(%command, ?mux, "Executing action");

    Command::new("sh")
        .arg("-c")
        .arg(command)
        .spawn()
        .with_context(|| format!("Failed to execute action: {command}"))?;

    Ok(())
}

/// Check if the action will spawn in a new pane (non-blocking)
/// or replace the current terminal (blocking, needs TUI suspend)
pub fn action_is_blocking(mux: &Multiplexer) -> bool {
    matches!(mux, Multiplexer::None)
}
