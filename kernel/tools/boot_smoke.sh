#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KERNEL_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
TARGET_DIR="$KERNEL_DIR/target"
IMAGE_PATH="${BOOT_SMOKE_IMAGE_PATH:-$TARGET_DIR/waros-bios.img}"
LOG_PATH="${BOOT_SMOKE_LOG_PATH:-$TARGET_DIR/qemu-boot-smoke.log}"
QEMU_BIN="${QEMU_BIN:-qemu-system-x86_64}"
TIMEOUT_SECS="${BOOT_SMOKE_TIMEOUT_SECS:-90}"

BOOT_MARKER="WarOS: entering kernel bootstrap"
FS_MARKER="[OK] WarFS: filesystem core ready"
EXEC_STDOUT_MARKER="warexec smoke user program"
EXEC_MARKER="[OK] WarExec smoke: /bin/warexec-smoke.elf exited with code 42"
SHELL_MARKER="[INFO] WarOS shell online. Type 'help' for available commands."

QEMU_PID=""

cleanup() {
    if [ -n "${QEMU_PID}" ] && kill -0 "${QEMU_PID}" 2>/dev/null; then
        kill "${QEMU_PID}" 2>/dev/null || true
        sleep 1
        if kill -0 "${QEMU_PID}" 2>/dev/null; then
            kill -9 "${QEMU_PID}" 2>/dev/null || true
        fi
        wait "${QEMU_PID}" 2>/dev/null || true
    fi
}

fail() {
    echo "Kernel boot smoke FAILED: $1" >&2
    if [ -f "${LOG_PATH}" ]; then
        echo "--- serial log tail (${LOG_PATH}) ---" >&2
        tail -n 200 "${LOG_PATH}" >&2 || true
    fi
    exit 1
}

trap cleanup EXIT INT TERM

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
    echo "QEMU not found: ${QEMU_BIN}" >&2
    exit 1
fi

mkdir -p "${TARGET_DIR}"

if [ ! -f "${IMAGE_PATH}" ]; then
    echo "Boot smoke: kernel image missing, creating it with kernel/tools/create_image.sh"
    "${SCRIPT_DIR}/create_image.sh"
fi

if [ ! -s "${IMAGE_PATH}" ]; then
    fail "kernel image missing or empty: ${IMAGE_PATH}"
fi

echo "Boot smoke: using image ${IMAGE_PATH}"
ls -lh "${IMAGE_PATH}" || true

: > "${LOG_PATH}"

# The assertions intentionally use stable serial markers that already exist in the kernel:
# 1. bootstrap entry
# 2. WarFS core initialization
# 3. userspace stdout observed on serial
# 4. minimal WarExec user-ELF exit marker
# 5. shell-ready banner
#
# This keeps CI deterministic without introducing timing-sensitive interaction or GUI automation.
"${QEMU_BIN}" \
    -drive format=raw,file="${IMAGE_PATH}" \
    -m 512M \
    -serial "file:${LOG_PATH}" \
    -display none \
    -monitor none \
    -no-reboot \
    -no-shutdown \
    >/dev/null 2>&1 &
QEMU_PID=$!

deadline=$(( $(date +%s) + TIMEOUT_SECS ))

while [ "$(date +%s)" -lt "${deadline}" ]; do
    if grep -Fq "${BOOT_MARKER}" "${LOG_PATH}" \
        && grep -Fq "${FS_MARKER}" "${LOG_PATH}" \
        && grep -Fq "${EXEC_STDOUT_MARKER}" "${LOG_PATH}" \
        && grep -Fq "${EXEC_MARKER}" "${LOG_PATH}" \
        && grep -Fq "${SHELL_MARKER}" "${LOG_PATH}"; then
        echo "Kernel boot smoke passed."
        echo "  Log: ${LOG_PATH}"
        echo "  Found: ${BOOT_MARKER}"
        echo "  Found: ${FS_MARKER}"
        echo "  Found: ${EXEC_STDOUT_MARKER}"
        echo "  Found: ${EXEC_MARKER}"
        echo "  Found: ${SHELL_MARKER}"
        exit 0
    fi

    if ! kill -0 "${QEMU_PID}" 2>/dev/null; then
        wait "${QEMU_PID}" 2>/dev/null || true
        fail "QEMU exited before all expected boot markers were observed"
    fi

    sleep 1
done

fail "timed out after ${TIMEOUT_SECS}s waiting for shell-ready boot markers"
