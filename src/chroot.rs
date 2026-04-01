//! Chroot operations for finalizing a bootc rootfs: bind-mount host
//! filesystems, run depmod/dracut/preset-all/bootupd, then unmount.

use std::path::Path;

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;

/// Run all chroot operations needed to finalize a bootc rootfs:
/// depmod, dracut initramfs generation, systemd presets, and bootupd
/// metadata.
///
/// Bind-mounts /proc, /sys, /dev into the rootfs before running,
/// and unmounts them when done.
///
/// On non-Linux platforms this is a no-op since chroot into a Linux
/// rootfs is not possible.
#[cfg(target_os = "linux")]
pub fn run_chroot_operations(rootfs: &Path) -> Result<()> {
    let kver = detect_kernel_version(rootfs)?;

    // Temporarily restore /root as a real directory for dracut.
    // dracut-install fails if /root is a dangling symlink (which it is
    // after toplevel_symlinks converts it to var/roothome).
    let root_path = rootfs.join("root");
    let root_was_symlink = root_path.is_symlink();
    if root_was_symlink {
        let target = std::fs::read_link(&root_path)
            .with_context(|| format!("reading symlink {}", root_path.display()))?;
        std::fs::remove_file(&root_path)
            .with_context(|| format!("removing symlink {}", root_path.display()))?;
        std::fs::create_dir_all(&root_path)
            .with_context(|| format!("creating dir {}", root_path.display()))?;
        tracing::debug!(
            "temporarily converted /root symlink -> real dir for dracut (was -> {})",
            target.display()
        );
    }

    // Mount /proc, /sys, /dev into rootfs for chroot operations.
    // Use type mounts for proc/sys (works in chroot-isolated CI)
    // and --rbind for /dev (recursive, includes /dev/pts etc).
    let proc_target = rootfs.join("proc");
    let sys_target = rootfs.join("sys");
    let dev_target = rootfs.join("dev");
    for target in [&proc_target, &sys_target, &dev_target] {
        std::fs::create_dir_all(target)
            .with_context(|| format!("creating {}", target.display()))?;
    }
    mount_fs("proc", "proc", &proc_target)?;
    mount_fs("sysfs", "sys", &sys_target)?;
    rbind_mount("/dev", &dev_target)?;

    let result = (|| -> Result<()> {
        depmod(rootfs, &kver)?;
        generate_initramfs(rootfs, &kver)?;
        preset_all(rootfs)?;
        bootupd_generate(rootfs)?;
        Ok(())
    })();

    // Always unmount, even if operations failed. Reverse order: dev, sys, proc.
    // Use -R for /dev since it was --rbind mounted.
    if let Err(e) = umount_recursive(&dev_target) {
        tracing::warn!("failed to unmount {}: {e}", dev_target.display());
    }
    if let Err(e) = umount(&sys_target) {
        tracing::warn!("failed to unmount {}: {e}", sys_target.display());
    }
    if let Err(e) = umount(&proc_target) {
        tracing::warn!("failed to unmount {}: {e}", proc_target.display());
    }

    // Restore /root symlink
    if root_was_symlink {
        std::fs::remove_dir_all(&root_path)
            .with_context(|| format!("removing temp dir {}", root_path.display()))?;
        std::os::unix::fs::symlink("var/roothome", &root_path)
            .with_context(|| format!("restoring symlink {}", root_path.display()))?;
        tracing::debug!("restored /root -> var/roothome symlink");
    }

    result
}

#[cfg(not(target_os = "linux"))]
pub fn run_chroot_operations(rootfs: &Path) -> Result<()> {
    tracing::warn!(
        "chroot operations skipped: not running on Linux (rootfs: {})",
        rootfs.display()
    );
    Ok(())
}

/// Detect the installed kernel version by scanning `usr/lib/modules/`
/// inside the rootfs for the first directory entry.
#[cfg(target_os = "linux")]
fn detect_kernel_version(rootfs: &Path) -> Result<String> {
    let modules_dir = rootfs.join("usr/lib/modules");

    let entries = std::fs::read_dir(&modules_dir)
        .with_context(|| format!("reading {}", modules_dir.display()))?;

    for entry in entries {
        let entry = entry.with_context(|| format!("reading entry in {}", modules_dir.display()))?;
        if entry.path().is_dir() {
            let kver = entry
                .file_name()
                .to_str()
                .with_context(|| "kernel version directory name is not valid UTF-8")?
                .to_string();
            tracing::info!("detected kernel version: {kver}");
            return Ok(kver);
        }
    }

    anyhow::bail!("no kernel version found in {}", modules_dir.display())
}

/// Run a command inside the rootfs via `chroot`.
#[cfg(target_os = "linux")]
fn run_in_chroot(rootfs: &Path, program: &str, args: &[&str]) -> Result<()> {
    tracing::debug!("chroot {} {} {}", rootfs.display(), program, args.join(" "));

    let status = std::process::Command::new("chroot")
        .arg(rootfs)
        .arg(program)
        .args(args)
        .status()
        .with_context(|| format!("running chroot {program}"))?;

    anyhow::ensure!(status.success(), "{program} failed with {status}");
    Ok(())
}

