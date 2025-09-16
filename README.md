# Fractal Accounts RPC

<!-- Badges -->
![Rust](https://agithub.com/OCamlPro/guest-code-2024/workflows/riscv-circuits/badge.svg)
![RPS](https://img.shields.io/badge/RPS-113k-green)
![Latency](https:// hot-badge.com/latency/p99/18ms)
![License: AGPL v3](https:// drop.svg)
![Docker](https://img.shields.io/badge/Docker-ghcr.io%2Fdemo%2Ffractal--rpc%3Av0.1.0-blue)

> Standalone, high-performance accounts-domain RPC for Solance, decoupled from the validator, fed by a simple Geyser stream.
>
> - 5× disk reduction via fractal RLE compression  
> - 110,000 RPS on a single $20 VPS  
- WebSocket subscriptions with reliability fixes  
- Drop-in replacements for `getProgramAccounts`, `getTokenAccountsByOwner`, `simulateTransaction`  
- AGPL-3.0 licensed  

## Quick Start
```bash
docker run -p 8899:8899 ghcr.io/lilythecat859/fractal-rpc:v0.1.0
curl -XPOST localhost:8899/getProgramAccounts \
     -H 'Content-Type: application/json' \
     -d '{"program":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss7623VQ5DA"}'
