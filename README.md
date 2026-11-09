# MIRASET Chain

Self-hosted permissioned blockchain development project: an **experimental,
devnet-only** HotStuff validator node (vendored `hotstuff_rs` 0.4.0 with MIRASET
patches). The HotStuff path is pending authoritative decision review (protocol
ADR-0016 is Proposed); it is not an approved replacement for production
settlement. See `TODO.md` §8 for what is and is not hardened.

## Getting started

Install a Rust toolchain supporting edition 2024, then:

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Run it (`miraset-chain` with no arguments prints help):

```sh
cargo run -- demo                       # in-memory single-validator demo
cargo run -- devnet-init --out devnet --validators 1 --p2p-addr-template 127.0.0.1:9948
cargo run -- start --genesis devnet/genesis.json --key-file devnet/node0.key --init-signing-guard
cargo run -- db inspect --data-dir .miraset_data   # offline maintenance, node stopped
```

A five-validator Docker devnet and its fault suite are described in
[docs/devnet.md](docs/devnet.md).

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
