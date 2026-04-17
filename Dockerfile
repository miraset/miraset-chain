# Use Rust official image for building (latest stable)
FROM rust:latest AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary
RUN cargo build --release --bin miraset

# Runtime image
FROM debian:trixie-slim

# Install required libraries
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /app/target/release/miraset /usr/local/bin/miraset

# Create data directory
RUN mkdir -p /data

# Set working directory
WORKDIR /data

# Expose RPC port
EXPOSE 9944

# Default command
CMD ["/usr/local/bin/miraset", "node", "start", "--rpc-addr", "0.0.0.0:9944"]
