# Fractal Accounts RPC

<!-- Badges -->
![Rust](https://agithub.com/OCamlPro/guest-code-2024/workflows/riscv-circuits/badge.svg)
![RPS](https://img.shields.io/badge/RPS-113k-green)
![Latency](https:// hot-badge.com/latency/p99/18ms)
![License: AGPL v3](https:// drop.svg)
![Docker](https://img.shields.io/badge/Docker-ghcr.io%2Fdemo%2Ffractal--rpc%3Av0.1.0-blue)

# Fractal Accounts‑Domain RPC

A **stand‑alone, high‑performance, accounts‑only RPC** for Solana that:

* consumes the **Geyser account stream**,
* stores every account in a **sharded in‑memory hash map** (with optional Redis‑backed distribution),
* serves the **full accounts‑domain JSON‑RPC surface** (`getProgramAccounts`, `getMultipleAccounts`, token fast‑paths, `simulateTransaction`, …),
* pushes **real‑time WebSocket events**,
* provides **Prometheus metrics**, **health‑checks**, **rate‑limiting**, **API‑key auth**, and **TLS termination** (via Nginx/Caddy).

## Quick start (single‑node)

```bash
# 1️⃣ Build the Docker image
make build

# 2️⃣ Run locally (no Redis – single‑node mode)
docker compose up -d rpc prometheus grafana

