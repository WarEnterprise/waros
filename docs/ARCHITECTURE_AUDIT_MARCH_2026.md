# WarOS Architecture Audit
## 1. Executive Summary
WarOS today is two materially different things in one repository: a solid classical-hosted quantum/PQC software stack in the top-level Cargo workspace, and a bootable x86_64 `no_std` kernel prototype in `kernel/` that can boot under QEMU, mount a custom WarFS volume, run a shell, manage memory, poll limited devices, and expose an in-kernel quantum simulator. It is not yet the broad hybrid quantum-classical operating system described in `BLUEPRINT.md`; most of that architecture is still absent, partially sketched, or represented by placeholder surfaces rather than completed subsystems.

WarOS fits best today as a `quantum SDK platform plus bootable research kernel prototype`, not as a mature operating system, not as a Linux-compatible OS, and not as a real quantum-hardware operating environment.

## 2. Workspace and Repository Map
### Top-level workspace
- `Cargo.toml`
  Purpose: Rust workspace for the userspace/tooling crates only.
  Key fact: `kernel/` is intentionally outside the workspace and builds separately.

### `kernel/`
- Purpose: standalone `waros-kernel` crate for a bootable x86_64 `no_std` prototype.
- Key entry points:
  - `kernel/src/main.rs`: boot sequence, subsystem init, login loop, shell handoff.
  - `kernel/src/boot/mod.rs`: extracts bootloader framebuffer and memory map.
  - `kernel/src/arch/x86_64/*`: GDT, IDT, PIC, PIT, ports, FPU.
  - `kernel/src/shell/mod.rs` and `kernel/src/shell/commands.rs`: interactive shell and command surface.
  - `kernel/src/fs/mod.rs`: in-memory WarFS plus shell-facing FS operations.
  - `kernel/src/disk/*`: virtio-blk persistence for WarFS.
  - `kernel/src/net/*`: PCI scan, virtio-net/e1000 paths, DHCP/DNS/TCP/HTTP/TLS, IBM Runtime client.
  - `kernel/src/quantum/*`: in-kernel state-vector simulator and shell commands.
  - `kernel/src/exec/*`: process table, ELF loader, syscall entry, partial Linux-style syscall numbering.
- Key commands:
  - `cargo +nightly build --release --target x86_64-unknown-none`
  - `kernel/tools/create_image.ps1`
  - `kernel/tools/run_qemu.ps1`
- Key artifacts/evidence:
  - `kernel/target/waros.img`
  - `kernel/target/waros-bios.img`
  - `kernel/target/qemu-disk-firstboot.log`
  - `kernel/target/qemu-disk-smoke.log`
  - `kernel/target/qemu-https-smoke.log`

### `crates/waros-quantum`
- Purpose: primary userspace quantum SDK.
- Key entry points:
  - `crates/waros-quantum/src/lib.rs`
  - `crates/waros-quantum/src/simulator/*`
  - `crates/waros-quantum/src/algorithms/*`
  - `crates/waros-quantum/src/qasm/*`
  - `crates/waros-quantum/src/backends/ibm/*`
- Key tests/benchmarks/examples:
  - Tests: `tests/algorithms.rs`, `tests/qasm.rs`, `tests/noise_model.rs`, `tests/qec.rs`, `tests/mps.rs`, `tests/gate_behavior.rs`, `tests/gate_unitarity.rs`
  - Bench: `benches/statevector.rs`
  - Examples: `examples/bell_state.rs`, `noise_simulation.rs`, `shor_demo.rs`, `vqe_demo.rs`, `qaoa_demo.rs`, `ibm_real_hardware.rs`
- Verified commands:
  - `cargo test --workspace`
  - `cargo run -p waros-cli -- run examples/qasm/bell.qasm --shots 128`

