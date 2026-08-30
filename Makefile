# soroban-sdk requires wasm32v1-none on Rust 1.82+; wasm32-unknown-unknown
# enables wasm features the Soroban environment does not support.
WASM_TARGET := wasm32v1-none
WASM := target/$(WASM_TARGET)/release/tricklepay_stream.wasm

.PHONY: all help build wasm test fmt fmt-check lint audit clean deploy

all: fmt-check lint test

# List available targets with their descriptions.
help:
	@awk '/^# /{line=substr($$0,3);msg=(msg==""?line:msg " " line);next} /^[a-zA-Z_-]+:/{if(msg!=""){split($$0,t,":");printf "  %-10s %s\n",t[1],msg};msg=""} {if(!/^# /)msg=""}' $(MAKEFILE_LIST)

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

# Audit dependencies. Unavoidable Soroban transitive warnings are ignored via
# .cargo/audit.toml (derivative/paste unmaintained, spin yanked) - these crates
# are not used in the deployed WASM; vulnerability advisories remain enabled.
audit:
	cargo audit --deny warnings

# Remove build artifacts.
clean:
	cargo clean

# Build, install, and deploy to testnet. Pass an identity: make deploy ID=alice
deploy:
	./scripts/deploy.sh $(ID)
