mod install;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use install::{link_binaries, unlink_binary};

/// Binaries the dev tool manages. Intentionally only `perch` (not `git-perch`).
const BINARIES: &[&str] = &["perch"];

fn main() -> Result<()> {
    match env::args().nth(1).as_deref() {
        Some("install") => install_cmd(),
        Some("uninstall") => uninstall_cmd(),
        other => bail!(
            "usage: cargo run -p xtask -- <install|uninstall> (got {:?})",
            other
        ),
    }
}

/// xtask's manifest dir is `<root>/xtask`; its parent is the workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent dir")
        .to_path_buf()
}

fn target_debug_dir() -> PathBuf {
    let base = match env::var("CARGO_TARGET_DIR") {
        Ok(dir) => {
            let p = PathBuf::from(dir);
            if p.is_absolute() {
                p
            } else {
                workspace_root().join(p)
            }
        }
        Err(_) => workspace_root().join("target"),
    };
    base.join("debug")
}

fn bin_dir() -> Result<PathBuf> {
    let home = env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local").join("bin"))
}

fn install_cmd() -> Result<()> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(&cargo)
        .args(["build", "--bin", "perch"])
        .current_dir(workspace_root())
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    let bin = bin_dir()?;
    let target = target_debug_dir();

    for name in BINARIES {
        let built = target.join(name);
        if !built.exists() {
            bail!(
                "expected built binary {} not found after build",
                built.display()
            );
        }
    }

    link_binaries(&bin, &target, BINARIES)?;

    for name in BINARIES {
        println!(
            "linked {} -> {}",
            bin.join(name).display(),
            target.join(name).display()
        );
    }

    warn_path(&bin);
    Ok(())
}

fn uninstall_cmd() -> Result<()> {
    let bin = bin_dir()?;
    for name in BINARIES {
        if unlink_binary(&bin, name)? {
            println!("removed {}", bin.join(name).display());
        } else {
            println!("{} not installed; nothing to do", bin.join(name).display());
        }
    }
    Ok(())
}

/// Warn if `bin` is not on PATH, or if an earlier PATH entry shadows our binary.
fn warn_path(bin: &Path) {
    let path = env::var("PATH").unwrap_or_default();
    let entries: Vec<PathBuf> = env::split_paths(&path).collect();

    if !entries.iter().any(|p| p == bin) {
        eprintln!(
            "warning: {} is not on your PATH; add it to run `perch` from anywhere",
            bin.display()
        );
        return;
    }

    for entry in &entries {
        if entry == bin {
            break;
        }
        if entry.join("perch").exists() {
            eprintln!(
                "warning: {} is earlier on PATH and will shadow the dev build",
                entry.join("perch").display()
            );
            break;
        }
    }
}