/// Mount a filesystem by type (e.g., proc, sysfs).
#[cfg(target_os = "linux")]
fn mount_fs(fstype: &str, source: &str, target: &Path) -> Result<()> {
    let status = std::process::Command::new("mount")
        .args(["-t", fstype, source])
        .arg(target)
        .status()
        .with_context(|| format!("mount -t {fstype} {source} {}", target.display()))?;
    anyhow::ensure!(status.success(), "mount -t {fstype} failed with {status}");
    tracing::debug!("mounted {fstype} on {}", target.display());
    Ok(())
}

/// Recursive bind mount (for /dev which has submounts like /dev/pts).
#[cfg(target_os = "linux")]
fn rbind_mount(source: &str, target: &Path) -> Result<()> {
    let status = std::process::Command::new("mount")
        .args(["--rbind", source])
        .arg(target)
        .status()
        .with_context(|| format!("mount --rbind {source} {}", target.display()))?;
    anyhow::ensure!(
        status.success(),
        "mount --rbind {source} failed with {status}"
    );
    tracing::debug!("rbind mounted {source} -> {}", target.display());
    Ok(())
}

/// Recursive unmount (for --rbind mounts).
#[cfg(target_os = "linux")]
fn umount_recursive(target: &Path) -> Result<()> {
    let status = std::process::Command::new("umount")
        .args(["-R"])
        .arg(target)
        .status()
        .with_context(|| format!("umount -R {}", target.display()))?;
    anyhow::ensure!(
        status.success(),
        "umount -R {} failed with {status}",
        target.display()
    );
    tracing::debug!("recursively unmounted {}", target.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn umount(target: &Path) -> Result<()> {
    let status = std::process::Command::new("umount")
        .arg(target)
        .status()
        .with_context(|| format!("umount {}", target.display()))?;
    anyhow::ensure!(
        status.success(),
        "umount {} failed with {status}",
        target.display()
    );
    tracing::debug!("unmounted {}", target.display());
    Ok(())
}

/// Generate kernel module dependency files.
#[cfg(target_os = "linux")]
fn depmod(rootfs: &Path, kver: &str) -> Result<()> {
    tracing::info!("running depmod for {kver}");
    run_in_chroot(rootfs, "depmod", &[kver]).with_context(|| format!("depmod {kver}"))
}

/// Generate the initramfs using dracut.
///
/// Output to /usr/lib/modules/{kver}/initramfs.img which is where
/// bootc expects it for ostree deployments.
#[cfg(target_os = "linux")]
fn generate_initramfs(rootfs: &Path, kver: &str) -> Result<()> {
    tracing::info!("generating initramfs for {kver}");
    let output = format!("/usr/lib/modules/{kver}/initramfs.img");
    run_in_chroot(
        rootfs,
        "dracut",
        &["--no-hostonly", "--kver", kver, "--force", &output],
    )
    .with_context(|| format!("dracut --kver {kver}"))
}

/// Apply systemd presets (both system and user/global).
///
/// First removes any unit symlinks RPM scriptlets created in /etc/systemd/,
/// then runs preset-all so presets are the canonical source of enabled state.
/// This matches what rpm-ostree compose does (see rpm-ostree#1803).
#[cfg(target_os = "linux")]
fn preset_all(rootfs: &Path) -> Result<()> {
    tracing::info!("applying systemd presets");

    // Clear RPM-scriptlet-created unit symlinks so presets are canonical
    for subdir in ["etc/systemd/system", "etc/systemd/user"] {
        let dir = rootfs.join(subdir);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("recreating {}", dir.display()))?;
            tracing::debug!("cleared {subdir} before preset-all");
        }
    }

    run_in_chroot(rootfs, "systemctl", &["preset-all"]).with_context(|| "systemctl preset-all")?;
    run_in_chroot(rootfs, "systemctl", &["--user", "--global", "preset-all"])
        .with_context(|| "systemctl --user --global preset-all")?;
    Ok(())
}

/// Bootupd metadata JSON structure.
#[cfg(target_os = "linux")]
#[derive(Debug, serde::Serialize)]
struct BootupdMeta {
    timestamp: String,
    version: String,
    versions: Vec<BootupdVersion>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, serde::Serialize)]
struct BootupdVersion {
    name: String,
    rpm_evr: String,
}

/// Get the architecture-specific EFI package names.
#[cfg(target_os = "linux")]
fn efi_package_names() -> (&'static str, &'static str) {
    #[cfg(target_arch = "x86_64")]
    {
        ("grub2-efi-x64", "shim-x64")
    }
    #[cfg(target_arch = "aarch64")]
    {
        ("grub2-efi-aa64", "shim-aa64")
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        ("grub2-efi-x64", "shim-x64")
    } // fallback to x86_64
}

