#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
KERNEL_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$KERNEL_DIR/.." && pwd)"
TARGET_DIR="$KERNEL_DIR/target"
KERNEL_BIN="$TARGET_DIR/x86_64-unknown-none/release/waros-kernel"
IMAGE_BUILDER_DIR="$REPO_ROOT/tools/kernel-image-builder"
IMAGE_BUILDER_MANIFEST="$REPO_ROOT/tools/kernel-image-builder/Cargo.toml"
IMAGE_BUILDER_BIN="$IMAGE_BUILDER_DIR/target/release/waros-kernel-image-builder"

export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"

fail() {
    echo "create_image.sh: $*" >&2
    print_bootloader_diagnostics
    exit 1
}

require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "required command not found: $1"
    fi
}

require_nightly_component() {
    local component="$1"
    local install_name="$2"
    local installed
    installed="$(rustup component list --installed --toolchain nightly 2>/dev/null || true)"
    if ! printf '%s\n' "$installed" | grep -Eq "^${component}($|-)" ; then
        fail "nightly toolchain component '${component}' is required; install it with: rustup component add ${install_name} --toolchain nightly"
    fi
}

print_host_diagnostics() {
    echo "create_image.sh: host diagnostics"
    uname -a || true
    cargo +nightly -V
    rustc +nightly -Vv
    echo "create_image.sh: nightly components"
    rustup component list --installed --toolchain nightly || true
    echo "create_image.sh: kernel binary path = $KERNEL_BIN"
    echo "create_image.sh: image builder manifest = $IMAGE_BUILDER_MANIFEST"
}

print_bootloader_diagnostics() {
    local build_dir="$IMAGE_BUILDER_DIR/target/release/build"
    if [ ! -d "$build_dir" ]; then
        return
    fi

    while IFS= read -r stderr_file; do
        echo "--- bootloader build stderr: $stderr_file ---" >&2
        tail -n 200 "$stderr_file" >&2 || true
    done < <(find "$build_dir" -maxdepth 2 -path '*/bootloader-*/stderr' -type f 2>/dev/null)
}

build_kernel_if_needed() {
    if [ -f "$KERNEL_BIN" ]; then
        echo "create_image.sh: using existing kernel binary"
        return
    fi

    echo "create_image.sh: kernel binary missing, building waros-kernel"
    cd "$KERNEL_DIR"
    cargo +nightly build --release --target x86_64-unknown-none
}

build_image_builder() {
    cd "$REPO_ROOT"
    echo "create_image.sh: building host-side kernel image builder"
    if ! cargo +nightly build --manifest-path "$IMAGE_BUILDER_MANIFEST" --release -vv; then
        fail "failed to build tools/kernel-image-builder; confirm nightly rust-src and llvm-tools are installed"
    fi
}

resolve_image_builder_bin() {
    if [ -x "$IMAGE_BUILDER_BIN" ]; then
        printf '%s\n' "$IMAGE_BUILDER_BIN"
        return
    fi

    if [ -x "${IMAGE_BUILDER_BIN}.exe" ]; then
        printf '%s\n' "${IMAGE_BUILDER_BIN}.exe"
        return
    fi

    fail "image builder binary not found after build: $IMAGE_BUILDER_BIN"
}

require_cmd cargo
require_cmd rustup
require_nightly_component rust-src rust-src
require_nightly_component llvm-tools llvm-tools-preview
print_host_diagnostics
build_kernel_if_needed
mkdir -p "$TARGET_DIR"
build_image_builder

IMAGE_BUILDER_EXE="$(resolve_image_builder_bin)"
echo "create_image.sh: running $IMAGE_BUILDER_EXE"
"$IMAGE_BUILDER_EXE" "$KERNEL_BIN" "$TARGET_DIR"

if [ ! -f "$TARGET_DIR/waros.img" ] || [ ! -f "$TARGET_DIR/waros-bios.img" ]; then
    fail "image builder completed without producing both waros.img and waros-bios.img"
fi

ls -lh "$TARGET_DIR/waros.img" "$TARGET_DIR/waros-bios.img"