### `crates/waros-crypto`
- Purpose: post-quantum cryptography and hashing wrapper crate.
- Key entry points:
  - `crates/waros-crypto/src/kem.rs`
  - `crates/waros-crypto/src/sign.rs`
  - `crates/waros-crypto/src/hash.rs`
  - `crates/waros-crypto/src/qrng.rs`
- Key tests/examples:
  - `crates/waros-crypto/tests/crypto.rs`
  - `crates/waros-crypto/examples/pqc_demo.rs`

### `crates/waros-cli`
- Purpose: userspace CLI for simulation, QASM execution, REPL, status display, and IBM Runtime operations.
- Key entry points:
  - `crates/waros-cli/src/main.rs`
  - `crates/waros-cli/src/commands/run.rs`
  - `crates/waros-cli/src/commands/repl.rs`
  - `crates/waros-cli/src/commands/qstat.rs`
  - `crates/waros-cli/src/commands/ibm.rs`
- Verified commands:
  - `cargo run -p waros-cli -- qstat`
  - `cargo run -p waros-cli -- run examples/qasm/bell.qasm --shots 128`

### `crates/waros-python`
- Purpose: PyO3/maturin Python bindings over the quantum and crypto crates.
- Key entry points:
  - `crates/waros-python/src/lib.rs`
  - `crates/waros-python/src/{circuit,simulator,noise,qasm,crypto,algorithms,ibm}.rs`
  - `crates/waros-python/waros/compat.py`
  - `crates/waros-python/pyproject.toml`
- Key tests:
  - `crates/waros-python/tests/test_waros.py`
- Verified command:
  - `.\crates\waros-python\.venv\Scripts\python.exe -m pytest crates/waros-python/tests/test_waros.py -q`

### Tooling / build scripts
- `tools/kernel-image-builder`
  Purpose: creates BIOS and UEFI disk images from the kernel ELF using `bootloader`.
- `kernel/tools/create_image.ps1`, `kernel/tools/run_qemu.ps1`, `kernel/tools/run_qemu_pair.ps1`
  Purpose: convenience wrappers for kernel build/image/QEMU execution.
- `.github/workflows/ci.yml`
  Purpose: workspace build/test/clippy/doc, kernel build, Python wheel/test pipeline.
  Important limitation: CI builds the kernel but does not boot it headlessly and assert a successful shell prompt.

### Docs / blueprint / README
- `README.md`
  Purpose: current high-level project description and quick-start surface.
- `BLUEPRINT.md`
  Purpose: large forward-looking architecture design.
  Important limitation: much broader than repository reality.

