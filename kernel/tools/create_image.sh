#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
KERNEL_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$KERNEL_DIR/.." && pwd)"
TARGET_DIR="$KERNEL_DIR/target"
KERNEL_BIN="$TARGET_DIR/x86_64-unknown-none/release/waros-kernel"
IMAGE_BUILDER_MANIFEST="$REPO_ROOT/tools/kernel-image-builder/Cargo.toml"

cd "$KERNEL_DIR"

cargo +nightly build --release --target x86_64-unknown-none
cd "$REPO_ROOT"
cargo +nightly run --manifest-path "$IMAGE_BUILDER_MANIFEST" --release -- "$KERNEL_BIN" "$TARGET_DIR"
