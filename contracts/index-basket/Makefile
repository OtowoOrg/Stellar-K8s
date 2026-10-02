# Index basket + factory. The factory tests deploy the compiled basket WASM,
# so `test` builds it first.

WASM_TARGET := wasm32v1-none

.PHONY: all build test lint fmt clean

all: build test lint

build:
	cargo build --workspace --target $(WASM_TARGET) --release

test: build
	cargo test --workspace

lint: build
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy --workspace --target $(WASM_TARGET) --release -- -D warnings

fmt:
	cargo fmt --all

clean:
	cargo clean
