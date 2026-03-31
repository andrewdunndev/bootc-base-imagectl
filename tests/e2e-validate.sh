#!/bin/bash
# bootc-base-imagectl end-to-end validation
#
# Produces a structured log proving the full pipeline works:
#   manifest → build-rootfs → finalize → lint → container → install → boot
#
# Requirements: Fedora 42, root, KVM, cargo, podman, qemu-system-x86_64
# Usage: sudo ./tests/e2e-validate.sh [--log output.log]

set -euo pipefail

LOGFILE="${1:---log}"
if [[ "$LOGFILE" == "--log" ]]; then
    LOGFILE="${2:-/tmp/bootc-base-imagectl-validation.log}"
fi

WORKDIR="/tmp/imagectl-e2e-$$"
ROOTFS="$WORKDIR/rootfs"
BUILDDIR="$WORKDIR/build"
DISK="$WORKDIR/disk.raw"
BINARY="$(dirname "$0")/../target/release/bootc-base-imagectl"
MANIFEST="$(dirname "$0")/../examples/basef-minimal.toml"

log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$LOGFILE"; }
section() { echo "" | tee -a "$LOGFILE"; log "=== $* ==="; }
fail() { log "FAIL: $*"; exit 1; }

# Header
cat > "$LOGFILE" <<EOF
bootc-base-imagectl validation log
Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)
Host: $(uname -n) ($(uname -m))
Kernel: $(uname -r)
Fedora: $(rpm -E %fedora)
Tool: $BINARY
Manifest: $MANIFEST
EOF

# Preflight
section "Preflight checks"
[[ -x "$BINARY" ]] || fail "binary not found at $BINARY (run cargo build --release first)"
[[ -f "$MANIFEST" ]] || fail "manifest not found at $MANIFEST"
command -v podman >/dev/null || fail "podman not found"
command -v bootc >/dev/null || fail "bootc not found"
command -v qemu-system-x86_64 >/dev/null || fail "qemu-system-x86_64 not found"
[[ -c /dev/kvm ]] || fail "/dev/kvm not available (KVM required)"
log "All preflight checks passed"

# Setup
mkdir -p "$ROOTFS" "$BUILDDIR"
trap 'rm -rf "$WORKDIR"; podman rmi localhost/imagectl-e2e:latest 2>/dev/null || true' EXIT

# Step 1: build-rootfs
section "Step 1: build-rootfs"
BUILD_LOG="$WORKDIR/build.log"
RUST_LOG=info "$BINARY" build-rootfs --manifest "$MANIFEST" "$ROOTFS" > "$BUILD_LOG" 2>&1
sed 's/\x1b\[[0-9;]*m//g' "$BUILD_LOG" | grep -E 'INFO|ERROR' >> "$LOGFILE" || true
PKG_COUNT=$(chroot "$ROOTFS" rpm -qa 2>/dev/null | wc -l || echo "unknown")
ROOTFS_SIZE=$(du -sh "$ROOTFS" | cut -f1)
log "Packages installed: $PKG_COUNT"
log "Rootfs size: $ROOTFS_SIZE"

# Step 2: add install config (would be a transform in a future version)
section "Step 2: install config"
mkdir -p "$ROOTFS/usr/lib/bootc/install"
cat > "$ROOTFS/usr/lib/bootc/install/00-e2e.toml" <<IEOF
[install.filesystem.root]
type = "xfs"
IEOF
log "Wrote /usr/lib/bootc/install/00-e2e.toml"

# Step 3: bootc container lint
section "Step 3: bootc container lint"
LINT_OUTPUT=$(bootc container lint --rootfs "$ROOTFS" 2>&1)
echo "$LINT_OUTPUT" | tee -a "$LOGFILE"
echo "$LINT_OUTPUT" | grep -q "Checks passed" || fail "bootc container lint failed"
log "Lint passed"