## 3. What Is Actually Implemented
| Area | Status | Evidence | Notes |
|---|---|---|---|
| Boot flow | IMPLEMENTED | `kernel/src/main.rs`, `kernel/src/boot/mod.rs`, `kernel/tools/create_image.ps1`, `tools/kernel-image-builder/src/main.rs`, successful `cargo +nightly build --release --target x86_64-unknown-none`, QEMU serial logs in `kernel/target/qemu-disk-firstboot.log` and `kernel/target/qemu-disk-smoke.log` | This is a real bootable prototype under QEMU. It is not a secure boot chain. |
| Kernel console/shell | IMPLEMENTED | `kernel/src/display/console.rs`, `kernel/src/shell/mod.rs`, `kernel/src/shell/commands.rs`, boot logs reach `WarOS shell online` | Real shell with commands for FS, tasks, networking, quantum, auth, and package management. |
| Interrupts/timers | IMPLEMENTED | `kernel/src/arch/x86_64/idt.rs`, `kernel/src/arch/x86_64/interrupts.rs`, `kernel/src/arch/x86_64/pit.rs` | Timer IRQ increments ticks and calls `exec::tick`; keyboard IRQ reads PS/2 scancodes. |
| Memory/heap/frame allocation | IMPLEMENTED | `kernel/src/memory/physical.rs`, `kernel/src/memory/heap.rs`, `kernel/src/memory/paging.rs`, boot log reports physical memory and heap init | Bitmap frame allocator and 8 MiB heap are real. README still says 4 MiB, which is stale. |
| Tasking/scheduler | PARTIALLY IMPLEMENTED | `kernel/src/task/mod.rs`, `kernel/src/exec/scheduler.rs`, `kernel/src/exec/mod.rs`, shell commands `spawn`, `jobs`, `wait`, `kill`, `nice` | Cooperative shell tasks are real. Process scheduling and time slices exist, but end-to-end userspace execution proof is thin and many blocked states/features are unused. |
| Filesystem | PARTIALLY IMPLEMENTED | `kernel/src/fs/mod.rs`, `kernel/src/disk/mod.rs`, shell commands `ls/cat/write/rm/touch/stat/df`, QEMU logs show WarFS format/load | Real custom WarFS exists in RAM and can persist to virtio-blk. This is not the blueprint WarFS with quantum object types or signed metadata. |
| Device drivers | PARTIALLY IMPLEMENTED | Serial: `kernel/src/drivers/serial.rs`; keyboard: `kernel/src/drivers/keyboard.rs`; framebuffer: `kernel/src/display/framebuffer.rs`; disk: `kernel/src/disk/virtio_blk.rs`; NICs: `kernel/src/net/virtio/*`, `kernel/src/hal/net/e1000.rs`; USB/xHCI code under `kernel/src/hal/usb/*` | Several concrete drivers/probes exist, but device coverage is narrow and validation is limited. |
| Networking | PARTIALLY IMPLEMENTED | `kernel/src/net/mod.rs`, `kernel/src/net/http.rs`, `kernel/src/net/tls/mod.rs`, `kernel/src/net/ibm.rs`, QEMU logs show PCI scan, NIC, DHCP, DNS | Kernel networking is real enough for boot-time DHCP/DNS and HTTP/TLS code paths. It is not a general-purpose socket API; syscall networking is still `ENOSYS`. TLS explicitly lacks certificate validation. |
| Quantum simulation | IMPLEMENTED | Userspace: `crates/waros-quantum/src/simulator/*`; kernel: `kernel/src/quantum/*`; tests pass under `cargo test --workspace`; shell help lists `qalloc/qrun/qstate/qmeasure/...` | Strongest core capability in the repo. Entirely classical simulation today. |
| Quantum algorithms | IMPLEMENTED | `crates/waros-quantum/src/algorithms/*`, `crates/waros-quantum/tests/algorithms.rs`, examples `shor_demo.rs`, `vqe_demo.rs`, `qaoa_demo.rs` | Real algorithm implementations and tests exist in the userspace SDK. |
| Crypto / PQC | IMPLEMENTED | `crates/waros-crypto/src/{kem,sign,hash,qrng}.rs`, `crates/waros-crypto/tests/crypto.rs` | Real userspace PQC wrappers and hashing exist. Kernel package-signature flow is still placeholder-grade. |
| CLI tools | IMPLEMENTED | `crates/waros-cli/src/main.rs`, `crates/waros-cli/src/commands/*`, verified `qstat` and QASM run commands | CLI is a working front-end for the userspace SDK and IBM backend. |
| Python SDK | IMPLEMENTED | `crates/waros-python/src/*`, `crates/waros-python/waros/compat.py`, `crates/waros-python/tests/test_waros.py`, verified `pytest` with 46 passing tests | Good packaging surface via PyO3 + maturin. |
| Build/release tooling | IMPLEMENTED | `.github/workflows/ci.yml`, `tools/kernel-image-builder`, `kernel/tools/*.ps1`, `crates/waros-python/pyproject.toml` | Rust workspace CI and Python wheel build exist. Kernel boot smoke is not automated in CI. |
| Documentation | PARTIALLY IMPLEMENTED | `README.md`, `BLUEPRINT.md`, this audit | Docs are extensive, but some statements drift from code: README still says 4 MiB heap and says the kernel does not perform HTTPS, while kernel HTTPS/IBM code exists. |
| Linux compatibility | SCAFFOLDED / STUBBED | `kernel/src/exec/compat.rs` returns identity mapping; `kernel/src/exec/syscall.rs` exposes Linux-like numbers; many handlers in `kernel/src/exec/syscalls/*` return `ENOSYS` | There is a partial ELF/syscall/process skeleton, but not a usable Linux compatibility layer. |
| Security hardening | PARTIALLY IMPLEMENTED | Auth/session/users modules in `kernel/src/auth/*`; PQC crates in userspace; `kernel/src/net/tls/mod.rs` warns about no certificate validation; `kernel/src/pkg/signature.rs` says embedded root key is placeholder | Security is present as theme and API surface, not as hardened end-to-end implementation. |

