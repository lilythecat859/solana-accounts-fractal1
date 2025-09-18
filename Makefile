.PHONY: build push run test load-test tf-up tf-down clean lint benchmark

build:
	docker build -t ghcr.io/lilythecat859/fractal-rpc:v0.1.0 .

push:
	docker push ghcr.io/lilythecat859/fractal-rpc:v0.1.0

run:
	docker compose up --build -d

test:
	cargo test --all

# New benchmark target – uses the Rust load‑tester in ./bench/benchmark.rs
benchmark:
	cargo run --manifest-path=bench/Cargo.toml --release -- \
	    --duration 30s \
	    --concurrency 12 \
	    --connections 400 \
	    --endpoint http://localhost:8899

load-test:
	# Simple wrk example (still useful for quick sanity checks)
	wrk -t12 -c400 -d30s -s bench.lua http://localhost:8899/getProgramAccounts

tf-up:
	cd terraform && terraform init && terraform apply -auto-approve

tf-down:
	cd terraform && terraform destroy -auto-approve

clean:
	docker compose down -v
	cargo clean

lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -Dwarnings
