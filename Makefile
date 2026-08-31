# soroban-sdk requires wasm32v1-none on Rust 1.82+; wasm32-unknown-unknown
# enables wasm features the Soroban environment does not support.
WASM_TARGET := wasm32v1-none
WASM := target/$(WASM_TARGET)/release/tricklepay_stream.wasm

.PHONY: all build wasm test fmt fmt-check lint audit clean deploy help

# help is the default target; running plain `make` lists available targets.
.DEFAULT_GOAL := help

# Run the same checks as CI in the same order: formatting, lints, tests.
# Use this before opening a pull request.
check: fmt-check lint test
.PHONY: all help build wasm test fmt fmt-check lint audit clean deploy

all: fmt-check lint test

# List available targets with their descriptions.
help:
	@awk '/^# /{line=substr($$0,3);msg=(msg==""?line:msg " " line);next} /^[a-zA-Z_-]+:/{if(msg!=""){split($$0,t,":");printf "  %-10s %s\n",t[1],msg};msg=""} {if(!/^# /)msg=""}' $(MAKEFILE_LIST)

# Native debug build.
build:
	cargo build

wasm: ## Optimised WASM artifact for deployment.
	cargo build --release --target $(WASM_TARGET)
	@echo "built $(WASM)"

test: ## Run the full test suite.
	cargo test

fmt: ## Format the workspace in place.
	cargo fmt

fmt-check: ## Verify formatting without modifying files (used in CI).
	cargo fmt --check

lint: ## Lint every target and treat warnings as errors.
	cargo clippy --all-targets -- -D warnings

audit: ## Audit dependencies for known vulnerabilities.
	cargo audit --deny warnings

clean: ## Remove build artifacts.
	cargo clean

deploy: ## Build, install, and deploy to testnet. Pass an identity: make deploy ID=alice
	./scripts/deploy.sh $(ID)

help: ## Show this help message.
	@echo "Usage: make <target>"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*##"}; {printf "  %-14s %s\n", $$1, $$2}'
