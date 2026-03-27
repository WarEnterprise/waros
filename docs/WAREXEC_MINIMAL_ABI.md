# WarExec Minimal ABI

WarOS does not currently provide a Linux userspace ABI.
The kernel reuses selected x86_64 Linux syscall numbers for convenience only.
Today, WarExec is an experimental minimal ABI with twelve CI-proven static ELF paths plus a narrow execution-hardening envelope from WarShield Pass 1:

- `/bin/warexec-smoke.elf`
  proves ELF load, stdout write, and exit
- `/bin/warexec-read-smoke.elf`
  proves ELF load, read-only open/read/close against a seeded WarFS file, stdout write, and exit
- `/bin/warexec-offset-smoke.elf`
  proves per-FD read-offset advancement and EOF on a seeded WarFS file, plus stdout write and exit
- `/bin/warexec-argv-smoke.elf`
  proves the current process-entry ABI: stack-based `argc`/`argv`, deterministic argument strings, and exit
- `/bin/warexec-exec-parent.elf` -> `/bin/warexec-exec-child.elf`
  proves one narrow userspace-triggered `execve` transition: in-place image replacement, reused stack-based `argc`/`argv`, deterministic child output, and exit
- `/bin/warexec-heap-smoke.elf`
  proves one narrow per-process heap-growth path: current-break query, monotonic growth, writable+NX heap memory, heap-backed stdout, and exit
- `/bin/warexec-fault-smoke.elf`
  proves one narrow pointer/error contract path: deterministic bad-pointer rejection on `write`, explicit negative error return, and exit
- `/bin/warexec-wait-smoke.elf` with `/bin/warexec-wait-child.elf`
  proves one narrow lifecycle path: a real child exits, `wait4(-1, status_ptr, 0)` observes one deterministic exit-only status word, reaps the child, and the parent exits
- `/bin/warexec-stat-smoke.elf`
  proves one narrow metadata path: `stat(path, out_ptr)` and `fstat(fd, out_ptr)` report regular-file type plus exact byte size for a seeded WarFS file
- `/bin/warexec-readdir-smoke.elf`
  proves one narrow directory path: `open(path, O_DIRECTORY, 0)` snapshots a deterministic directory view, `readdir(fd, out_ptr)` returns one entry per call in lexicographic order, and end-of-directory is explicit
- `/bin/warexec-path-smoke.elf`
  proves one narrow pathname contract: absolute path success for `stat` and `open`, deterministic rejection of relative paths, and exit
- `/bin/warexec-write-smoke.elf`
  proves one narrow create/write path: create-new open, userspace write into a staged regular-file descriptor, commit on `close`, `stat` size verification, read-back verification, and exit

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
- Minimal exec replacement
  `execve(path, argv, envp)` is a narrow in-place image replacement path only
  path resolution uses WarFS and the target must be a static little-endian x86_64 ELF
  a successful `execve` does not return to the caller; the current image is replaced and entered through the same stack-based `argc`/`argv` ABI
  `envp` is currently ignored and the replacement image receives no envp array at entry
  no fork, interpreter handoff, dynamic loader, or Linux-compatible `execve` semantics are claimed
- Minimal heap growth
  `brk(0)` queries the current heap break
  `brk(new_end)` grows the current process heap monotonically if `new_end` stays inside the reserved heap window
  newly mapped heap pages are writable, user-accessible, and NX
  shrinking is intentionally unsupported for now; requests below the current break return the current break unchanged
  no broad Linux `brk`/`sbrk`/`mmap` compatibility is claimed
- Minimal lifecycle / wait observation
  `exit(code)` stores one deterministic exit code for the current process
  `wait4(pid, status_ptr, 0)` currently supports only `pid = -1` (any exited direct child) or one direct child PID
  the call only observes already-exited children; no broad blocking or signal semantics are claimed
  on success, `wait4` returns the reaped child PID
  if `status_ptr` is non-NULL, the kernel writes one exit-only status word: `(exit_code & 0xFF) << 8`
  after a successful wait, the matching child is reaped and removed
  if no matching exited child exists, the call fails with `-10`
  nonzero `options`, `pid = 0`, and `pid < -1` are intentionally unsupported and fail with `-38`
  the current proof is kernel-orchestrated because WarExec still does not expose a general userspace spawn or fork ABI
- Pointer and error contract
  the current proven ABI validates userspace buffers and strings against the current process image, stack, and live heap before dereferencing them
  `write(fd, buf, len)` requires `buf..buf+len` to stay inside currently mapped user memory when `len > 0`
  `read(fd, buf, len)` requires `buf..buf+len` to stay inside currently mapped user memory when `len > 0`
  `open(path, 0, 0)` and `execve(path, argv, envp)` require `path` to be a NUL-terminated userspace string within a bounded maximum length
  `execve(path, argv, envp)` requires each non-NULL `argv[i]` to point to a bounded NUL-terminated userspace string
  `envp` is intentionally ignored today and is not dereferenced
  bad userspace ranges or unterminated bounded strings fail with `-14`
  invalid file descriptors fail with `-9`
  missing files fail with `-2`
  unsupported operations fail with `-38`
  this is a narrow experimental WarExec error subset only; it is not a claim of broad Linux errno compatibility
