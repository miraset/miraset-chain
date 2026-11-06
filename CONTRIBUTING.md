# Contributing

Read [AGENTS.md](AGENTS.md) and its linked shared instructions first.
Start with the [RDP](RDP.md), [plan](PLAN.md) and
[architecture](ARCHITECTURE.md). Target documents are proposals; the current
code is a starter binary.

## Scope and review

Keep public changes limited to chain protocol, cryptography, consensus,
execution, storage, network/node RPC and their documentation/tests.
Do not import private worker/product code, prompts, payment integrations,
credit ledgers or extracted history. Confirm licenses before dependency reuse
or publication. Do not change visibility or deploy without authorization.

Use requirement slugs from the PRD throughout stories, tests and reviews.
Documentation, test generation and trivial refactors may use the configured
inline path. Multi-module and medium/high-risk features require the SDLC
specialist sequence and independent gates. Human Architecture, Security and
Code Review approvals are mandatory for high-risk changes.

## Rust checks

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

After creating an approved workspace, apply `--workspace` to build/test/clippy.
Add focused built-in Rust tests and reproducible property/fault tests.
Do not use the placeholder Node test script as Rust validation.

Record commands and outcomes, update affected docs, and keep changes reviewable
by slice. Do not commit credentials or alter accepted ADRs in place.
See [ADR policy](docs/adr/README.md) and [security](SECURITY.md).
