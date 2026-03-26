# WarOS Implementation Status Matrix
| Subsystem | Current state | Proof | Immediate next step |
|---|---|---|---|
| Workspace quantum SDK | IMPLEMENTED | `crates/waros-quantum/*`; `cargo test --workspace` passed | Keep expanding tests before adding new blueprint-scale claims |
| Quantum simulators | IMPLEMENTED | `src/simulator/{statevector,mps,density,trajectory}.rs`; tests `mps.rs`, `result_and_statistics.rs` | Add performance/regression baselines to CI |
| OpenQASM toolchain | IMPLEMENTED | `crates/waros-quantum/src/qasm/*`; tests `tests/qasm.rs` | Decide whether WarQIR is actually needed or keep QASM-first |
| IBM userspace backend | IMPLEMENTED | `crates/waros-quantum/src/backends/ibm/*`; CLI `ibm` commands | Add integration tests against mocked IBM API responses |
| PQ crypto crate | IMPLEMENTED | `crates/waros-crypto/src/{kem,sign,hash,qrng}.rs`; `tests/crypto.rs` | Add more negative/fuzz-style serialization tests |
| CLI | IMPLEMENTED | `crates/waros-cli/src/*`; verified `qstat` and `run examples/qasm/bell.qasm --shots 128` | Add snapshot tests for command output |
| Python SDK | IMPLEMENTED | `crates/waros-python/src/*`; `pytest` passed with 46 tests | Add wheel-build smoke test for import on clean env |
| Kernel boot | IMPLEMENTED | `kernel/src/main.rs`; kernel build succeeded; QEMU logs reach shell | Put boot verification into CI |
| Kernel shell | IMPLEMENTED | `kernel/src/shell/*`; QEMU logs show shell ready | Add serial-driven shell smoke harness |
| Interrupts/PIT | IMPLEMENTED | `kernel/src/arch/x86_64/{idt,interrupts,pit}.rs` | Add boot/runtime assertions in smoke tests |
| Memory allocator + heap | IMPLEMENTED | `kernel/src/memory/{physical,heap,paging}.rs`; boot logs show init | Fix README heap-size drift and add allocator tests where practical |
| WarFS core | PARTIALLY IMPLEMENTED | `kernel/src/fs/mod.rs`; shell FS commands; boot logs show format/load | Add reboot persistence tests and stronger invariants |
| Disk persistence | PARTIALLY IMPLEMENTED | `kernel/src/disk/*`; boot logs show virtio-blk format/load | Add automated first-boot/second-boot validation |
| Kernel device registry/HAL | PARTIALLY IMPLEMENTED | `kernel/src/hal/*`; shell can report devices | Clarify whether this is generic HAL or proto-QHAL |
| Kernel networking stack | PARTIALLY IMPLEMENTED | `kernel/src/net/*`; boot logs show DHCP/DNS/NIC detection | Add automated networking smoke tests |
| Kernel TLS/HTTPS | PARTIALLY IMPLEMENTED | `kernel/src/net/tls/mod.rs`; explicit warning says no cert validation | Implement verification or disable by default |
| Kernel IBM Runtime path | PARTIALLY IMPLEMENTED | `kernel/src/net/ibm.rs`; shell `ibm` commands | Decide whether kernel-side IBM support is intentional and testable |
| Kernel quantum simulator | IMPLEMENTED | `kernel/src/quantum/*`; shell quantum commands; boot log says subsystem ready | Cross-validate outputs with `waros-quantum` fixtures |
| WarExec / ELF loading | PARTIALLY IMPLEMENTED | `kernel/src/exec/{loader,elf,syscall}.rs` | Add one proven end-to-end user ELF execution path |
| Linux compatibility layer | SCAFFOLDED / STUBBED | `kernel/src/exec/compat.rs`; many `kernel/src/exec/syscalls/*` return `ENOSYS` | Narrow claims and implement only a tested subset |
| Syscall surface | SCAFFOLDED / STUBBED | `kernel/src/exec/syscall.rs` plus many `ENOSYS` handlers | Gate unsupported calls and document status explicitly |
| Package manager | PARTIALLY IMPLEMENTED | `kernel/src/pkg/*`; built-in bootstrap packages are seeded into WarFS | Replace placeholder signing and add remote/index tests |
| Secure boot / PQ boot chain | PLANNED ONLY | No verifying boot stage in `kernel/` tooling | Do not market this until a real chain exists |
| QRM | PLANNED ONLY | No dedicated module; only simple quantum register allocation | Define a narrow first milestone or drop the term from near-term docs |
| QAPS | PARTIALLY IMPLEMENTED | Generic scheduler has `Quantum` priority/time slice | Add or remove coherence-aware scheduling claims |
| UMA-Q | PLANNED ONLY | No unified classical/quantum memory subsystem in source tree | Keep out of implementation claims until designed concretely |
| QuantumIPC | PLANNED ONLY | No IPC subsystem; `PipeHandle` is only a stub | Start with ordinary IPC before quantum-specific variants |
| QHAL | PLANNED ONLY | Shell status literally says `QHAL drivers:   None loaded` | Define explicit quantum-driver interfaces before adding vendors |
| QISA / WarQIR | PLANNED ONLY | No source modules; only QASM exists today | Decide whether to standardize on QASM first |
| AI subsystem | PLANNED ONLY | `kernel/src/exec/syscalls/ai.rs` returns `ENOSYS` | Remove implied AI maturity from near-term architecture claims |
| QuantumNet | PLANNED ONLY | Kernel quantum status says `Quantum net:    Not available`; `sys_qkd_bb84` is `ENOSYS` | Keep separate from current classical networking work |
