//! passwd/group file propagation to the immutable /usr/lib layer.

use anyhow::{Context, Result};

use super::{Phase, Transform};

/// Copy /etc/passwd and /etc/group into /usr/lib so they are available
/// on the immutable rootfs before /etc is mounted.
#[derive(Debug)]
pub struct PasswdGenerate;

impl Transform for PasswdGenerate {
    fn name(&self) -> &str {
        "passwd_generate"
    }

    fn phase(&self) -> Phase {
        Phase::PreChroot
    }

    fn check(&self, ctx: &super::Context) -> Result<bool> {
        Ok(ctx.dir().exists("usr/lib/passwd"))
    }

    fn apply(&self, ctx: &super::Context) -> Result<()> {
        let dir = ctx.dir();

        dir.create_dir_all("usr/lib")
            .context("creating usr/lib")?;

        let passwd = dir.read_to_string("etc/passwd")
            .context("reading etc/passwd")?;
        dir.write("usr/lib/passwd", &passwd)
            .context("writing usr/lib/passwd")?;
        tracing::debug!("copied etc/passwd -> usr/lib/passwd");

        let group = dir.read_to_string("etc/group")
            .context("reading etc/group")?;
        dir.write("usr/lib/group", &group)
            .context("writing usr/lib/group")?;
        tracing::debug!("copied etc/group -> usr/lib/group");

        Ok(())
    }
}
