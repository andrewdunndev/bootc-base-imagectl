# bootc-base-imagectl

Transforms a `dnf --installroot` rootfs into a bootc-compatible layout.

## Why this exists

Building bootc base images today requires either `rpm-ostree compose` (the
Fedora official path) or encoding 80+ lines of filesystem transforms as
shell in a Containerfile (the FROM-scratch path). Both work, but the shell
approach is fragile and the rpm-ostree path couples you to a build system
that the ecosystem is moving away from.

This tool extracts the transform logic into a typed, tested Rust pipeline.
Each transform is a named function with a check/apply pattern: it detects
whether it has already been applied and skips if so. The result is
idempotent, auditable, and produces rootfs layouts that pass
`bootc container lint`.

bootc itself is intentionally distro-agnostic. Distro-specific build
logic belongs in external tooling. This follows the same pattern as
[system-reinstall-bootc](https://github.com/bootc-dev/ci-sandbox/tree/main/crates/system-reinstall-bootc),
and is designed to eventually live under the bootc-dev org.

## Usage

### finalize: transform an existing rootfs

```bash
# After dnf --installroot populates the rootfs:
bootc-base-imagectl finalize --rootfs /path/to/rootfs

# With a manifest for transform configuration:
bootc-base-imagectl finalize --rootfs /path/to/rootfs --manifest config.toml

# Dry-run (check what needs to be done without modifying):
bootc-base-imagectl finalize --rootfs /path/to/rootfs --check
```

### build-rootfs: install packages and finalize in one step

```bash
bootc-base-imagectl build-rootfs --manifest examples/basef-minimal.toml /path/to/rootfs
```

### lint: check a rootfs without modifying it

```bash
bootc-base-imagectl lint --rootfs /path/to/rootfs
```

## Transforms

Transforms run in three phases. Within each phase, they execute in the
order listed.

**PreChroot** (filesystem layout, no chroot needed):

| Transform | What it does |
|-----------|-------------|
| `toplevel_symlinks` | Convert /home, /root, /opt, /srv, /mnt, /media, /usr/local to ostree-convention symlinks. Create /sysroot and /ostree. |
| `var_tmpfiles` | Scan /var, generate tmpfiles.d entries for first-boot directory creation. Remove conflicting home.conf. Fix provision.conf paths. |
| `rpmdb_relocate` | Move rpmdb from /var/lib/rpm or /usr/lib/sysimage/rpm to /usr/share/rpm (immutable /usr). Handles both dnf4 and dnf5. |
| `passwd_generate` | Copy /etc/passwd and /etc/group to /usr/lib/ for nss-altfiles in initramfs. |
| `ostree_config` | Write prepare-root.conf (composefs enabled, sysroot readonly) and kernel install.conf (layout=ostree). |
| `dracut_config` | Write dracut configs: hostonly=no, bootc modules, altfiles install items. TPM2 module conditional on rootfs support. |
| `dnf_config` | Configure package manager to not accumulate kernels. Handles both dnf4 (/etc/dnf/dnf.conf) and dnf5 drop-in directories. |
| `systemd_config` | Enable tmp.mount, write systemd presets from manifest, empty /etc/machine-id for first-boot generation. |

**Chroot** (operations requiring bind-mounted /proc, /sys, /dev):

| Transform | What it does |
|-----------|-------------|
| `chroot_ops` | Run depmod, generate initramfs via dracut, apply systemd presets, generate bootupd metadata (BIOS + EFI). |

**PostChroot** (cleanup after chroot operations):

| Transform | What it does |
|-----------|-------------|
| `boot_relocate` | Move /boot contents to /usr/lib/ostree-boot/ (bootc convention). |
| `post_cleanup` | Clear /var/cache, /var/tmp, /var/log, /run, /tmp. Remove stale rpmdb WAL/SHM files. |

## Manifest format

TOML. All fields optional; sensible defaults apply.

```toml
[image]
name = "my-image"
releasever = "42"

[packages]
include = ["systemd", "kernel-core", "bootc", "..."]

[transforms]
skip = ["dnf_config"]  # skip specific transforms by name

[transforms.dracut]
extra_modules = ["virtiofs", "tpm2-tss"]

[transforms.systemd]
enable_units = ["systemd-networkd", "sshd"]
disable_timers = ["dnf-makecache.timer"]
```

The tool also reads rpm-ostree treefile YAML (`.yaml`/`.yml`) for package
lists, providing compatibility with existing Fedora bootc manifests.

## How it compares to rpm-ostree compose

| | rpm-ostree compose | bootc-base-imagectl |
|---|---|---|
| Package installation | rpm-ostree (internal resolver) | dnf --installroot (standard dnf) |
| Transforms | Built into compose pipeline | Explicit, named, auditable |
| Idempotent | No (always rebuilds) | Yes (check before apply) |
| Lint/check mode | No | Yes (--check flag) |
| Treefile support | Native format | Read-only import for package lists |
| Dependencies | rpm-ostree, libostree | dnf, chroot, standard coreutils |

## Building

```bash
cargo build --release
```

Requires Rust 1.85+ (edition 2024, matching bootc MSRV).

## Testing

```bash
cargo test                            # unit + integration tests
cargo clippy -- -D warnings           # lint
```

Integration tests create temporary rootfs directories and run transforms
against them. No root privileges or container runtime needed.

## Full pipeline example

Build a bootable Fedora 42 disk image from a TOML manifest:

```bash
# 1. Build rootfs from manifest
sudo bootc-base-imagectl build-rootfs \
  --manifest examples/basef-minimal.toml \
  /tmp/my-rootfs

# 2. Add bootc install config
sudo mkdir -p /tmp/my-rootfs/usr/lib/bootc/install
echo '[install.filesystem.root]
type = "xfs"' | sudo tee /tmp/my-rootfs/usr/lib/bootc/install/00-my-image.toml

# 3. Validate
sudo bootc container lint --rootfs /tmp/my-rootfs

# 4. Build container image
cat > Containerfile <<'EOF'
FROM scratch
COPY my-rootfs/ /
LABEL containers.bootc 1
LABEL ostree.bootable 1
CMD ["/sbin/init"]
EOF
sudo podman build -t localhost/my-image:latest .

# 5. Install to disk
truncate -s 10G disk.raw
sudo podman run --rm --privileged --pid=host --ipc=host \
  -v /var/lib/containers:/var/lib/containers \
  -v /dev:/dev -v .:/output \
  --security-opt label=type:unconfined_t \
  localhost/my-image:latest \
  bootc install to-disk --generic-image --via-loopback /output/disk.raw
```

## License

MIT OR Apache-2.0