## 4. Blueprint vs Reality
| Subsystem | Classification | Evidence | Reality check |
|---|---|---|---|
| QRM | partial | `kernel/src/quantum/mod.rs` allocates per-process quantum register handles | There is simple quantum register allocation, but no dedicated QRM module, no entanglement graph, no coherence manager, no hardware resource arbitration. |
| QAPS | partial | `kernel/src/exec/scheduler.rs` defines priorities including `Quantum` and a `QUANTUM_TIME_SLICE` | Scheduler is generic and timer-driven, not coherence/deadline aware. |
| UMA-Q | missing | No `uma-q` subsystem or equivalent module; only classical address-space code in `kernel/src/exec/address_space.rs` and `syscalls/memory.rs` | No unified classical/quantum memory model or no-cloning enforcement. |
| WarFS | partial | `kernel/src/fs/mod.rs`, `kernel/src/disk/*`, shell FS commands | Real custom FS exists, but none of the blueprint quantum object types, signed metadata model, or serverized layout exists. |
| QuantumIPC | missing | No `ipc` subsystem; `kernel/src/exec/pipe.rs` is only `PipeHandle(pub u32)` | No IPC implementation, quantum or otherwise, beyond shell/process scaffolding. |
| QHAL | partial | Generic hardware layer in `kernel/src/hal/*`; shell status prints `QHAL drivers:   None loaded` | There is a generic HAL/device registry, but no true quantum hardware abstraction layer or vendor QPU drivers. |
| QISA / WarQIR | missing | No `warqir`, `qisa`, or assembler modules in source tree; only `OpenQASM` support under `crates/waros-quantum/src/qasm/*` | QASM exists; WarQIR/QISA do not. |
| AI subsystem | missing | `kernel/src/exec/syscalls/ai.rs` returns `ENOSYS`; no AI crate/module in kernel or workspace | AI exists in blueprint only. |
| QuantumNet | missing | `kernel/src/net/*` is classical network code; shell quantum status prints `Quantum net:    Not available`; `sys_qkd_bb84` returns `ENOSYS` | Networking is classical Ethernet/IP/TCP/HTTPS, not quantum networking. |
| Linux compatibility layer | partial | ELF loader/process machinery in `kernel/src/exec/loader.rs` and `kernel/src/exec/syscall.rs`; `compat.rs` and many syscalls remain stubbed | Enough code to show direction, not enough to claim compatibility. |
| Secure boot / PQ boot chain | missing | Boot uses `bootloader` image generation; no signature verification stage in `kernel/` build/run path | No PQ boot chain today. |
| Package manager / distribution path | partial | In-kernel `warpkg` under `kernel/src/pkg/*`; built-in bootstrap packages are JSON bundles and shell scripts | Local bootstrap package manager exists, but there is no distribution-grade repository, signing pipeline, installer, or ISO flow. |