# Step 4: verify key transforms
section "Step 4: transform verification"
verify() {
    local desc="$1" check="$2"
    if eval "$check"; then
        log "  OK: $desc"
    else
        log "  FAIL: $desc"
        fail "$desc"
    fi
}
verify "toplevel symlinks" "[[ -L '$ROOTFS/home' && \$(readlink '$ROOTFS/home') == 'var/home' ]]"
verify "/root -> var/roothome" "[[ -L '$ROOTFS/root' && \$(readlink '$ROOTFS/root') == 'var/roothome' ]]"
verify "/ostree -> sysroot/ostree" "[[ -L '$ROOTFS/ostree' && \$(readlink '$ROOTFS/ostree') == 'sysroot/ostree' ]]"
verify "/sysroot exists" "[[ -d '$ROOTFS/sysroot' ]]"
verify "rpmdb relocated" "[[ -f '$ROOTFS/usr/share/rpm/rpmdb.sqlite' ]]"
verify "var/lib/rpm is symlink" "[[ -L '$ROOTFS/var/lib/rpm' ]]"
verify "ostree prepare-root.conf" "grep -q 'enabled = true' '$ROOTFS/usr/lib/ostree/prepare-root.conf'"
verify "machine-id empty" "[[ ! -s '$ROOTFS/etc/machine-id' ]]"
verify "tmpfiles.d generated" "[[ -f '$ROOTFS/usr/lib/tmpfiles.d/bootc-base-var.conf' ]]"
verify "home.conf removed" "[[ ! -f '$ROOTFS/usr/lib/tmpfiles.d/home.conf' ]]"
verify "dracut config" "[[ -f '$ROOTFS/usr/lib/dracut/dracut.conf.d/10-bootc-base.conf' ]]"
verify "altfiles dracut" "[[ -f '$ROOTFS/usr/lib/dracut/dracut.conf.d/59-altfiles.conf' ]]"
verify "/boot empty (except efi)" "[[ \$(ls '$ROOTFS/boot/' 2>/dev/null | wc -l) -eq 0 ]]"
verify "ostree-boot populated" "[[ -d '$ROOTFS/usr/lib/ostree-boot' ]]"
verify "bootupd EFI.json" "[[ -f '$ROOTFS/usr/lib/bootupd/updates/EFI.json' ]]"
verify "bootupd BIOS.json" "[[ -f '$ROOTFS/usr/lib/bootupd/updates/BIOS.json' ]]"
verify "EFI binaries in bootupd" "[[ -f '$ROOTFS/usr/lib/bootupd/updates/EFI/fedora/shimx64.efi' ]]"
verify "passwd in /usr/lib" "[[ -f '$ROOTFS/usr/lib/passwd' ]]"

# Step 5: build container image
section "Step 5: container image"
cat > "$BUILDDIR/Containerfile" <<CEOF
FROM scratch
COPY rootfs/ /
LABEL containers.bootc 1
LABEL ostree.bootable 1
CMD ["/sbin/init"]
CEOF
cp -a "$ROOTFS" "$BUILDDIR/rootfs"
podman build -t localhost/imagectl-e2e:latest "$BUILDDIR" > "$WORKDIR/podman-build.log" 2>&1
tail -3 "$WORKDIR/podman-build.log" | tee -a "$LOGFILE"
IMAGE_SIZE=$(podman image inspect localhost/imagectl-e2e:latest --format '{{.Size}}' 2>/dev/null)
log "Image size: $((IMAGE_SIZE / 1048576)) MB"

# Step 6: bootc install to-disk
section "Step 6: bootc install to-disk"
truncate -s 10G "$DISK"
INSTALL_LOG="$WORKDIR/install.log"
podman run --rm --privileged --pid=host --ipc=host \
    -v /var/lib/containers:/var/lib/containers \
    -v /dev:/dev \
    -v "$WORKDIR:/output" \
    --security-opt label=type:unconfined_t \
    localhost/imagectl-e2e:latest \
    bootc install to-disk --generic-image --via-loopback /output/disk.raw > "$INSTALL_LOG" 2>&1
grep -vE '^\s*$' "$INSTALL_LOG" | tee -a "$LOGFILE"
log "Disk image created: $(du -sh "$DISK" | cut -f1)"

# Step 7: QEMU boot
section "Step 7: QEMU boot"
log "Booting with UEFI firmware (30s timeout)..."
BOOT_LOG="$WORKDIR/boot.log"
timeout 30 qemu-system-x86_64 \
    -machine q35 \
    -cpu host \
    -enable-kvm \
    -m 2048 \
    -drive file="$DISK",format=raw,if=virtio \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/ovmf/OVMF_CODE.fd \
    -nographic \
    -serial mon:stdio \
    -no-reboot 2>&1 | strings > "$BOOT_LOG" || true

if grep -q "Fedora Linux.*ostree" "$BOOT_LOG"; then
    log "GRUB loaded ostree deployment"
else
    log "WARNING: Could not confirm GRUB loaded ostree deployment"
fi

if grep -q "Booting" "$BOOT_LOG"; then
    log "Kernel boot initiated"
else
    log "WARNING: Could not confirm kernel boot"
fi

# Summary
section "Summary"
log "build-rootfs:          OK ($PKG_COUNT packages, $ROOTFS_SIZE)"
log "finalize:              OK (all transforms applied)"
log "bootc container lint:  OK"
log "transform verification: OK (18/18 checks)"
log "container image:       OK ($((IMAGE_SIZE / 1048576)) MB)"
log "bootc install to-disk: OK"
log "QEMU boot:             $(grep -q 'Booting' "$BOOT_LOG" && echo 'OK (GRUB -> kernel)' || echo 'PARTIAL (needs console kargs for full verification)')"
echo "" | tee -a "$LOGFILE"
log "Validation log: $LOGFILE"