/// Generate bootupd update metadata.
///
/// bootupctl cannot run inside a chroot (it tries to unmount /boot/efi),
/// so we generate the metadata manually: copy EFI binaries from /boot/efi/
/// to /usr/lib/bootupd/updates/ and write the JSON manifests.
///
/// This matches what rpm-ostree compose does internally.
#[cfg(target_os = "linux")]
fn bootupd_generate(rootfs: &Path) -> Result<()> {
    let bootupctl = rootfs.join("usr/bin/bootupctl");
    if !bootupctl.exists() {
        tracing::debug!("bootupctl not found, skipping bootupd metadata generation");
        return Ok(());
    }

    tracing::info!("generating bootupd update metadata");

    let updates_dir = rootfs.join("usr/lib/bootupd/updates");
    std::fs::create_dir_all(&updates_dir)
        .with_context(|| format!("creating {}", updates_dir.display()))?;

    // Generate BIOS metadata only if grub2-install exists (indicates BIOS
    // bootloader support is available). Without grub2-pc-modules, a BIOS
    // install would fail, so only write BIOS.json when the full BIOS chain
    // is present.
    let grub2_install = rootfs.join("usr/sbin/grub2-install");
    let bios_modules = rootfs.join("usr/lib/grub/i386-pc");
    if grub2_install.exists() && bios_modules.is_dir() {
        let grub2_ver = rpm_query_evr(rootfs, "grub2-tools");
        if let Some(evr) = &grub2_ver {
            let meta = BootupdMeta {
                timestamp: utc_timestamp(),
                version: format!("grub2-tools-{evr}"),
                versions: vec![BootupdVersion {
                    name: "grub2".into(),
                    rpm_evr: evr.clone(),
                }],
            };
            let bios_json = serde_json::to_string_pretty(&meta).context("serializing BIOS.json")?;
            std::fs::write(updates_dir.join("BIOS.json"), &bios_json)
                .context("writing BIOS.json")?;
            tracing::debug!("wrote BIOS.json (grub2 {evr})");
        }
    } else {
        tracing::debug!(
            "skipping BIOS.json: grub2-install or i386-pc modules not found (EFI-only image)"
        );
    }

    // Generate EFI metadata: copy binaries + write JSON
    let efi_src = rootfs.join("boot/efi/EFI");
    if efi_src.is_dir() {
        let efi_dest = updates_dir.join("EFI");
        copy_dir_recursive(&efi_src, &efi_dest)
            .with_context(|| format!("copying EFI files to {}", efi_dest.display()))?;

        let (grub_efi_pkg, shim_pkg) = efi_package_names();
        let grub_efi_ver = rpm_query_evr(rootfs, grub_efi_pkg);
        let shim_ver = rpm_query_evr(rootfs, shim_pkg);

        let mut versions = Vec::new();
        let mut version_parts = Vec::new();
        if let Some(evr) = &grub_efi_ver {
            versions.push(BootupdVersion {
                name: "grub2".into(),
                rpm_evr: evr.clone(),
            });
            version_parts.push(format!("{grub_efi_pkg}-{evr}"));
        }
        if let Some(evr) = &shim_ver {
            versions.push(BootupdVersion {
                name: "shim".into(),
                rpm_evr: evr.clone(),
            });
            version_parts.push(format!("{shim_pkg}-{evr}"));
        }

        let meta = BootupdMeta {
            timestamp: utc_timestamp(),
            version: version_parts.join(","),
            versions,
        };
        let efi_json = serde_json::to_string_pretty(&meta).context("serializing EFI.json")?;
        std::fs::write(updates_dir.join("EFI.json"), &efi_json).context("writing EFI.json")?;
        tracing::debug!("wrote EFI.json");
    } else {
        tracing::debug!("no /boot/efi/EFI found, skipping EFI metadata");
    }

    Ok(())
}

/// Get the EVR (epoch:version-release) of an installed RPM in the rootfs.
#[cfg(target_os = "linux")]
fn rpm_query_evr(rootfs: &Path, pkg: &str) -> Option<String> {
    let output = std::process::Command::new("chroot")
        .arg(rootfs)
        .args(["rpm", "-q", "--qf", "%{EVR}", pkg])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

/// UTC timestamp in ISO 8601 format for bootupd JSON metadata.
#[cfg(target_os = "linux")]
fn utc_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let remaining = secs % 86400;
    let h = remaining / 3600;
    let m = (remaining % 3600) / 60;
    let s = remaining % 60;
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Convert days since 1970-01-01 to (year, month, day) using
/// Howard Hinnant's civil_from_days algorithm.
#[cfg(target_os = "linux")]
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Recursively copy a directory tree.
#[cfg(target_os = "linux")]
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path).with_context(|| {
                format!("copying {} -> {}", src_path.display(), dest_path.display())
            })?;
        }
    }
    Ok(())
}