## 5. Current Technical Strengths
- The strongest part of WarOS today is not the kernel: it is the userspace quantum SDK. `waros-quantum` has real simulators, algorithm implementations, QASM parsing/serialization, IBM Runtime support, examples, benchmarks, and a broad automated Rust test suite that actually passes.
- The project already exposes the same quantum core through three usable front doors: Rust crate, CLI, and Python package. That is real developer leverage and a credible differentiation point right now.
- The kernel is not vapor. It does boot, initialize interrupts/memory/devices, mount a persistent WarFS volume on virtio-blk, obtain DHCP on a supported NIC, and present an interactive shell under QEMU.
- The kernel networking stack is more substantial than a toy shell command set. There is concrete TCP/HTTP/TLS plumbing and a kernel-side IBM Runtime client, even though it is not yet hardened enough to market aggressively.
- The repo already has a decent build-and-test discipline for the workspace crates and Python bindings. The absence of fake maturity is not total, but there is real executable evidence across multiple surfaces.

## 6. Highest-Risk Weaknesses
- The biggest risk is architectural overhang: `BLUEPRINT.md` describes a much larger system than the codebase currently delivers. Without a strict status discipline, contributors and outsiders can easily mistake direction for implementation.
- The kernel syscall and process model are ahead of their proof. There is real ELF/process code, but a large fraction of Linux-like syscalls still return `ENOSYS`, there are no kernel integration tests for userspace execution, and CI does not boot the kernel.
- Security claims are fragile today. TLS in the kernel explicitly warns that certificate validation is not implemented, package signatures are bootstrap hashes rather than real ML-DSA verification, and there is no secure boot chain.
- The naming boundary between generic HAL code and the blueprint's QHAL is muddy. The repo has `kernel/src/hal/*`, but the shell itself reports that no QHAL drivers are loaded. That ambiguity is a documentation and design risk.
- Contributor productivity will be constrained by the repo's split personality unless boundaries are clarified: the userspace SDK is test-rich and concrete, while the kernel has many large modules with much lighter validation and a high ratio of surface area to proved behavior.

## 7. Recommended Priority Order
- P0: Add an automated headless kernel boot-and-shell smoke test.
  Why it matters: the kernel currently only gets a compile check in CI, so the most important OS claim, "it boots and reaches a usable shell," is not continuously verified.
  Files/crates likely involved: `.github/workflows/ci.yml`, `kernel/tools/run_qemu.sh`, `kernel/tools/run_qemu.ps1`, a new headless smoke script under `kernel/tools/`.
  Acceptance criteria: CI boots `waros-bios.img` or `waros.img` under QEMU, captures serial output, and asserts markers for memory init, WarFS init, task scheduler init, NIC/DHCP init when enabled, and shell readiness.
- P0: Audit and gate the syscall surface so unsupported paths are explicit.
  Why it matters: `kernel/src/exec/syscall.rs` advertises far more than the kernel actually supports, which creates false Linux-compatibility expectations.
  Files/crates likely involved: `kernel/src/exec/syscall.rs`, `kernel/src/exec/syscalls/*`, `kernel/src/exec/compat.rs`, `kernel/src/shell/commands.rs`, `README.md`.
  Acceptance criteria: every exported syscall is either implemented with a test, hidden behind a feature/experimental gate, or documented as unsupported; no broad "Linux compatibility" claims remain without coverage.
- P0: Reconcile public documentation with repository reality.
  Why it matters: there are already concrete drifts, including README's 4 MiB heap claim and the statement that the kernel does not perform HTTPS requests.
  Files/crates likely involved: `README.md`, `BLUEPRINT.md`, `docs/ARCHITECTURE_AUDIT_MARCH_2026.md`, future status docs.
  Acceptance criteria: all top-level docs distinguish implemented/partial/planned status and remove contradicted statements.
- P1: Prove one minimal end-to-end userspace execution path in the kernel.
  Why it matters: WarExec/ELF loading exists in code, but there is not enough repository evidence yet to claim a real userspace model.
  Files/crates likely involved: `kernel/src/exec/*`, `kernel/src/shell/commands.rs`, a minimal sample userspace ELF build target.
  Acceptance criteria: a small user binary can be loaded from WarFS, started from the shell, return an exit code, and be exercised by an automated QEMU smoke test.
