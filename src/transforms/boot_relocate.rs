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

        if dir.is_dir(BOOT_DIR) {
            for entry in dir
                .read_dir(BOOT_DIR)
                .with_context(|| format!("reading {BOOT_DIR}"))?
            {
                let entry = entry.with_context(|| format!("reading entry in {BOOT_DIR}"))?;
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                let from = format!("{BOOT_DIR}/{name_str}");
                let to = format!("{OSTREE_BOOT_DIR}/{name_str}");

                dir.rename(&from, dir, &to)
                    .with_context(|| format!("moving {from} -> {to}"))?;
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
