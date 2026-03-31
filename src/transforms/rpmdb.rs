//! RPM database relocation to the immutable /usr partition.

use anyhow::{Context, Result};
use cap_std::fs::Dir;
use cap_std_ext::cap_std;

use super::{Phase, Transform};

/// Relocate the RPM database from /var/lib/rpm to /usr/share/rpm so that
/// it lives on the immutable /usr partition.
#[derive(Debug)]
pub struct RpmdbRelocate;

/// Possible source locations for the rpmdb (checked in order).
const SRC_DIRS: &[&str] = &[
    "usr/lib/sysimage/rpm", // dnf5 / Fedora 42+
    "var/lib/rpm",          // dnf4 / traditional
];
const DEST_DIR: &str = "usr/share/rpm";
const VAR_LINK: &str = "var/lib/rpm";
const MACROS_DIR: &str = "usr/lib/rpm/macros.d";
const MACROS_FILE: &str = "macros.rpm-ostree";

impl Transform for RpmdbRelocate {
    fn name(&self) -> &str {
        "rpmdb_relocate"
    }

    fn phase(&self) -> Phase {
        Phase::PreChroot
    }

    fn check(&self, ctx: &super::Context) -> Result<bool> {
        let dir = ctx.dir();
        Ok(dir.exists(format!("{DEST_DIR}/rpmdb.sqlite")))
    }

    fn apply(&self, ctx: &super::Context) -> Result<()> {
        let dir = ctx.dir();

        dir.create_dir_all(DEST_DIR)
            .with_context(|| format!("creating {DEST_DIR}"))?;

        // Find the source rpmdb directory
        let src = SRC_DIRS
            .iter()
            .find(|d| {
                let is_real_dir = dir.symlink_metadata(d).map(|m| m.is_dir()).unwrap_or(false);
                is_real_dir && has_db_files(dir, d)
            })
            .copied();

        let src = match src {
            Some(s) => {
                tracing::debug!("found rpmdb at {s}");
                s
            }
            None => {
                tracing::warn!("no rpmdb found at any known location, creating empty dest");
                SRC_DIRS[0]
            }
        };

        // Move all files from source to /usr/share/rpm/
        let is_real_dir = dir
            .symlink_metadata(src)
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if is_real_dir {
            let src_dir = dir
                .open_dir(src)
                .with_context(|| format!("opening {src}"))?;
            for entry in src_dir
                .entries()
                .with_context(|| format!("reading {src}"))?
            {
                let entry = entry.with_context(|| format!("reading entry in {src}"))?;
                let file_name = entry.file_name();
                let from = format!("{src}/{}", file_name.to_string_lossy());
                let to = format!("{DEST_DIR}/{}", file_name.to_string_lossy());

                dir.rename(&from, dir, &to)
                    .with_context(|| format!("moving {from} -> {to}"))?;
                tracing::debug!("moved {}", file_name.to_string_lossy());
            }

            // Remove the now-empty source directory
            dir.remove_dir_all(src)
                .with_context(|| format!("removing {src}"))?;
        }

        // Ensure /var/lib/rpm is a symlink to the dest
        let var_link_is_real_dir = dir
            .symlink_metadata(VAR_LINK)
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if dir.exists(VAR_LINK) && var_link_is_real_dir {
            dir.remove_dir_all(VAR_LINK)
                .with_context(|| format!("removing {VAR_LINK}"))?;
        }
        let var_link_is_symlink = dir
            .symlink_metadata(VAR_LINK)
            .map(|m| m.is_symlink())
            .unwrap_or(false);
        if !var_link_is_symlink {
            if let Some(parent) = std::path::Path::new(VAR_LINK).parent() {
                if !parent.as_os_str().is_empty() {
                    dir.create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
            }
            dir.symlink("../../usr/share/rpm", VAR_LINK)
                .with_context(|| format!("creating symlink {VAR_LINK}"))?;
            tracing::debug!("symlink var/lib/rpm -> ../../usr/share/rpm");
        }

        // Write rpm macros to redirect %_dbpath
        dir.create_dir_all(MACROS_DIR)
            .with_context(|| format!("creating {MACROS_DIR}"))?;

        let macros_path = format!("{MACROS_DIR}/{MACROS_FILE}");
        dir.write(&macros_path, "%_dbpath /usr/share/rpm\n")
            .with_context(|| format!("writing {macros_path}"))?;
        tracing::debug!("wrote {MACROS_DIR}/{MACROS_FILE}");

        Ok(())
    }
}

fn has_db_files(dir: &Dir, subdir: &str) -> bool {
    let Ok(sub) = dir.open_dir(subdir) else {
        return false;
    };
    sub.entries()
        .map(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.ends_with(".sqlite") || name.ends_with(".db")
            })
        })
        .unwrap_or(false)
}
