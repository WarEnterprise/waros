#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
KERNEL_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE="$KERNEL_DIR/target/waros.img"
OVMF_PATH="${OVMF_PATH:-/usr/share/OVMF/OVMF_CODE.fd}"

"$SCRIPT_DIR/create_image.sh"

if ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
    echo "qemu-system-x86_64 was not found in PATH." >&2
    exit 1
fi

if [ ! -f "$OVMF_PATH" ]; then
    echo "OVMF firmware not found at '$OVMF_PATH'. Set OVMF_PATH to a valid OVMF image." >&2
    exit 1
fi

qemu-system-x86_64 \
    -bios "$OVMF_PATH" \
    -drive format=raw,file="$IMAGE" \
    -m 128M \
    -serial stdio \
    -display sdl \
    -no-reboot \
    -no-shutdown
