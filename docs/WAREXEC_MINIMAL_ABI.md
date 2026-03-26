# WarExec Minimal ABI

WarOS does not currently provide a Linux userspace ABI.
The kernel reuses selected x86_64 Linux syscall numbers for convenience only.
Today, WarExec is an experimental minimal ABI with four CI-proven static ELF paths:

- `/bin/warexec-smoke.elf`
  proves ELF load, stdout write, and exit
- `/bin/warexec-read-smoke.elf`
  proves ELF load, read-only open/read/close against a seeded WarFS file, stdout write, and exit
- `/bin/warexec-offset-smoke.elf`
  proves per-FD read-offset advancement and EOF on a seeded WarFS file, plus stdout write and exit
- `/bin/warexec-argv-smoke.elf`
  proves the current process-entry ABI: stack-based `argc`/`argv`, deterministic argument strings, and exit

## CI-Proven ABI Contract

The following behavior is part of the currently proven minimal ABI:

- ELF format
  static little-endian x86_64 ELF images with PT_LOAD segments
  no dynamic linker or interpreter support
- Process entry
  WarExec maps the ELF, provides a user stack, and enters ring 3 at the ELF entry point
  `%rsp` is 16-byte aligned at entry
  `*(u64*)rsp` is `argc`
  `((u64*)rsp)[1..argc]` are `argv` pointers to NUL-terminated user strings
  `argv[argc]` is `NULL`
  no envp array or auxv is exposed at entry yet
  general-purpose registers other than `%rsp` are not currently part of the ABI contract
- Standard file descriptors
  fd `1` and fd `2` support `write`
  fd `0` exists but no interactive stdin ABI is proven yet
- Exit
  `exit` returns a deterministic code to the kernel bootstrap path
- Read-only file path
  `open(path, 0, 0)` for an existing WarFS file
  each successful `open` creates an independent WarFS-backed descriptor with offset `0`
  `read(fd, buf, len)` copies bytes from the descriptor's current offset, advances it by the bytes read, and returns `0` at EOF
  `close(fd)` closes the descriptor

## Current Limitations

These limitations are intentional and should be treated as part of the ABI contract today:

- `open` is currently a narrow read-only path only
- `read` maintains only a narrow per-FD forward offset
- `lseek` is unsupported
- no shared-offset, dup-like, or pipe semantics are claimed
- process entry is currently stack-based only; no broad SysV or libc startup contract is claimed
- envp is omitted from the userspace entry ABI for now
- no broad libc compatibility is claimed
- no `fork` ABI is exposed
- `execve` exists only as an experimental in-place image replacement path; it reuses the same minimal stack ABI and currently passes an empty environment
- no dynamic linking, shared libraries, or interpreter handoff

## Syscall Surface Status

| Number | Syscall | Status | Notes |
| --- | --- | --- | --- |
| 0 | `read` | implemented and proven | WarFS file descriptors only; per-FD forward read offset, EOF returns 0 |
| 1 | `write` | implemented and proven | fd `1`/`2` only |
| 2 | `open` | implemented and proven | existing WarFS file only; `flags=0`, `mode=0` |
| 3 | `close` | implemented and proven | closes a descriptor |
| 60 | `exit` | implemented and proven | deterministic exit code |
| 4 | `stat` | implemented but experimental | basic metadata struct only |
| 9 / 11 / 12 | `mmap` / `munmap` / `brk` | implemented but experimental | narrow anonymous memory management only |
| 20 / 39 / 102 | `getpid` / `getppid` / `getuid` | implemented but experimental | basic identity queries |
| 59 / 61 | `execve` / `wait4` | implemented but experimental | not Linux-compatible semantics |
| 79 / 80 | `getcwd` / `chdir` | implemented but experimental | not CI-proven as ABI |
| 228 / 230 | `clock_gettime` / `nanosleep` | implemented but experimental | simple time support |
| 300-304 | `qalloc`..`qstate` | implemented but experimental | not part of the current minimal WarExec ABI |
| 420 / 421 | `sha3_256` / `random_bytes` | implemented but experimental | helper syscalls, not CI-proven |
| 8 | `seek` | scaffold / unsupported | returns `ENOSYS` |
| 57 | `fork` | scaffold / unsupported | returns `ENOSYS` |
| 200-211 | network syscalls | scaffold / unsupported | return `ENOSYS` |
| 305 / 310 / 311 / 320 | advanced quantum syscalls | scaffold / unsupported | return `ENOSYS` |
| 400-411 except `420/421` | PQC/signature syscalls | scaffold / unsupported | return `ENOSYS` |
| 500-501 | AI syscalls | scaffold / unsupported | return `ENOSYS` |
| 600-601 | ioctl / lsdev | scaffold / unsupported | return `ENOSYS` |

## Current Proof Files

- WarFS proof file: `/abi/waros-abi-proof.txt`
- WarFS proof file: `/abi/waros-offset-proof.txt`
- ELF proof binary: `/bin/warexec-smoke.elf`
- ELF proof binary: `/bin/warexec-read-smoke.elf`
- ELF proof binary: `/bin/warexec-offset-smoke.elf`
- ELF proof binary: `/bin/warexec-argv-smoke.elf`

This document is intentionally narrow.
If a behavior is not listed above as proven or implemented, WarOS should not claim it as current userspace ABI support.
