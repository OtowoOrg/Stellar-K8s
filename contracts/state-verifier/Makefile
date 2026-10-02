# State verifier. The gas profile runs the release WASM, so `test` builds it
# first. `contractimport!` does not make Cargo notice a rebuilt WASM, so the
# profile test is touched to force it to re-embed the fresh artifact.

WASM_TARGET := wasm32v1-none

.PHONY: all build test profile lint fmt fixture clean

all: build test lint

build:
	cargo build --target $(WASM_TARGET) --release

test: build
	touch tests/gas_profile.rs
	cargo test

profile: build ## Print the per-entry-point gas table
	touch tests/gas_profile.rs
	cargo test --test gas_profile -- --nocapture --test-threads 1

lint: build
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo clippy --target $(WASM_TARGET) --release -- -D warnings

fmt:
	cargo fmt

fixture: ## Regenerate the mainnet fixture from Horizon
	python3 scripts/fetch_fixture.py > src/fixtures.rs

clean:
	cargo clean
