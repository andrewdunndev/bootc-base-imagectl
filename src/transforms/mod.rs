//! Transform pipeline: ordered filesystem mutations that convert a
//! dnf --installroot rootfs into a bootc-compatible layout.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use cap_std_ext::cap_std;
use cap_std::fs::Dir;
use fn_error_context::context;

use crate::manifest::ImageManifest;

mod boot_relocate;
mod chroot_ops;
mod cleanup;
mod dnf_config;
mod dracut_config;
mod ostree_config;
mod passwd;
mod rpmdb;
mod symlinks;
mod systemd_config;
mod var_tmpfiles;

/// Phase in the transform pipeline. Transforms within a phase run in
/// dependency order; phases run sequentially.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// Filesystem layout changes before any chroot operations
    PreChroot,
    /// Operations that require chroot/nsenter into the rootfs
    Chroot,
    /// Cleanup after chroot operations complete
    PostChroot,
}

/// Result of a single transform's check or apply.
#[derive(Debug)]
pub enum TransformResult {
    Applied,
    AlreadyCorrect,
    CheckFailed(String),
    Skipped(String),
}

impl std::fmt::Display for TransformResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Applied => write!(f, "applied"),
            Self::AlreadyCorrect => write!(f, "already correct"),
            Self::CheckFailed(reason) => write!(f, "check failed: {reason}"),
            Self::Skipped(reason) => write!(f, "skipped: {reason}"),
        }
    }
}

/// Execution context shared across all transforms.
#[derive(Debug)]
pub struct Context {
    /// Cap-std directory handle scoped to the rootfs.
    pub rootfs_dir: Dir,
    /// Raw path to rootfs (needed for chroot operations that require real paths).
    pub rootfs_path: PathBuf,
    pub manifest: ImageManifest,
    pub check_only: bool,
    results: Vec<(String, TransformResult)>,
}

impl Context {
    pub fn new(rootfs: PathBuf, manifest: ImageManifest, check_only: bool) -> Result<Self> {
        let rootfs_dir = Dir::open_ambient_dir(&rootfs, cap_std::ambient_authority())
            .with_context(|| format!("opening rootfs dir {}", rootfs.display()))?;
        Ok(Self {
            rootfs_dir,
            rootfs_path: rootfs,
            manifest,
            check_only,
            results: Vec::new(),
        })
    }

    /// Return a reference to the cap-std Dir scoped to the rootfs.
    pub fn dir(&self) -> &Dir {
        &self.rootfs_dir
    }

    /// Resolve a path relative to the rootfs (for operations that need
    /// real paths, e.g. chroot, symlink checks via std::fs).
    pub fn path(&self, relative: &str) -> PathBuf {
        self.rootfs_path.join(relative.trim_start_matches('/'))
    }

    pub fn record(&mut self, name: String, result: TransformResult) {
        tracing::info!("{name}: {result}");
        self.results.push((name, result));
    }

    pub fn results(&self) -> (usize, usize) {
        let pass = self
            .results
            .iter()
            .filter(|(_, r)| {
                matches!(
                    r,
                    TransformResult::Applied | TransformResult::AlreadyCorrect | TransformResult::Skipped(..)
                )
            })
            .count();
        let fail = self
            .results
            .iter()
            .filter(|(_, r)| matches!(r, TransformResult::CheckFailed(_)))
            .count();
        (pass, fail)
    }
}

/// A single rootfs transform. Each transform knows how to check whether it
/// has already been applied and how to apply itself.
pub trait Transform {
    fn name(&self) -> &str;
    fn phase(&self) -> Phase;

    /// Check whether this transform is needed. Returns Ok(true) if the rootfs
    /// already satisfies this transform, Ok(false) if it needs to be applied.
    fn check(&self, ctx: &Context) -> Result<bool>;

    /// Apply the transform to the rootfs. Only called when check() returns false.
    fn apply(&self, ctx: &Context) -> Result<()>;
}

/// Remove all contents of a directory without removing the directory itself.
#[context("Clearing directory {:?}", relative)]
pub(crate) fn clear_dir(dir: &Dir, relative: &str) -> Result<()> {
    let sub = dir.open_dir(relative)
        .with_context(|| format!("opening directory {relative}"))?;
    for entry in sub.entries()
        .with_context(|| format!("reading directory {relative}"))?
    {
        let entry = entry
            .with_context(|| format!("reading entry in {relative}"))?;
        let name = entry.file_name();
        let ft = entry.file_type()
            .with_context(|| format!("reading file type of {name:?} in {relative}"))?;
        if ft.is_dir() {
            sub.remove_dir_all(&name)
                .with_context(|| format!("removing {name:?} in {relative}"))?;
        } else {
            sub.remove_file(&name)
                .with_context(|| format!("removing {name:?} in {relative}"))?;
        }
    }
    Ok(())
}

/// Run all transforms in phase order.
pub fn run_all(ctx: &mut Context) -> Result<()> {
    let transforms = all_transforms();

    for phase in [Phase::PreChroot, Phase::Chroot, Phase::PostChroot] {
        for t in transforms.iter().filter(|t| t.phase() == phase) {
            let name = t.name().to_string();

            if ctx.manifest.transforms.skip.iter().any(|s| s == &name) {
                ctx.record(name, TransformResult::Skipped("skipped by manifest".into()));
                continue;
            }

            match t.check(ctx) {
                Ok(true) => {
                    ctx.record(name, TransformResult::AlreadyCorrect);
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    ctx.record(name, TransformResult::CheckFailed(e.to_string()));
                    continue;
                }
            }

            if ctx.check_only {
                ctx.record(name, TransformResult::CheckFailed("needs apply".into()));
                continue;
            }

            match t.apply(ctx) {
                Ok(()) => ctx.record(name, TransformResult::Applied),
                Err(e) => {
                    anyhow::bail!("transform {name} failed: {e}");
                }
            }
        }
    }

    Ok(())
}

fn all_transforms() -> Vec<Box<dyn Transform>> {
    vec![
        // PreChroot
        Box::new(symlinks::ToplevelSymlinks),
        Box::new(var_tmpfiles::VarTmpfiles),
        Box::new(rpmdb::RpmdbRelocate),
        Box::new(passwd::PasswdGenerate),
        Box::new(ostree_config::OstreeConfig),
        Box::new(dracut_config::DracutConfig),
        Box::new(dnf_config::DnfConfig),
        Box::new(systemd_config::SystemdConfig),
        // Chroot
        Box::new(chroot_ops::ChrootOps),
        // PostChroot
        Box::new(boot_relocate::BootRelocate),
        Box::new(cleanup::PostCleanup),
    ]
}
