//! Filesystem helpers for the dev-install xtask.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// Symlink each named binary from `target_dir` into `bin_dir`.
///
/// Replaces an existing symlink at the destination, but refuses to clobber a
/// real (non-symlink) file. Creates `bin_dir` if it does not exist.
pub fn link_binaries(bin_dir: &Path, target_dir: &Path, names: &[&str]) -> Result<()> {
    fs::create_dir_all(bin_dir)
        .with_context(|| format!("creating bin dir {}", bin_dir.display()))?;

    for name in names {
        let src = target_dir.join(name);
        let dest = bin_dir.join(name);

        match dest.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                fs::remove_file(&dest)
                    .with_context(|| format!("removing existing symlink {}", dest.display()))?;
            }
            Ok(_) => bail!(
                "refusing to overwrite real file {} (not a symlink)",
                dest.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("inspecting {}", dest.display())),
        }

        std::os::unix::fs::symlink(&src, &dest)
            .with_context(|| format!("symlinking {} -> {}", dest.display(), src.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn links_fresh_binary() {
        let bin = tempdir().unwrap();
        let target = tempdir().unwrap();
        fs::write(target.path().join("perch"), b"binary").unwrap();

        link_binaries(bin.path(), target.path(), &["perch"]).unwrap();

        let link = bin.path().join("perch");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), target.path().join("perch"));
    }

    #[test]
    fn relink_replaces_existing_symlink() {
        let bin = tempdir().unwrap();
        let target = tempdir().unwrap();
        fs::write(target.path().join("perch"), b"binary").unwrap();

        link_binaries(bin.path(), target.path(), &["perch"]).unwrap();
        // second call must not error
        link_binaries(bin.path(), target.path(), &["perch"]).unwrap();

        let link = bin.path().join("perch");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    }

    #[test]
    fn refuses_to_clobber_real_file() {
        let bin = tempdir().unwrap();
        let target = tempdir().unwrap();
        let real = bin.path().join("perch");
        fs::write(&real, b"do not delete me").unwrap();

        let err = link_binaries(bin.path(), target.path(), &["perch"]).unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));
        assert_eq!(fs::read(&real).unwrap(), b"do not delete me");
    }
}
