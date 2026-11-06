# MIRASET Chain

Self-hosted permissioned blockchain development project. **Current code is
only a Rust starter binary; blockchain functionality is not implemented.**
The proposed HotStuff path is experimental/devnet-only, pending authoritative
decision review. It is not an approved replacement for production settlement.

## Getting started

Install a Rust toolchain supporting edition 2024, then run:

```sh
cargo build
cargo run
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Currently `cargo run` prints `Hello, world!` and `cargo test` runs zero tests.
The Node metadata supplies agent configuration, not an application build.

## Development documents

- [RDP: Requirements and Development Plan](RDP.md)
- [PRD](docs/business/miraset-chain-prd.md)
- [Implementation plan](PLAN.md)
- [Architecture](ARCHITECTURE.md)
- [Documentation index and governance](docs/README.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy and proposed threat controls](SECURITY.md)

These are draft engineering inputs, not passed SDLC gates or evidence of
production readiness. No deployment, throughput, or latency guarantees exist.