- P1: Harden kernel HTTPS and package verification or mark them experimental.
  Why it matters: the kernel currently has real network/TLS/IBM/package code, but critical security properties are missing.
  Files/crates likely involved: `kernel/src/net/tls/mod.rs`, `kernel/src/net/ibm.rs`, `kernel/src/pkg/signature.rs`, `kernel/src/pkg/*`.
  Acceptance criteria: either certificate verification and real package-signature verification are implemented, or the commands are clearly labeled experimental and disabled by default in release docs.
- P1: Clarify the HAL/QHAL boundary.
  Why it matters: the current `hal` tree is generic device infrastructure, not the blueprint QHAL, and the distinction needs to be explicit before more code accumulates.
  Files/crates likely involved: `kernel/src/hal/*`, `README.md`, `BLUEPRINT.md`, shell status/help text.
  Acceptance criteria: the code and docs consistently say whether `hal` is generic hardware plumbing, proto-QHAL, or a separate concept; no implied real-QPU support remains without drivers.
- P2: Expand kernel integration coverage for WarFS persistence and networking.
  Why it matters: these are among the most real kernel subsystems, and they deserve reproducible boot/runtime tests before new subsystems are added.
  Files/crates likely involved: `kernel/src/fs/*`, `kernel/src/disk/*`, `kernel/src/net/*`, QEMU smoke harness.
  Acceptance criteria: automated tests prove first-boot format, second-boot persistence, DHCP acquisition, DNS resolution, and one deterministic HTTP/TLS request path.
- P2: Converge the kernel quantum API with the userspace SDK where possible.
  Why it matters: the repo currently has two separate simulation/control surfaces with overlapping concepts and no shared IR beyond QASM export.
  Files/crates likely involved: `kernel/src/quantum/*`, `crates/waros-quantum/*`.
  Acceptance criteria: shared fixtures or shared serialization logic exist, and kernel quantum demos are validated against userspace results for equivalent circuits.
- P3: Tackle blueprint-scale subsystems only after the above proof/hardening work.
  Why it matters: QRM, UMA-Q, QuantumIPC, WarQIR, AI, QuantumNet, secure boot, and a real Linux compatibility layer are all large efforts that should not be layered onto an unverified base.
  Files/crates likely involved: new modules rather than today's core crates.
  Acceptance criteria: each future subsystem begins from a narrow spec, explicit non-goals, and testable milestones rather than blueprint-scale scope.

## 8. Prompting Guidance for Future Codex Work
- Keep prompts small and local. Ask for one subsystem change at a time, not "implement QHAL" or "add Linux compatibility."
- Name exact files or modules to inspect or edit, especially in `kernel/src/exec/*`, `kernel/src/net/*`, `kernel/src/fs/*`, and the userspace crates.
- State explicit non-goals, for example: "do not invent real QPU support," "do not claim Linux compatibility," or "do not widen the syscall surface."
- Require tests or proof for every claim: Rust tests, Python tests, CLI runs, kernel serial boot logs, or QEMU smoke assertions.
- Do not allow invented maturity language. Prompts should force classification as implemented, partial, stubbed, or planned.

## 9. Proposed Next 10 Issues
### 1. Add Headless QEMU Boot Smoke Test to CI
- Title: `kernel: add headless QEMU boot smoke test with serial-output assertions`
- Scope: boot the kernel image in CI and assert successful arrival at the shell.
- Affected modules: `.github/workflows/ci.yml`, `kernel/tools/*`, new smoke script.
- Acceptance criteria: CI fails if serial output does not contain core boot markers and `WarOS shell online`.

