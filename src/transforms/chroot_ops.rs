//! Chroot operation orchestration: depmod, dracut, presets, bootupd.

use anyhow::{Context, Result};

use super::{Phase, Transform};

/// Runs chroot operations on the rootfs: depmod, dracut initramfs
/// generation, systemd preset-all, and bootupd metadata.
#[derive(Debug)]
pub struct ChrootOps;

impl Transform for ChrootOps {
    fn name(&self) -> &str {
        "chroot_ops"
    }

    fn phase(&self) -> Phase {
        Phase::Chroot
    }

    fn check(&self, ctx: &super::Context) -> Result<bool> {
        let dir = ctx.dir();
        if !dir.is_dir("usr/lib/modules") {
            // No kernel installed, nothing to do
            return Ok(true);
        }

        // Check whether every installed kernel already has an initramfs.
        // After boot_relocate, the initramfs may be in usr/lib/ostree-boot/
        // instead of boot/, so check both locations.
        let modules = dir.open_dir("usr/lib/modules")
            .context("opening usr/lib/modules")?;
        for entry in modules.entries()
            .context("reading usr/lib/modules")?
        {
            let entry = entry.context("reading entry in usr/lib/modules")?;
            let ft = entry.file_type().context("file type in usr/lib/modules")?;
            if ft.is_dir() {
                let kver = entry.file_name().to_string_lossy().to_string();
                let in_boot = dir.exists(format!("boot/initramfs-{kver}.img"));
                let in_ostree_boot = dir.exists(format!("usr/lib/ostree-boot/initramfs-{kver}.img"));
                if !in_boot && !in_ostree_boot {
                    return Ok(false);
                }
            }
        }

        // Check bootupd metadata if bootupctl is installed
        if dir.exists("usr/bin/bootupctl")
            && !dir.exists("usr/lib/bootupd/updates")
        {
            return Ok(false);
        }

        Ok(true)
    }

    fn apply(&self, ctx: &super::Context) -> Result<()> {
        crate::chroot::run_chroot_operations(&ctx.rootfs_path)
    }
}
