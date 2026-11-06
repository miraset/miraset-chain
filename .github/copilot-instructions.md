# Copilot instructions

## Source of truth

Use `/home/daochild/.obsidian/miraset-knowledge-vault` as the primary source
for protocol, PoCC economics and product requirements. Read
`docs/business/knowledge-sources.md` for the source map, decision precedence,
privacy boundaries and unavailable-source behavior. Inspect original chain
engineering ADRs and code directly; the vault's engineering summary is not
implementation evidence. Demos are secondary examples, not authority.

## Project shape

This repository is currently a Rust 2024 binary crate. `src/main.rs` is its only Rust source and entry point; `Cargo.toml` defines the crate and currently has no Rust dependencies. The repository name does not yet imply any implemented blockchain-specific architecture or behavior.

The root `package.json` and `pnpm-workspace.yaml` provide Node package metadata and the `@daochild/agents-config` development dependency. They do not define an application build; the `test` script in `package.json` is a placeholder that exits with an error.

## Build, test, and lint

Use Cargo for the Rust project:

```sh
cargo build
cargo run
cargo test
cargo test module::tests::test_name -- --exact
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

There are no Rust tests or configured lint scripts yet. Add tests alongside the Rust code using Rust's built-in test framework; run one test by replacing the example filter with its fully qualified test name.

## Architecture and conventions

- Keep the executable entry point in `src/main.rs` unless the crate grows enough to justify splitting implementation into modules.
- No application layering, domain model, service boundaries, or blockchain conventions have been established. Inspect the existing Rust code and manifest before introducing structure or dependencies; do not assume functionality from the repository name.
- Treat the root `AGENTS.md` as the project-specific workflow entry point. It links to the shared `@daochild/agents-config` instructions; read and follow those instructions before making changes. In particular, use the SDLC-gated workflow for multi-module or medium/high-risk changes, and confirm/document architecture choices before implementation.
