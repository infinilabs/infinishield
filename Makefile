.PHONY: all build debug release test test-unit test-integration test-release sanity clean fmt lint check help

# Default target
all: debug

# ── Build ─────────────────────────────────────────────────────────────────

debug: ## Build debug mode
	cargo build

release: ## Build release mode
	cargo build --release

build: debug release ## Build both debug and release

# ── Test ──────────────────────────────────────────────────────────────────

test-unit: ## Run unit tests (debug)
	cargo test --lib

test-integration: ## Run integration tests with real images and SVGs (debug)
	cargo test --test picture_tests
	cargo test --test svg_tests

test: test-unit test-integration ## Run all tests (debug)

test-release: ## Run all tests (release)
	cargo test --release

# ── Code Quality ──────────────────────────────────────────────────────────

fmt: ## Format code
	cargo fmt

fmt-check: ## Check formatting without modifying
	cargo fmt -- --check

lint: ## Run clippy lints
	cargo clippy -- -D warnings

check: ## Type-check without building
	cargo check

# ── Sanity ────────────────────────────────────────────────────────────────

sanity: fmt-check lint debug test-unit test-integration release test-release ## Full build + lint + all tests (debug & release)
	@echo ""
	@echo "========================================="
	@echo " All sanity checks passed."
	@echo "========================================="

# ── Clean ─────────────────────────────────────────────────────────────────

clean: ## Remove build artifacts, test outputs, and testing_output directory
	cargo clean
	rm -rf testing_output

# ── Help ──────────────────────────────────────────────────────────────────

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'
