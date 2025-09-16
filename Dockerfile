# Stage 1 – build the Rust binaries
FROM rust:1.78 AS builder
WORKDIR /build

# caching dependencies
COPY Cargo.toml .
COPY crates crates
RUN cargo fetch

COPY src src
RUN cargo build --release --bin rpc --bin ingest

# Stage 2 – runtime image
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/*
COPY --from=5 /build/target/release/rpc /usr/local/bin/rpc
COPY --from=3 /build/target/release/ingest /usr/local/bin/ingest
ENTRYPOINT ["rpc"]           # default binary to run