### 2. Verify WarFS Persistence Across Reboot
- Title: `kernel/fs: add automated first-boot format and second-boot persistence test`
- Scope: prove that WarFS formats a blank virtio-blk disk and reloads files on the next boot.
- Affected modules: `kernel/src/fs/*`, `kernel/src/disk/*`, QEMU smoke harness.
- Acceptance criteria: test writes a file on boot N and reads the same file on boot N+1.

### 3. Collapse or Annotate Stubbed Syscalls
- Title: `kernel/exec: gate or document ENOSYS syscall paths`
- Scope: review syscall table and either implement, feature-gate, or explicitly mark unsupported handlers.
- Affected modules: `kernel/src/exec/syscall.rs`, `kernel/src/exec/syscalls/*`, `kernel/src/exec/compat.rs`.
- Acceptance criteria: no silent compatibility claims remain; each syscall has status documentation and tests where implemented.

### 4. Prove One Minimal ELF Userspace Program
- Title: `kernel/exec: add minimal user ELF sample and execve smoke test`
- Scope: create a tiny userspace binary and validate `spawn/execve/wait`.
- Affected modules: `kernel/src/exec/*`, `kernel/src/shell/commands.rs`, new sample binary target.
- Acceptance criteria: shell launches the sample binary, captures its stdout, and reports its exit status in automated boot testing.

### 5. Harden Kernel TLS or Disable It by Default
- Title: `kernel/net: implement certificate verification or mark HTTPS paths experimental`
- Scope: resolve the current "encrypted but not verified" TLS state.
- Affected modules: `kernel/src/net/tls/mod.rs`, `kernel/src/net/http.rs`, `kernel/src/net/ibm.rs`, `README.md`.
- Acceptance criteria: either certificate validation exists with tests, or release-facing docs/features clearly disable/flag the path.

### 6. Replace Bootstrap Package Signatures
- Title: `kernel/pkg: replace SHA3 bootstrap signatures with real PQ signature verification`
- Scope: move package verification beyond digest equality.
- Affected modules: `kernel/src/pkg/signature.rs`, `kernel/src/pkg/*`, possibly `crates/waros-crypto`.
- Acceptance criteria: package verification uses an actual PQ signature scheme and rejects tampered bundles in tests.

### 7. Clarify HAL vs QHAL Naming
- Title: `kernel/hal: define whether current hal tree is generic HAL or proto-QHAL`
- Scope: remove architectural ambiguity between shipped code and blueprint terminology.
- Affected modules: `kernel/src/hal/*`, `kernel/src/shell/commands.rs`, `README.md`, `BLUEPRINT.md`.
- Acceptance criteria: terminology is consistent and no docs imply real QPU-driver support without code.

### 8. Add Networking Smoke Coverage
- Title: `kernel/net: add automated DHCP, DNS, and deterministic HTTP/TLS smoke tests`
- Scope: validate the most real kernel networking paths under QEMU.
- Affected modules: `kernel/src/net/*`, QEMU harness, possibly a controlled test endpoint or fixture.
- Acceptance criteria: automated test proves NIC init, DHCP lease, DNS resolution, and one HTTP/TLS request path.

### 9. Cross-Validate Kernel Quantum Output Against Userspace SDK
- Title: `kernel/quantum: add shared Bell/GHZ/QFT fixtures validated against waros-quantum`
- Scope: reduce drift between the kernel simulator and the userspace simulator.
- Affected modules: `kernel/src/quantum/*`, `crates/waros-quantum/*`, shared fixtures under `examples/qasm/`.
- Acceptance criteria: equivalent fixtures produce matching distributions within tolerance in automated tests.

### 10. Introduce a Maintained Implementation Status Document
- Title: `docs: maintain machine-readable implementation status for blueprint subsystems`
- Scope: prevent future drift between blueprint and shipped reality.
- Affected modules: `README.md`, `BLUEPRINT.md`, `docs/*`.
- Acceptance criteria: one status document is updated whenever a subsystem claim changes, and top-level docs link to it directly.
