//! CLI entry point for bootc-base-imagectl.

use anyhow::{Context, Result};
use camino::Utf8Path;
use clap::Parser;
use fn_error_context::context;

use bootc_base_imagectl::{manifest, transforms};

/// Transforms a dnf --installroot rootfs into a bootc-compatible layout.
///
/// A Rust reimplementation of bootc-base-imagectl, designed for the
/// dnf --installroot build path rather than rpm-ostree compose.
#[derive(Parser)]
#[command(version, about)]
enum Cli {
    /// Apply all transforms to convert a rootfs into a bootc-compatible layout
    Finalize(FinalizeArgs),
    /// Check a rootfs for bootc compatibility without modifying it
    Lint(LintArgs),
    /// Build a rootfs from scratch using dnf --installroot, then finalize
    BuildRootfs(BuildRootfsArgs),
    /// Rechunk an OCI image into reproducible RPM-split layers
    Rechunk(RechunkArgs),
}

#[derive(clap::Args)]
struct FinalizeArgs {
    /// Path to the rootfs to transform
    #[arg(long)]
    rootfs: std::path::PathBuf,
    /// Path to a TOML manifest (optional; uses defaults if not provided)
    #[arg(long)]
    manifest: Option<std::path::PathBuf>,
    /// Dry-run: show what would be done without modifying anything
    #[arg(long)]
    check: bool,
}

#[derive(clap::Args)]
struct LintArgs {
    /// Path to the rootfs to check
    #[arg(long)]
    rootfs: std::path::PathBuf,
}

#[derive(clap::Args)]
struct BuildRootfsArgs {
    /// Path to a TOML or treefile YAML manifest
    #[arg(long)]
    manifest: std::path::PathBuf,
    /// Target directory for the rootfs
    target: std::path::PathBuf,
    /// Fedora release version (overrides manifest)
    #[arg(long)]
    releasever: Option<String>,
    /// Additional packages to install (can be repeated)
    #[arg(long = "install")]
    extra_packages: Vec<String>,
    /// Skip finalize transforms after installing packages
    #[arg(long)]
    no_finalize: bool,
}

#[derive(clap::Args)]
struct RechunkArgs {
    /// Source OCI image reference
    from: String,
    /// Target OCI image reference
    to: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli {
        Cli::Finalize(args) => cmd_finalize(args),
        Cli::Lint(args) => cmd_lint(args),
        Cli::BuildRootfs(args) => cmd_build_rootfs(args),
        Cli::Rechunk(args) => cmd_rechunk(args),
    }
}

#[context("Finalizing rootfs")]
fn cmd_finalize(args: FinalizeArgs) -> Result<()> {
    let manifest = match &args.manifest {
        Some(path) => {
            let path = Utf8Path::from_path(path)
                .with_context(|| format!("manifest path is not valid UTF-8: {}", path.display()))?;
            manifest::load(path)?
        }
        None => manifest::ImageManifest::default(),
    };

    anyhow::ensure!(
        args.rootfs.is_dir(),
        "rootfs path does not exist: {}",
        args.rootfs.display()
    );

    let mut ctx = transforms::Context::new(args.rootfs, manifest, args.check)?;
    transforms::run_all(&mut ctx)?;

    if args.check {
        tracing::info!("Dry run complete. No changes made.");
    } else {
        tracing::info!("Finalize complete.");
    }
    Ok(())
}

#[context("Linting rootfs")]
fn cmd_lint(args: LintArgs) -> Result<()> {
    anyhow::ensure!(
        args.rootfs.is_dir(),
        "rootfs path does not exist: {}",
        args.rootfs.display()
    );

    let manifest = manifest::ImageManifest::default();
    let mut ctx = transforms::Context::new(args.rootfs, manifest, true)?;
    transforms::run_all(&mut ctx)?;

    let (pass, fail) = ctx.results();
    tracing::info!("{pass} passed, {fail} failed");
    if fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[context("Building rootfs")]
fn cmd_build_rootfs(args: BuildRootfsArgs) -> Result<()> {
    let manifest_path = Utf8Path::from_path(&args.manifest).with_context(|| {
        format!(
            "manifest path is not valid UTF-8: {}",
            args.manifest.display()
        )
    })?;
    let mut manifest = manifest::load(manifest_path)?;

    // Override releasever from CLI
    if let Some(rv) = &args.releasever {
        manifest.image.releasever = rv.clone();
    }

    // Append extra packages
    manifest.packages.include.extend(args.extra_packages);

    anyhow::ensure!(
        !manifest.packages.include.is_empty(),
        "no packages specified in manifest or --install"
    );
    anyhow::ensure!(
        !manifest.image.releasever.is_empty(),
        "releasever must be set in manifest or via --releasever"
    );

    // Create target directory
    std::fs::create_dir_all(&args.target)
        .with_context(|| format!("creating target {}", args.target.display()))?;

    // Run dnf --installroot
    tracing::info!(
        "Installing {} packages into {}",
        manifest.packages.include.len(),
        args.target.display()
    );

    // Determine the target path (create first, then canonicalize)
    let target_abs = args
        .target
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", args.target.display()))?;

    let mut cmd = std::process::Command::new("dnf");
    cmd.arg("install")
        .arg("-y")
        .arg(format!("--installroot={}", target_abs.display()))
        .arg("--use-host-config")
        .arg(format!("--releasever={}", manifest.image.releasever))
        .arg("--setopt=install_weak_deps=False")
        .arg("--nodocs");

    for pkg in &manifest.packages.include {
        cmd.arg(pkg);
    }

    let status = cmd.status().context("running dnf install")?;
    anyhow::ensure!(status.success(), "dnf install failed with {status}");

    tracing::info!("Package installation complete");

    // Run finalize unless --no-finalize
    if !args.no_finalize {
        tracing::info!("Running finalize transforms");
        let mut ctx = transforms::Context::new(target_abs.clone(), manifest, false)?;
        transforms::run_all(&mut ctx)?;
        tracing::info!("Finalize complete");
    }

    Ok(())
}

#[context("Rechunking image")]
fn cmd_rechunk(args: RechunkArgs) -> Result<()> {
    tracing::info!("Rechunking {} -> {}", args.from, args.to);

    let status = std::process::Command::new("rpm-ostree")
        .args([
            "experimental",
            "compose",
            "build-chunked-oci",
            "--bootc",
            "--format-version=1",
            &args.from,
            &args.to,
        ])
        .status()
        .context("running rpm-ostree (is it installed?)")?;

    anyhow::ensure!(status.success(), "rpm-ostree rechunk failed with {status}");
    tracing::info!("Rechunk complete");
    Ok(())
}
