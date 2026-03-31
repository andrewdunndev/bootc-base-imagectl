//! rpm-ostree treefile parsing with recursive include resolution.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Partial representation of an rpm-ostree treefile.
/// Only extracts fields needed for package resolution.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Treefile {
    pub packages: Vec<String>,
    pub repos: Vec<String>,
    pub releasever: Option<String>,
    pub units: Vec<String>,
    pub recommends: Option<bool>,
    pub documentation: Option<bool>,
    #[serde(deserialize_with = "deserialize_include", default)]
    pub include: Vec<String>,
    // We intentionally ignore all other fields for compatibility
}

/// Deserialize the `include` field which can be either a single string
/// or a list of strings in treefile YAML.
fn deserialize_include<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Single(String),
        Multiple(Vec<String>),
    }

    match Option::<StringOrVec>::deserialize(deserializer)? {
        Some(StringOrVec::Single(s)) => Ok(vec![s]),
        Some(StringOrVec::Multiple(v)) => Ok(v),
        None => Ok(Vec::new()),
    }
}

/// Load a treefile from `path`, recursively resolving `include` directives.
///
/// Included files are resolved relative to the parent directory of the
/// file being parsed. Fields from the root treefile take precedence for
/// scalar values (`releasever`, `recommends`, `documentation`); list
/// fields (`packages`, `repos`, `units`) are merged with deduplication.
pub fn load(path: &Path) -> Result<Treefile> {
    let path = path
        .canonicalize()
        .with_context(|| format!("resolving treefile path {}", path.display()))?;

    load_inner(&path)
}

fn load_inner(path: &Path) -> Result<Treefile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading treefile {}", path.display()))?;

    let mut treefile: Treefile = serde_yaml_ng::from_str(&contents)
        .with_context(|| format!("parsing treefile {}", path.display()))?;

    let base_dir = path
        .parent()
        .with_context(|| format!("resolving parent directory of {}", path.display()))?;

    // Recursively resolve includes
    let includes = std::mem::take(&mut treefile.include);
    for inc in &includes {
        let inc_path = base_dir.join(inc);
        let included = load_inner(&inc_path)
            .with_context(|| format!("loading included treefile {}", inc_path.display()))?;
        treefile.merge(&included);
    }
    treefile.include = includes;

    Ok(treefile)
}

impl Treefile {
    /// Merge another treefile into this one. Appends list fields with
    /// deduplication. Scalar fields from `self` take precedence (the
    /// parent treefile wins).
    pub fn merge(&mut self, other: &Treefile) {
        merge_dedup(&mut self.packages, &other.packages);
        merge_dedup(&mut self.repos, &other.repos);
        merge_dedup(&mut self.units, &other.units);

        if self.releasever.is_none() {
            self.releasever.clone_from(&other.releasever);
        }
        if self.recommends.is_none() {
            self.recommends = other.recommends;
        }
        if self.documentation.is_none() {
            self.documentation = other.documentation;
        }
    }
}

/// Append items from `source` into `target`, skipping duplicates.
fn merge_dedup(target: &mut Vec<String>, source: &[String]) {
    let existing: HashSet<String> = target.iter().cloned().collect();
    for item in source {
        if !existing.contains(item) {
            target.push(item.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_include_single_string() {
        let yaml = "include: base.yaml\npackages:\n  - vim\n";
        let tf: Treefile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(tf.include, vec!["base.yaml"]);
    }

    #[test]
    fn deserialize_include_list() {
        let yaml = "include:\n  - base.yaml\n  - extras.yaml\npackages:\n  - vim\n";
        let tf: Treefile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(tf.include, vec!["base.yaml", "extras.yaml"]);
    }

    #[test]
    fn deserialize_no_include() {
        let yaml = "packages:\n  - vim\n";
        let tf: Treefile = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(tf.include.is_empty());
    }

    #[test]
    fn merge_deduplicates() {
        let mut base = Treefile {
            packages: vec!["vim".into(), "git".into()],
            releasever: Some("42".into()),
            ..Default::default()
        };
        let child = Treefile {
            packages: vec!["git".into(), "curl".into()],
            releasever: Some("41".into()),
            recommends: Some(false),
            ..Default::default()
        };
        base.merge(&child);
        assert_eq!(base.packages, vec!["vim", "git", "curl"]);
        // Parent releasever wins
        assert_eq!(base.releasever.as_deref(), Some("42"));
        // Child recommends fills in the blank
        assert_eq!(base.recommends, Some(false));
    }

    #[test]
    fn unknown_fields_ignored() {
        let yaml = "packages:\n  - vim\nautomatic-version-prefix: '42'\nselinux: true\n";
        let tf: Treefile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(tf.packages, vec!["vim"]);
    }
}
