.PHONY: build test fmt clean

# Real .wasm build of the deployable contract. Not needed by the test
# suite (fee_bumper_tests registers the contract natively), but proves
# fee-bumper actually compiles as a real Soroban contract.
build:
	cargo build --manifest-path fee_bumper/Cargo.toml --target wasm32v1-none --release

test:
	cargo test --manifest-path fee_bumper/Cargo.toml
	cargo test --manifest-path fee_bumper_tests/Cargo.toml

fmt:
	cargo fmt --manifest-path fee_bumper/Cargo.toml
	cargo fmt --manifest-path fee_bumper_tests/Cargo.toml

clean:
	cargo clean --manifest-path fee_bumper/Cargo.toml
	cargo clean --manifest-path fee_bumper_tests/Cargo.toml
