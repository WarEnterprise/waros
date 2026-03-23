#!/bin/sh

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KERNEL_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
IMAGE_PATH="$KERNEL_DIR/target/waros-bios.img"
QEMU_BIN="${QEMU_BIN:-qemu-system-x86_64}"

if ! command -v "$QEMU_BIN" >/dev/null 2>&1; then
    echo "QEMU not found: $QEMU_BIN" >&2
    exit 1
fi

if [ ! -f "$IMAGE_PATH" ]; then
    echo "Kernel image not found at '$IMAGE_PATH'. Run ./create_image.sh first." >&2
    exit 1
fi

echo "Node A:"
echo "$QEMU_BIN -drive format=raw,file=\"$IMAGE_PATH\" -m 128M -serial stdio -serial tcp:127.0.0.1:4444,server,nowait -no-reboot -no-shutdown"
echo
echo "Node B:"
echo "$QEMU_BIN -drive format=raw,file=\"$IMAGE_PATH\" -m 128M -serial stdio -serial tcp:127.0.0.1:4444 -no-reboot -no-shutdown"