- Standard file descriptors
  fd `1` and fd `2` support `write`
  fd `0` exists but no interactive stdin ABI is proven yet
- Minimal file creation and write path
  `open(path, O_CREATE|O_WRITE, 0)` is the only currently proven writable file-open form
  the path must satisfy the same absolute-only pathname contract as the rest of WarExec
  the call is create-new only and fails if the target file already exists
  the parent directory must already exist
  the returned descriptor is a regular-file create/write handle only; it is not a broad POSIX writable FD
  `write(fd, buf, len)` on that descriptor copies from validated userspace memory into a per-FD staged buffer and advances the forward offset
  `write` either accepts the full buffer or fails with a deterministic negative error; no broad partial-write contract is claimed
  `close(fd)` is the commit point: on successful close, the staged buffer becomes the new WarFS file contents
  if the process exits without a successful close, WarOS does not currently guarantee that staged create/write data is persisted
  `read(fd, ...)` on a create/write descriptor remains unsupported
  `fstat(fd, ...)` on a live create/write descriptor reports the staged size, while `stat(path, ...)` and reopen/read after close observe the committed file
  this is a narrow WarExec create/write contract only; it is not POSIX `open`/`creat`/`write`/`truncate` compatibility
- Exit
  `exit` returns a deterministic code to the kernel bootstrap path
- Read-only file path
  `open(path, 0, 0)` for an existing WarFS file
  each successful `open` creates an independent WarFS-backed descriptor with offset `0`
  `read(fd, buf, len)` copies bytes from the descriptor's current offset, advances it by the bytes read, and returns `0` at EOF
  `close(fd)` closes the descriptor
- Minimal file metadata
  `stat(path, out_ptr)` reports metadata for one existing WarFS regular file path
  `fstat(fd, out_ptr)` reports the same metadata for a WarFS-backed descriptor returned by `open(path, 0, 0)`
  the writeback struct is `WarExecStat { size: u64, file_type: u8, readonly: u8, _reserved: [u8; 6] }`
  `file_type = 1` currently means regular file
  `size` is the exact current byte length of the file contents
  `readonly` reflects whether the current WarFS entry is system/read-only
  metadata writeback uses the same validated userspace pointer path as the rest of the proven ABI
  this is a narrow WarExec metadata contract only; it is not POSIX `struct stat` compatibility
- Minimal directory iteration
  `open(path, O_DIRECTORY, 0)` opens one existing WarFS directory and returns a directory descriptor
  the current ABI supports only `flags = 0` for regular files and `flags = O_DIRECTORY` for directories
  opening a directory snapshots the currently visible entries in lexicographic `name` order
  `readdir(fd, out_ptr)` writes exactly one `WarExecDirEntry` and advances the per-descriptor cursor
  `readdir` returns `1` when an entry is written, `0` at end-of-directory, and a negative error on failure
  the writeback struct is `WarExecDirEntry { file_type: u8, name_len: u8, _reserved: [u8; 6], name: [u8; 32] }`
  `file_type = 1` currently means regular file and `file_type = 2` currently means directory
  `name_len` is the byte length of the returned basename and `name` is NUL-padded scratch space for that basename
  `read(fd, ...)` on a directory descriptor remains unsupported
  this is a narrow WarExec directory contract only; it is not POSIX `opendir`/`readdir`/`getdents` compatibility
- Minimal pathname contract
  current proven path-taking syscalls are `open(path, ...)`, `stat(path, ...)`, and `execve(path, argv, envp)`
  only absolute paths are accepted
  empty strings are rejected
  relative paths are rejected
  repeated slashes are rejected rather than normalized
  `.` and `..` path components are rejected explicitly
  file-like paths reject a trailing slash
  directory-open paths may include one trailing slash, which is stripped deterministically before lookup
  no cwd-relative resolution, `openat`, symlink traversal, or broad POSIX normalization rules are claimed
- Heap growth
  each process has one heap base, current break, and heap limit tracked by WarExec
  the current heap proof grows by one page and writes a deterministic string into the newly mapped region

## Current Hardening Around the ABI

WarShield Pass 1 added execution hardening around the current WarExec path. These are implementation facts about the current kernel, not a claim of broad Linux compatibility:

- new WarExec stacks are randomized at load time through the current ASLR path
- heap and mmap base randomization also exist today as implementation details, but they should not yet be treated as stable ABI promises
- PT_LOAD segments are populated through temporary writable NX mappings and then tightened to their final permissions
- the loader rejects writable-and-executable user segments and verifies the final mapped segment set before entry
- user stack and heap mappings are NX in the current implementation
- capability enforcement currently applies to kernel-resident shell and system operations; there is not yet a general userspace capability syscall ABI

