.PHONY: all build debug release debug-video release-video test test-unit test-integration test-video test-release sanity clean fmt lint check help build-webapp webapp

# Default target
all: debug

# ── Build ─────────────────────────────────────────────────────────────────

debug: ## Build debug mode (images + SVG only)
	cargo build

release: ## Build release mode (images + SVG only)
	cargo build --release

debug-video: ## Build debug mode with video support (requires nasm, gcc, pkg-config)
	cargo build --features video

release-video: ## Build release mode with video support
	cargo build --release --features video

build: debug release ## Build both debug and release

# ── Test ──────────────────────────────────────────────────────────────────

test-unit: ## Run unit tests (debug)
	cargo test --lib

test-integration: ## Run integration tests for images and SVGs (debug)
	cargo test --test picture_tests
	cargo test --test svg_tests

test-video: ## Run video integration tests (requires video feature + ffmpeg)
	cargo test --features video --test video_tests

test: test-unit test-integration ## Run all non-video tests (debug)

test-release: ## Run all non-video tests (release)
	cargo test --release

# ── Code Quality ──────────────────────────────────────────────────────────

fmt: ## Format code
	cargo fmt

fmt-check: ## Check formatting without modifying
	cargo fmt -- --check

lint: ## Run clippy lints (both default and video)
	cargo clippy -- -D warnings
	cargo clippy --features video -- -D warnings

check: ## Type-check without building
	cargo check

# ── Sanity ────────────────────────────────────────────────────────────────

sanity: fmt-check lint debug test-unit test-integration release test-release debug-video test-video ## Full check (default + video)
	@echo ""
	@echo "========================================="
	@echo " All sanity checks passed."
	@echo "========================================="

# ── Web App ───────────────────────────────────────────────────────────────

build-webapp: release ## Build webapp server binary + infinishield release binary
	go build -o target/release/infinishield-webapp webapp/main.go

webapp: build-webapp ## Run webapp on port 1983
	@echo "Starting infinishield webapp on http://localhost:1983"
	cd $(CURDIR) && target/release/infinishield-webapp

# ── Clean ─────────────────────────────────────────────────────────────────

clean: ## Remove build artifacts and test outputs
	cargo clean
	rm -rf testing_output

# ── Help ──────────────────────────────────────────────────────────────────

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'
