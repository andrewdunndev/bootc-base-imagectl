//! Image manifest loading and parsing. Supports both TOML manifests and
//! rpm-ostree treefile YAML with automatic conversion.

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::Deserialize;

/// Primary manifest format (TOML). Configures which transforms run
/// and their parameters.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImageManifest {
    pub image: ImageSection,
    pub packages: PackagesSection,
    pub repos: ReposSection,
    pub transforms: TransformsSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ImageSection {
    pub name: String,
    pub releasever: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PackagesSection {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ReposSection {
    /// Additional .repo file paths to inject into the installroot.
    /// Reserved for future use when build-rootfs supports custom repos
    /// instead of --use-host-config.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TransformsSection {
    /// Transforms to skip by name
    pub skip: Vec<String>,
    pub dracut: DracutOptions,
    pub systemd: SystemdOptions,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DracutOptions {
    pub extra_modules: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SystemdOptions {
    pub enable_units: Vec<String>,
    pub disable_timers: Vec<String>,
}

/// Load a manifest from a TOML file or treefile YAML.
///
/// Files ending in `.yaml` or `.yml` are loaded as rpm-ostree
/// treefiles and converted to an `ImageManifest`. All other files
/// are parsed as TOML.
pub fn load(path: &Utf8Path) -> Result<ImageManifest> {
    if path
        .extension()
        .is_some_and(|ext| ext == "yaml" || ext == "yml")
    {
        let treefile = crate::treefile::load(path.as_std_path())
            .with_context(|| format!("loading treefile {path}"))?;
        return Ok(ImageManifest::from(treefile));
    }

    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading manifest {path}"))?;

    toml::from_str(&contents).with_context(|| format!("parsing manifest {path}"))
}

impl From<crate::treefile::Treefile> for ImageManifest {
    fn from(tf: crate::treefile::Treefile) -> Self {
        Self {
            image: ImageSection {
                name: String::new(),
                releasever: tf.releasever.unwrap_or_default(),
            },
            packages: PackagesSection {
                include: tf.packages,
                exclude: Vec::new(),
            },
            repos: ReposSection { paths: tf.repos },
            transforms: TransformsSection {
                skip: Vec::new(),
                dracut: DracutOptions::default(),
                systemd: SystemdOptions {
                    enable_units: tf.units,
                    disable_timers: Vec::new(),
                },
            },
        }
    }
}
