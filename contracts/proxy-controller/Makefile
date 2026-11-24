.PHONY: build build-mocks test fmt clean

# Builds the two test fixture contracts to real Wasm. The integration tests
# in proxy_controller_tests import these compiled .wasm files at compile
# time (via soroban_sdk::contractimport!), so this must run before `test`.
build-mocks:
	cargo build --manifest-path mock_v1/Cargo.toml --target wasm32v1-none --release
	cargo build --manifest-path mock_v2/Cargo.toml --target wasm32v1-none --release

build: build-mocks
	cargo build --manifest-path proxy_controller/Cargo.toml --target wasm32v1-none --release

test: build-mocks
	cargo test --manifest-path proxy_controller/Cargo.toml
	cargo test --manifest-path proxy_controller_tests/Cargo.toml

fmt:
	cargo fmt --manifest-path proxy_controller/Cargo.toml
	cargo fmt --manifest-path mock_v1/Cargo.toml
	cargo fmt --manifest-path mock_v2/Cargo.toml
	cargo fmt --manifest-path proxy_controller_tests/Cargo.toml

clean:
	cargo clean --manifest-path proxy_controller/Cargo.toml
	cargo clean --manifest-path mock_v1/Cargo.toml
	cargo clean --manifest-path mock_v2/Cargo.toml
	cargo clean --manifest-path proxy_controller_tests/Cargo.toml
