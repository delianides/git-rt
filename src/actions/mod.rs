use std::process::Command;

use anyhow::{Context, Result};

/// Execute a resolved action command string.
///
/// The command is spawned as a child process via `sh -c`.
pub fn execute_action(command: &str) -> Result<()> {
    tracing::info!(%command, "Executing action");

    Command::new("sh")
        .arg("-c")
        .arg(command)
        .spawn()
        .with_context(|| format!("Failed to execute action: {command}"))?;

    Ok(())
}
