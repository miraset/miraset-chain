# syntax=docker/dockerfile:1
# Experimental/devnet image for miraset-chain (ADR-0016 Proposed; not production settlement).

FROM rust:1.97-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY proto ./proto
COPY src ./src
COPY lib ./lib
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin miraset-chain \
    && cp target/release/miraset-chain /usr/local/bin/miraset-chain

FROM debian:bookworm-slim
# /keys/<name> are mount points for the devnet's per-key volumes (key custody D5): a fresh named
# volume copies their owner (miraset) and mode (0700), so devnet-init can write the keys and
# only the miraset user can read them.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /var/lib/miraset miraset \
    && mkdir -p /var/lib/miraset /var/lib/miraset-guard /var/lib/miraset-key /devnet \
       /keys/faucet /keys/coordinator /keys/governance \
       /keys/node0 /keys/node1 /keys/node2 /keys/node3 /keys/node4 \
    && chmod 700 /var/lib/miraset-key /keys/faucet /keys/coordinator /keys/governance \
       /keys/node0 /keys/node1 /keys/node2 /keys/node3 /keys/node4 \
    && chown -R miraset:miraset /var/lib/miraset /var/lib/miraset-guard /var/lib/miraset-key \
       /devnet /keys
COPY --from=build /usr/local/bin/miraset-chain /usr/local/bin/miraset-chain
USER miraset
WORKDIR /var/lib/miraset
EXPOSE 8545 9948
ENTRYPOINT ["miraset-chain"]
CMD ["--help"]
