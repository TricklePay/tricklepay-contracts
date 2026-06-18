WASM_TARGET := wasm32-unknown-unknown
WASM := target/$(WASM_TARGET)/release/tricklepay_stream.wasm

.PHONY: all build wasm test fmt fmt-check lint clean deploy

all: fmt-check lint test

# Native debug build.
build:
	cargo build

# Optimized WASM artifact for deployment.
wasm:
	cargo build --release --target $(WASM_TARGET)
	@echo "built $(WASM)"

# Run the full test suite.
test:
	cargo test

# Format the workspace in place.
fmt:
	cargo fmt

# Verify formatting without modifying files (used in CI).
fmt-check:
	cargo fmt --check

# Lint every target and treat warnings as errors.
lint:
	cargo clippy --all-targets -- -D warnings

# Remove build artifacts.
clean:
	cargo clean

# Build, install, and deploy to testnet. Pass an identity: make deploy ID=alice
deploy:
	./scripts/deploy.sh $(ID)
