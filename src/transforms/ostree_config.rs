//! ostree prepare-root.conf and kernel-install layout configuration.

use anyhow::{Context, Result};

use super::{Phase, Transform};

/// Write ostree and kernel-install configuration so bootc recognizes
/// the rootfs as composefs-enabled with a readonly sysroot.
#[derive(Debug)]
pub struct OstreeConfig;

const PREPARE_ROOT_DIR: &str = "usr/lib/ostree";
const PREPARE_ROOT_CONF: &str = "prepare-root.conf";

const KERNEL_INSTALL_DIR: &str = "usr/lib/kernel";
const KERNEL_INSTALL_CONF: &str = "install.conf";

const KERNEL_INSTALL_DROP_DIR: &str = "usr/lib/kernel/install.conf.d";
const KERNEL_INSTALL_DROP_CONF: &str = "00-bootc-kernel-layout.conf";

impl Transform for OstreeConfig {
    fn name(&self) -> &str {
        "ostree_config"
    }

    fn phase(&self) -> Phase {
        Phase::PreChroot
    }

    fn check(&self, ctx: &super::Context) -> Result<bool> {
        Ok(ctx.dir().exists(format!("{PREPARE_ROOT_DIR}/{PREPARE_ROOT_CONF}")))
    }

    fn apply(&self, ctx: &super::Context) -> Result<()> {
        let dir = ctx.dir();

        // /usr/lib/ostree/prepare-root.conf
        dir.create_dir_all(PREPARE_ROOT_DIR)
            .with_context(|| format!("creating {PREPARE_ROOT_DIR}"))?;

        let prepare_root_path = format!("{PREPARE_ROOT_DIR}/{PREPARE_ROOT_CONF}");
        dir.write(
            &prepare_root_path,
            "[composefs]\nenabled = true\n\n[sysroot]\nreadonly = true\n",
        )
        .with_context(|| format!("writing {prepare_root_path}"))?;
        tracing::debug!("wrote {PREPARE_ROOT_DIR}/{PREPARE_ROOT_CONF}");

        // /usr/lib/kernel/install.conf
        dir.create_dir_all(KERNEL_INSTALL_DIR)
            .with_context(|| format!("creating {KERNEL_INSTALL_DIR}"))?;

        let install_conf_path = format!("{KERNEL_INSTALL_DIR}/{KERNEL_INSTALL_CONF}");
        dir.write(&install_conf_path, "layout=ostree\n")
            .with_context(|| format!("writing {install_conf_path}"))?;
        tracing::debug!("wrote {KERNEL_INSTALL_DIR}/{KERNEL_INSTALL_CONF}");

        // /usr/lib/kernel/install.conf.d/00-bootc-kernel-layout.conf
        dir.create_dir_all(KERNEL_INSTALL_DROP_DIR)
            .with_context(|| format!("creating {KERNEL_INSTALL_DROP_DIR}"))?;

        let drop_conf_path = format!("{KERNEL_INSTALL_DROP_DIR}/{KERNEL_INSTALL_DROP_CONF}");
        dir.write(&drop_conf_path, "layout=ostree\n")
            .with_context(|| format!("writing {drop_conf_path}"))?;
        tracing::debug!("wrote {KERNEL_INSTALL_DROP_DIR}/{KERNEL_INSTALL_DROP_CONF}");

        Ok(())
    }
}
