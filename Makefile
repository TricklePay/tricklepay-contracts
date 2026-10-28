# soroban-sdk requires wasm32v1-none on Rust 1.82+; wasm32-unknown-unknown
# enables wasm features the Soroban environment does not support.
WASM_TARGET := wasm32v1-none
WASM := target/$(WASM_TARGET)/release/tricklepay_stream.wasm

.PHONY: all check build wasm test fmt fmt-check lint audit clean deploy check-manifest check-markdown-links test-markdown-links snapshot help

# help is the default target; running plain `make` lists available targets.
.DEFAULT_GOAL := help

# Check the release overflow setting and the built contract interface.
# Use this before opening a pull request.
all check: check-manifest fmt-check lint test audit
	./scripts/diff-interface.sh

check-manifest: ## Verify Cargo manifests parse without resolving remote dependencies.
	cargo metadata --no-deps --locked

check-markdown-links: ## Check relative Markdown files and heading anchors.
	python3 scripts/check-markdown-links.py

test-markdown-links: ## Run unit tests for scripts/check-markdown-links.py.
	python3 scripts/test_check_markdown_links.py

snapshot: wasm ## Regenerate the checked-in contract interface snapshot.
	stellar contract inspect --wasm $(WASM) > docs/interface.txt

# Native debug build.
build: ## Native debug build.
	cargo build

wasm: ## Optimised WASM artifact for deployment.
	cargo build --locked --release --target $(WASM_TARGET)
	@echo "built $(WASM)"

.PHONY: wasm-hash
wasm-hash: ## Build the WASM with the locked dependencies and print its SHA-256 hash.
	@set -eu; \
	cargo build --locked --release --target "$(WASM_TARGET)"; \
	if command -v sha256sum >/dev/null 2>&1; then \
		hash=$$(sha256sum "$(WASM)" | awk '{print $$1}'); \
	elif command -v shasum >/dev/null 2>&1; then \
		hash=$$(shasum -a 256 "$(WASM)" | awk '{print $$1}'); \
	else \
		echo "Error: install sha256sum or shasum to compute the WASM SHA-256 hash." >&2; \
		exit 1; \
	fi; \
	printf 'sha256:%s\n' "$$hash"

size: wasm ## Report the compiled contract's WASM size (builds it first if needed).
	@echo "$(WASM): $$(wc -c < $(WASM) | tr -d ' ') bytes ($$(du -h $(WASM) | cut -f1))"

test: ## Run the full test suite.
	cargo test

fmt: ## Format the workspace in place.
	cargo fmt

fmt-check: ## Verify formatting without modifying files (used in CI).
	cargo fmt --check

lint: ## Lint every target and treat warnings as errors.
	cargo clippy --all-targets -- -D warnings

.PHONY: lint-scripts
lint-scripts: ## Check shell syntax and lint scripts with ShellCheck.
	@if ! command -v shellcheck >/dev/null 2>&1; then \
		echo "Error: ShellCheck is required. Install it with 'brew install shellcheck' or 'apt install shellcheck'." >&2; \
		exit 1; \
	fi
	@set -e; for script in scripts/*.sh; do bash -n "$$script"; done
	shellcheck scripts/*.sh

docs: ## Build crate documentation, treating warnings as errors.
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

audit: ## Audit dependencies for known vulnerabilities.
	cargo audit --deny warnings --file .cargo/audit.toml

clean: ## Remove build artifacts.
	cargo clean

deploy: ## Build, install, and deploy to testnet. Pass an identity: make deploy IDENTITY=alice
ifndef IDENTITY
	$(error IDENTITY is undefined. Please provide an identity argument (e.g. make deploy IDENTITY=alice))
endif
	./scripts/deploy.sh $(IDENTITY)

help: ## Show this help message.
	@echo "Usage: make <target>"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*##"}; {printf "  %-14s %s\n", $$1, $$2}'
