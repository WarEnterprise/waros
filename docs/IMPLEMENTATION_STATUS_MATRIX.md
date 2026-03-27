# WarOS Implementation Status Matrix

Status terms used here:

- `IMPLEMENTED`: present, exercised, and part of current repository truth
- `INTEGRATED`: merged across subsystem boundaries with explicit scope limits
- `PARTIAL`: real code path, but still missing broader coverage or hardening
- `EXPERIMENTAL`: present but not release-grade
- `SCAFFOLDED`: intentional stub or placeholder surface
- `PLANNED ONLY`: blueprint direction, not a current implementation claim

| Subsystem | Status | Proof | Current limit / next step |
|---|---|---|---|
| Workspace quantum SDK | IMPLEMENTED | `crates/waros-quantum/*`; `cargo test --workspace` | Keep adding regression and performance coverage before widening claims |
| CLI | IMPLEMENTED | `crates/waros-cli/*`; `waros qstat`; QASM run flow | Add output snapshots and more IBM mock coverage |
| Python SDK | IMPLEMENTED | `crates/waros-python/*`; Python tests in CI | Keep wheel/import smoke strong across platforms |
| PQ crypto crate | IMPLEMENTED | `crates/waros-crypto/*`; crate tests | Add more negative and serialization tests |
| Kernel boot + shell | IMPLEMENTED | `kernel/src/main.rs`; `kernel/src/shell/*`; headless BIOS smoke in CI | Keep build/image/QEMU diagnostics reliable |
| Kernel memory + heap | IMPLEMENTED | `kernel/src/memory/*`; `kernel/src/memory/heap.rs` | Current heap is 8 MiB; keep docs aligned |
| WarFS core + disk persistence | PARTIAL | `kernel/src/fs/mod.rs`; `kernel/src/disk/*`; shell FS commands | Real WarFS exists, but it is not the blueprint filesystem and still needs broader persistence tests |
| WarExec minimal ABI | INTEGRATED | `kernel/src/exec/*`; `kernel/src/exec/smoke.rs`; `docs/WAREXEC_MINIMAL_ABI.md` | Keep the ABI narrow; do not imply general Linux compatibility |
| Kernel generic HAL / device registry | PARTIAL | `kernel/src/hal/*`; shell hardware commands | This is generic hardware plumbing, not a shipped QHAL |
| Kernel networking stack | PARTIAL | `kernel/src/net/*`; DHCP/DNS/TCP/HTTP paths; shell `net`, `curl`, `wget`, `ibm` | Real code path exists, but syscall networking is still stubbed and deterministic network smoke should expand |
| Kernel TLS / HTTPS | EXPERIMENTAL | `kernel/src/net/tls/mod.rs` | Traffic is encrypted, but certificate validation is not implemented |
| Kernel IBM Runtime path | EXPERIMENTAL | `kernel/src/net/ibm.rs`; shell `ibm` commands | Depends on the experimental kernel TLS path; userspace remains the primary supported route |
| WarShield Pass 1 hardening | INTEGRATED | `kernel/src/security/*`; `kernel/src/main.rs`; `kernel/src/net/tcp.rs` | Current scope is audit hooks, outbound TCP firewall decisions, ASLR load-path integration, W^X loader enforcement, and selected capability gates |
| WarShield Pass 2 package verification | INTEGRATED | `crates/waros-pkg/*`; `kernel/src/pkg/*`; kernel boot proof in `kernel/src/main.rs` | Signed-manifest verification is real, but the trust model is still one embedded bootstrap ML-DSA root without rotation or revocation |
| Audit coverage | PARTIAL | login/logout in `kernel/src/auth/login.rs` and `kernel/src/shell/commands.rs`; file mutation hooks in `kernel/src/fs/mod.rs` | Coverage is not full-system yet; treat it as current hook coverage, not complete provenance |
| Capability enforcement + transition model | INTEGRATED | `kernel/src/security/capabilities.rs`; `kernel/src/exec/*`; capability proof in `kernel/src/main.rs` | The current model covers shell-process creation, spawn, and `execve` replacement only; no broad userspace capability ABI or POSIX credential model exists |
| Package manager + signature path | INTEGRATED | `kernel/src/pkg/*`; `crates/waros-pkg/*`; `kernel/src/pkg/smoke.rs` | Packages install only after signed-manifest verification plus payload-digest checks; repository metadata, rotation, and revocation remain deferred |
| QKD / quantum security demos | EXPERIMENTAL | `kernel/src/security/crypt/qkd.rs`; shell `qkd bb84` | Simulated BB84 only; no real quantum link or QKD transport exists |
| Linux-numbered syscall surface | SCAFFOLDED | `kernel/src/exec/syscall.rs`; `kernel/src/exec/syscalls/*` | Numbers are reused for convenience only; many handlers still return `ENOSYS` |
| Kernel networking syscalls | SCAFFOLDED | `kernel/src/exec/syscalls/network.rs` | Socket/connect/send/recv/DNS/HTTPS syscalls are currently unsupported |
| Linux compatibility layer | SCAFFOLDED | `kernel/src/exec/compat.rs` | Keep public claims narrow until libc/process semantics are real |
| QRM | PLANNED ONLY | No dedicated module in source tree | Blueprint direction only |
| QAPS | PARTIAL | `kernel/src/exec/scheduler.rs` has a quantum priority / time slice | Not coherence-aware scheduling |
| UMA-Q | PLANNED ONLY | No unified quantum/classical memory subsystem in source tree | Blueprint direction only |
| QuantumIPC | PLANNED ONLY | No IPC subsystem beyond stubs | Blueprint direction only |
| QHAL | PLANNED ONLY | Shell status says `QHAL drivers: None loaded` | Generic HAL should not be confused with QHAL |
| QISA / WarQIR | PLANNED ONLY | No source modules; QASM exists instead | Keep QASM-first truth explicit |
| AI subsystem | PLANNED ONLY | `kernel/src/exec/syscalls/ai.rs` is stubbed | Blueprint direction only |
| QuantumNet | PLANNED ONLY | No quantum network stack; shell reports unavailable | Keep separate from the current classical network stack |
| Secure boot / PQ boot chain | PLANNED ONLY | No verifying boot chain in `kernel/` tooling | Do not market this until it exists |
