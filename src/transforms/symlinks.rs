//! Toplevel symlink creation for ostree/bootc filesystem layout.

use anyhow::{Context, Result};

use super::{Phase, Transform};

/// Standard ostree toplevel symlinks. Converts real directories into
/// symlinks pointing at /var or /run equivalents.
#[derive(Debug)]
pub struct ToplevelSymlinks;

/// (target_link, symlink_destination)
/// The link is created AT target_link, pointing TO symlink_destination.
const TOPLEVEL_LINKS: &[(&str, &str)] = &[
    ("home", "var/home"),
    ("root", "var/roothome"),
    ("opt", "var/opt"),
    ("srv", "var/srv"),
    ("mnt", "var/mnt"),
    ("media", "run/media"),
    ("usr/local", "../var/usrlocal"),
];

const DIRS_TO_CREATE: &[&str] = &["sysroot"];

const OSTREE_LINK: (&str, &str) = ("ostree", "sysroot/ostree");

impl Transform for ToplevelSymlinks {
    fn name(&self) -> &str {
        "toplevel_symlinks"
    }

    fn phase(&self) -> Phase {
        Phase::PreChroot
    }

    fn check(&self, ctx: &super::Context) -> Result<bool> {
        let dir = ctx.dir();

        for (link, target) in TOPLEVEL_LINKS {
            let meta = dir.symlink_metadata(link);
            match meta {
                Ok(m) if m.is_symlink() => {
                    let actual = dir.read_link(link)
                        .with_context(|| format!("reading symlink {link}"))?;
                    if actual != std::path::Path::new(target) {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }

        if !dir.is_dir("sysroot") {
            return Ok(false);
        }

        let meta = dir.symlink_metadata(OSTREE_LINK.0);
        match meta {
            Ok(m) if m.is_symlink() => {}
            _ => return Ok(false),
        }

        Ok(true)
    }

    fn apply(&self, ctx: &super::Context) -> Result<()> {
        let dir = ctx.dir();

        for d in DIRS_TO_CREATE {
            dir.create_dir_all(d)
                .with_context(|| format!("creating {d}"))?;
        }

        for (link, target) in TOPLEVEL_LINKS {
            // Remove existing directory or file
            let is_symlink = dir.symlink_metadata(link).map(|m| m.is_symlink()).unwrap_or(false);
            let exists_follow = dir.exists(link);

            if exists_follow && !is_symlink {
                if dir.is_dir(link) {
                    dir.remove_dir_all(link)
                        .with_context(|| format!("removing dir {link}"))?;
                } else {
                    dir.remove_file(link)
                        .with_context(|| format!("removing file {link}"))?;
                }
            } else if is_symlink {
                dir.remove_file(link)
                    .with_context(|| format!("removing old symlink {link}"))?;
            }

            // Ensure parent exists
            if let Some(parent) = std::path::Path::new(link).parent() {
                if !parent.as_os_str().is_empty() {
                    dir.create_dir_all(parent)
                        .with_context(|| format!("creating parent {}", parent.display()))?;
                }
            }

            dir.symlink(target, link)
                .with_context(|| format!("creating symlink {link} -> {target}"))?;
            tracing::debug!("{link} -> {target}");
        }

        // ostree -> sysroot/ostree
        let ostree_is_symlink = dir.symlink_metadata(OSTREE_LINK.0).map(|m| m.is_symlink()).unwrap_or(false);
        let ostree_exists = dir.exists(OSTREE_LINK.0);

        if ostree_exists || ostree_is_symlink {
            if dir.is_dir(OSTREE_LINK.0) && !ostree_is_symlink {
                dir.remove_dir_all(OSTREE_LINK.0)
                    .with_context(|| format!("removing {}", OSTREE_LINK.0))?;
            } else {
                dir.remove_file(OSTREE_LINK.0)
                    .with_context(|| format!("removing {}", OSTREE_LINK.0))?;
            }
        }
        dir.symlink(OSTREE_LINK.1, OSTREE_LINK.0)
            .context("creating ostree symlink")?;

        Ok(())
    }
}
