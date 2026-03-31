//! Post-chroot cleanup: cache removal and stale rpmdb WAL/SHM cleanup.

use anyhow::{Context, Result};

use super::{Phase, Transform, clear_dir};

/// Final cleanup pass: remove caches, stale rpmdb WAL files, and
/// ensure ephemeral directories are empty.
#[derive(Debug)]
pub struct PostCleanup;

/// Directories whose contents should be removed entirely.
const CLEAR_DIRS: &[&str] = &["var/cache", "var/tmp", "var/log", "run", "tmp"];

/// Stale rpmdb sidecar files to remove.
const RPMDB_STALE_FILES: &[&str] = &["rpmdb.sqlite-wal", "rpmdb.sqlite-shm"];

/// Directories that may contain stale rpmdb files.
const RPMDB_DIRS: &[&str] = &["usr/share/rpm", "var/lib/rpm"];

impl Transform for PostCleanup {
    fn name(&self) -> &str {
        "post_cleanup"
    }

    fn phase(&self) -> Phase {
        Phase::PostChroot
    }

    fn check(&self, _ctx: &super::Context) -> Result<bool> {
        // Cleanup should always run
        Ok(false)
    }

    fn apply(&self, ctx: &super::Context) -> Result<()> {
        let dir = ctx.dir();

        // Clear cache, tmp, log, run, tmp directories
        for subdir in CLEAR_DIRS {
            if dir.is_dir(subdir) {
                clear_dir(dir, subdir).with_context(|| format!("clearing {subdir}"))?;
                tracing::debug!("cleared {subdir}");
            }
        }

        // Remove stale rpmdb WAL/SHM files
        for rpmdb_dir in RPMDB_DIRS {
            // Follow symlinks: if var/lib/rpm is a symlink, that's fine,
            // we still want to clean the target.
            if !dir.exists(rpmdb_dir) {
                continue;
            }
            for stale in RPMDB_STALE_FILES {
                let path = format!("{rpmdb_dir}/{stale}");
                if dir.exists(&path) {
                    dir.remove_file(&path)
                        .with_context(|| format!("removing {path}"))?;
                    tracing::debug!("removed {rpmdb_dir}/{stale}");
                }
            }
        }

        Ok(())
    }
}
