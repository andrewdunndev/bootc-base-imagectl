use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

/// Create a minimal mock rootfs that simulates a `dnf --installroot` output.
/// This is enough structure for all PreChroot and PostChroot transforms to
/// run without error (Chroot transforms are skipped since there's no kernel).
fn create_mock_rootfs(dir: &Path) {
    // Real directories that will be converted to symlinks
    for d in ["home", "root", "opt", "srv", "mnt", "media", "usr/local"] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }

    // /var structure
    for d in [
        "var/cache/dnf",
        "var/tmp",
        "var/log",
        "var/lib/rpm",
        "var/run",
    ] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }

    // Fake rpmdb
    fs::write(dir.join("var/lib/rpm/rpmdb.sqlite"), b"fake-rpmdb").unwrap();

    // /etc
    fs::create_dir_all(dir.join("etc/dnf")).unwrap();
    fs::write(dir.join("etc/passwd"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();
    fs::write(dir.join("etc/group"), "root:x:0:\n").unwrap();
    fs::write(dir.join("etc/machine-id"), "abcdef1234567890abcdef1234567890\n").unwrap();
    fs::write(dir.join("etc/dnf/dnf.conf"), "[main]\n").unwrap();

    // /usr structure
    for d in [
        "usr/lib/tmpfiles.d",
        "usr/lib/systemd/system/local-fs.target.wants",
        "usr/lib/systemd/system",
        "usr/lib/dracut/dracut.conf.d",
        "usr/lib/ostree",
        "usr/lib/kernel/install.conf.d",
        "usr/lib/rpm/macros.d",
        "usr/share",
    ] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }

    // tmp.mount unit file (for the symlink target)
    fs::write(
        dir.join("usr/lib/systemd/system/tmp.mount"),
        "[Mount]\nWhat=tmpfs\nWhere=/tmp\nType=tmpfs\n",
    )
    .unwrap();

    // A provision.conf with /root references
    fs::write(
        dir.join("usr/lib/tmpfiles.d/provision.conf"),
        "d /root 0700 root root -\nd- /root 0700 root root -\n",
    )
    .unwrap();

    // home.conf (should be removed by var_tmpfiles)
    fs::write(dir.join("usr/lib/tmpfiles.d/home.conf"), "d /home 0755 root root -\n").unwrap();

    // /boot with fake initramfs
    fs::create_dir_all(dir.join("boot")).unwrap();
    fs::write(dir.join("boot/vmlinuz-test"), b"fake-kernel").unwrap();
    fs::write(dir.join("boot/initramfs-test.img"), b"fake-initramfs").unwrap();

    // /run and /tmp
    fs::create_dir_all(dir.join("run")).unwrap();
    fs::create_dir_all(dir.join("tmp")).unwrap();
}

#[test]
fn test_finalize_creates_symlinks() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    // Check symlinks
    assert!(dir.path().join("home").is_symlink());
    assert_eq!(
        fs::read_link(dir.path().join("home"))?.to_str().unwrap(),
        "var/home"
    );
    assert!(dir.path().join("root").is_symlink());
    assert_eq!(
        fs::read_link(dir.path().join("root"))?.to_str().unwrap(),
        "var/roothome"
    );
    assert!(dir.path().join("ostree").is_symlink());
    assert!(dir.path().join("sysroot").is_dir());
    Ok(())
}

#[test]
fn test_finalize_creates_tmpfiles() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    let conf = dir.path().join("usr/lib/tmpfiles.d/bootc-base-var.conf");
    assert!(conf.exists());
    let content = fs::read_to_string(&conf)?;
    assert!(content.contains("d /var/home 0755"));
    assert!(content.contains("d /var/roothome 0700"));
    assert!(content.contains("d /var/lib/rpm-state 0755"));

    // home.conf should be removed
    assert!(!dir.path().join("usr/lib/tmpfiles.d/home.conf").exists());
    Ok(())
}

#[test]
fn test_finalize_relocates_rpmdb() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    assert!(dir.path().join("usr/share/rpm/rpmdb.sqlite").exists());
    assert!(dir.path().join("var/lib/rpm").is_symlink());
    assert!(dir.path().join("usr/lib/rpm/macros.d/macros.rpm-ostree").exists());
    Ok(())
}

#[test]
fn test_finalize_ostree_config() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    let prepare = dir.path().join("usr/lib/ostree/prepare-root.conf");
    let content = fs::read_to_string(&prepare)?;
    assert!(content.contains("[composefs]"));
    assert!(content.contains("enabled = true"));
    assert!(content.contains("[sysroot]"));
    assert!(content.contains("readonly = true"));
    Ok(())
}

#[test]
fn test_finalize_empties_machine_id() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    let machine_id = fs::read_to_string(dir.path().join("etc/machine-id"))?;
    assert!(machine_id.is_empty(), "machine-id should be empty, got: {machine_id:?}");
    Ok(())
}

#[test]
fn test_finalize_relocates_boot() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    assert!(dir.path().join("usr/lib/ostree-boot/vmlinuz-test").exists());
    assert!(dir.path().join("usr/lib/ostree-boot/initramfs-test.img").exists());
    // boot should be empty
    let boot_entries: Vec<_> = fs::read_dir(dir.path().join("boot"))?.collect();
    assert!(boot_entries.is_empty(), "boot/ should be empty after relocate");
    Ok(())
}

#[test]
fn test_finalize_fixes_provision_conf() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    let content = fs::read_to_string(dir.path().join("usr/lib/tmpfiles.d/provision.conf"))?;
    assert!(!content.contains(" /root "), "provision.conf should not contain /root");
    assert!(content.contains("/var/roothome"));
    Ok(())
}

#[test]
fn test_finalize_idempotent() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();

    // First run
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest.clone(),
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    // Second run should succeed without errors
    let mut ctx2 = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx2)?;
    Ok(())
}

#[test]
fn test_lint_check_mode() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let manifest = bootc_base_imagectl::manifest::ImageManifest::default();

    // Check mode should not modify anything
    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        true, // check_only
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    // home should still be a real directory (not converted to symlink)
    assert!(dir.path().join("home").is_dir());
    assert!(!dir.path().join("home").is_symlink());
    Ok(())
}

#[test]
fn test_manifest_skip() -> Result<()> {
    let dir = TempDir::new()?;
    create_mock_rootfs(dir.path());

    let mut manifest = bootc_base_imagectl::manifest::ImageManifest::default();
    manifest.transforms.skip = vec!["toplevel_symlinks".to_string()];

    let mut ctx = bootc_base_imagectl::transforms::Context::new(
        dir.path().to_path_buf(),
        manifest,
        false,
    )?;
    bootc_base_imagectl::transforms::run_all(&mut ctx)?;

    // Symlinks should NOT have been created (transform was skipped)
    assert!(dir.path().join("home").is_dir());
    assert!(!dir.path().join("home").is_symlink());

    // But other transforms should have run
    assert!(dir.path().join("usr/lib/ostree/prepare-root.conf").exists());
    Ok(())
}
