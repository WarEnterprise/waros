# Contributing to WarOS

We welcome contributors of all backgrounds and skill levels.

## Setup

```bash
git clone https://github.com/warenterprise/waros.git
cd waros
cargo build
cargo test
```

## Code Standards

- **Rust**: `rustfmt` + `clippy` with all warnings
- No `unwrap()` in library code
- Document all public APIs
- Quantum tests: use statistical assertions with tolerance

## Pull Request Process

1. Fork and create a feature branch
2. Write code and tests
3. `cargo fmt && cargo clippy && cargo test`
4. Submit PR with clear description

## Areas for Contribution

| Area | Difficulty |
|------|------------|
| New quantum gates | Medium |
| Circuit optimizer | Hard |
| OpenQASM parser | Medium |
| Python bindings (PyO3) | Medium |
| Documentation | Easy |
| Benchmarks | Medium |
| Error correction codes | Hard |
| Post-quantum crypto | Hard |

## License

By contributing, you agree your code is licensed under Apache 2.0.

---

**War Enterprise © 2026**
