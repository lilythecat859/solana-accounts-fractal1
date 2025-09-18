---------- Builder ----------
FROM rust:1.80-slim AS builder

WORKDIR /app

# Build‑time dependencies (OpenSSL headers, pkg‑config)
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Cache Cargo dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p crates/ingest/src crates/fractal-rle/src crates/fractal-shard/src crates/rpc/src \
    && echo "fn main() {}" > crates/rpc/src/main.rs \
    && cargo build --release --bin fractal-rpc \
    && rm -rf crates/*/src

# Copy the real source and build the final binary
COPY . .
RUN touch crates/rpc/src/main.rs && cargo build --release --bin fractal-rpc

# ---------- Runtime ----------
FROM debian:bookworm-slim

# Runtime dependencies (CA certs + OpenSSL shared lib)
RUN apt-get update && \
    apt-get install -y ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

# Create a non‑root user for security
RUN useradd -m appuser
WORKDIR /home/appuser
COPY --from=builder /app/target/release/fractal-rpc /usr/local/bin/fractal-rpc
RUN chown appuser:appuser /usr/local/bin/fractal-rpc

USER appuser

EXPOSE 8899 9090

HEALTHCHECK CMD curl -f http://localhost:8899/health || exit 1

CMD ["fractal-rpc"]