## Current Limitations

These limitations are intentional and should be treated as part of the ABI contract today:

- `open` currently supports only three narrow forms: read-only regular file, `O_DIRECTORY`, and create-new `O_CREATE|O_WRITE`
- `read` maintains only a narrow per-FD forward offset
- writable file descriptors are create-new only; no overwrite, append, truncate, rename, or unlink semantics are claimed
- `brk` is currently monotonic growth only
- shrinking the heap is unsupported
- no userspace signal or page-fault delivery is exposed for bad pointers; the current ABI returns a deterministic negative error instead
- `wait4` currently observes only already-exited direct children; it is not broad POSIX `waitpid` compatibility
- no general userspace spawn or fork ABI exists, so the current lifecycle proof is kernel-orchestrated
- metadata is currently regular-file-only; no `lstat`, symlink metadata, inode model, timestamps, or mode-bit compatibility are claimed
- directory iteration is currently read-only and snapshot-based; no directory mutation, `openat`, cwd-relative dir iteration contract, or broad POSIX dirent semantics are claimed
- pathname handling is currently absolute-only; no cwd semantics, relative resolution, `openat`, symlink handling, or broad POSIX normalization rules are claimed
- `lseek` is unsupported
- no shared-offset, dup-like, or pipe semantics are claimed
- process entry is currently stack-based only; no broad SysV or libc startup contract is claimed
- envp is omitted from the userspace entry ABI for now
- network/socket/HTTPS syscalls remain outside the minimal ABI and currently return `ENOSYS`
- capability checks are not exposed as a general userspace ABI today; current capability enforcement is on kernel shell and system operations
- no broad libc compatibility is claimed
- no `fork` ABI is exposed
- `execve` is still intentionally narrow: one in-place image replacement path only
- no dynamic linking, shared libraries, or interpreter handoff

## Syscall Surface Status

| Number | Syscall | Status | Notes |
| --- | --- | --- | --- |
| 0 | `read` | implemented and proven | WarFS file descriptors only; per-FD forward read offset, EOF returns 0, bad buffers return `-14` |
| 1 | `write` | implemented and proven | fd `1`/`2`, plus create-new WarFS regular-file descriptors opened with `O_CREATE\|O_WRITE`; bad buffers return `-14`, bad FDs return `-9` |
| 2 | `open` | implemented and proven | absolute path only; existing WarFS file with `flags=0`, WarFS directory with `flags=O_DIRECTORY`, or create-new regular file with `flags=O_CREATE\|O_WRITE`; bad path pointers return `-14`, invalid path forms return `-22` |
| 3 | `close` | implemented and proven | closes a descriptor |
| 4 | `stat` | implemented and proven | absolute path only; writes `WarExecStat` for one existing WarFS regular file path |
| 5 | `fstat` | implemented and proven | writes `WarExecStat` for one WarFS-backed file descriptor |
| 78 | `readdir` | implemented and proven | one `WarExecDirEntry` per call, lexicographic snapshot order, returns `1` or `0` at end-of-directory |
| 60 | `exit` | implemented and proven | deterministic exit code |
| 12 | `brk` | implemented and proven | narrow monotonic heap growth only; `brk(0)` queries current break, growth is bounded and NX |
| 9 / 11 | `mmap` / `munmap` | implemented but experimental | narrow anonymous memory management only |
| 20 / 39 / 102 | `getpid` / `getppid` / `getuid` | implemented but experimental | basic identity queries |
| 59 | `execve` | implemented and proven | absolute path only; narrow in-place image replacement only; reuses stack-based `argc`/`argv`, envp omitted, bad path/argv pointers return `-14` |
| 61 | `wait4` | implemented and proven | narrow exited-child observation only; returns child PID, writes `(exit_code & 0xFF) << 8`, reaps on success |
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
- ELF proof binary: `/bin/warexec-exec-parent.elf`
- ELF proof binary: `/bin/warexec-exec-child.elf`
- ELF proof binary: `/bin/warexec-heap-smoke.elf`
- ELF proof binary: `/bin/warexec-fault-smoke.elf`
- ELF proof binary: `/bin/warexec-wait-smoke.elf`
- ELF proof binary: `/bin/warexec-wait-child.elf`
- ELF proof binary: `/bin/warexec-stat-smoke.elf`
- WarFS proof directory: `/abi/readdir-proof`
- ELF proof binary: `/bin/warexec-readdir-smoke.elf`
- ELF proof binary: `/bin/warexec-path-smoke.elf`
- WarFS proof directory: `/abi/write-proof`
- ELF proof binary: `/bin/warexec-write-smoke.elf`

This document is intentionally narrow.
If a behavior is not listed above as proven or implemented, WarOS should not claim it as current userspace ABI support.
