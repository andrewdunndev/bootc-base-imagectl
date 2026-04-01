//! Boot content relocation to /usr/lib/ostree-boot/ for ostree management.

use anyhow::{Context, Result};

use super::{Phase, Transform};

/// Relocate /boot contents to /usr/lib/ostree-boot/ so that the boot
/// partition is managed by ostree/bootc rather than containing files
/// directly.
#[derive(Debug)]
pub struct BootRelocate;

const BOOT_DIR: &str = "boot";
const OSTREE_BOOT_DIR: &str = "usr/lib/ostree-boot";

impl Transform for BootRelocate {
    fn name(&self) -> &str {
        "boot_relocate"
    }

    fn phase(&self) -> Phase {
        Phase::PostChroot
    }

    fn check(&self, ctx: &super::Context) -> Result<bool> {
        let dir = ctx.dir();

        if !dir.exists(OSTREE_BOOT_DIR) {
            return Ok(false);
        }

        // Check that /boot is empty
        if dir.is_dir(BOOT_DIR) {
            let count = dir
                .read_dir(BOOT_DIR)
                .with_context(|| format!("reading {BOOT_DIR}"))?
                .count();
            if count > 0 {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn apply(&self, ctx: &super::Context) -> Result<()> {
        let dir = ctx.dir();

        dir.create_dir_all(OSTREE_BOOT_DIR)
            .with_context(|| format!("creating {OSTREE_BOOT_DIR}"))?;

        // Use cp -a + rm instead of rename, since /boot and /usr may be
        // on different filesystems (common in chroot-isolated CI builds).
        if dir.is_dir(BOOT_DIR) {
            let boot_path = ctx.rootfs_path.join(BOOT_DIR);
            let dest_path = ctx.rootfs_path.join(OSTREE_BOOT_DIR);

            for entry in std::fs::read_dir(&boot_path)
                .with_context(|| format!("reading {}", boot_path.display()))?
            {
                let entry =
                    entry.with_context(|| format!("reading entry in {}", boot_path.display()))?;
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let from = entry.path();
                let to = dest_path.join(&*name_str);

                // Copy (preserving attributes) then remove original
                let status = std::process::Command::new("cp")
                    .args(["-a"])
                    .arg(&from)
                    .arg(&to)
                    .status()
                    .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
                anyhow::ensure!(status.success(), "cp -a failed for {}", name_str);

                if from.is_dir() && !from.is_symlink() {
                    std::fs::remove_dir_all(&from)
                        .with_context(|| format!("removing {}", from.display()))?;
                } else {
                    std::fs::remove_file(&from)
                        .with_context(|| format!("removing {}", from.display()))?;
                }
                tracing::debug!("moved boot/{name_str}");
            }
        }

        // Ensure /boot exists as an empty directory
        if !dir.exists(BOOT_DIR) {
            dir.create_dir_all(BOOT_DIR)
                .with_context(|| format!("creating {BOOT_DIR}"))?;
        }

        tracing::debug!("relocated boot contents to {OSTREE_BOOT_DIR}");

        Ok(())
    }
}
