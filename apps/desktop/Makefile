# Curio Reader Development Makefile

.PHONY: help setup dev build test lint fmt clean release docs

# Default target
help:
	@echo "Curio Reader Development Commands"
	@echo ""
	@echo "Setup:"
	@echo "  make setup       - First-time development setup"
	@echo "  make deps        - Install/update dependencies"
	@echo ""
	@echo "Development:"
	@echo "  make dev         - Start development server"
	@echo "  make build       - Build for production"
	@echo "  make build-debug - Build debug version"
	@echo ""
	@echo "Quality:"
	@echo "  make test        - Run all tests"
	@echo "  make test-rust   - Run Rust tests only"
	@echo "  make test-ts     - Run TypeScript tests only"
	@echo "  make test-e2e    - Run E2E tests"
	@echo "  make cov         - Generate coverage report"
	@echo "  make lint        - Run all linters"
	@echo "  make lint-fix    - Run linters and fix issues"
	@echo "  make fmt         - Format all code"
	@echo "  make check       - Run all checks (lint + test)"
	@echo ""
	@echo "Utilities:"
	@echo "  make gen-types   - Regenerate TypeScript types from Rust"
	@echo "  make db-reset    - Reset development database"
	@echo "  make clean       - Clean build artifacts"
	@echo "  make ytdlp-update - Update bundled yt-dlp"
	@echo ""
	@echo "Release:"
	@echo "  make release     - Build release for current platform"
	@echo "  make release-mac - Build macOS release"
	@echo "  make release-win - Build Windows release"
	@echo "  make release-linux - Build Linux release"

# ─────────────────────────────────────────────────────────────
# Setup
# ─────────────────────────────────────────────────────────────

setup: deps setup-hooks setup-ytdlp
	@echo "Development environment ready!"
	@echo "Run 'make dev' to start developing."

deps:
	@echo "Installing dependencies..."
	npm install
	cd src-tauri && cargo fetch

setup-hooks:
	@echo "Setting up git hooks..."
	npx lefthook install

setup-ytdlp:
	@echo "Setting up yt-dlp..."
	./scripts/bundle-ytdlp.sh

# ─────────────────────────────────────────────────────────────
# Development
# ─────────────────────────────────────────────────────────────

dev:
	npm run tauri dev

build:
	npm run tauri build

build-debug:
	npm run tauri build -- --debug

# ─────────────────────────────────────────────────────────────
# Testing
# ─────────────────────────────────────────────────────────────

test: test-rust test-ts

test-rust:
	@echo "Running Rust tests..."
	cd src-tauri && cargo test

test-ts:
	@echo "Running TypeScript tests..."
	npm test

test-e2e:
	@echo "Running E2E tests..."
	npm run test:e2e

test-watch:
	npm run test:watch

cov: cov-rust cov-ts
	@echo "Coverage reports generated in ./coverage/"

cov-rust:
	@echo "Generating Rust coverage..."
	cd src-tauri && cargo llvm-cov --html --output-dir ../coverage/rust 2>/dev/null || echo "Note: cargo-llvm-cov not installed. Run: cargo install cargo-llvm-cov"

cov-ts:
	@echo "Generating TypeScript coverage..."
	npm run test:coverage

# ─────────────────────────────────────────────────────────────
# Code Quality
# ─────────────────────────────────────────────────────────────

lint: lint-rust lint-ts

lint-rust:
	@echo "Linting Rust..."
	cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings

lint-ts:
	@echo "Linting TypeScript/Svelte..."
	npm run lint

lint-fix: lint-fix-rust lint-fix-ts

lint-fix-rust:
	cd src-tauri && cargo clippy --fix --allow-dirty --allow-staged

lint-fix-ts:
	npm run lint:fix

fmt: fmt-rust fmt-ts

fmt-rust:
	@echo "Formatting Rust..."
	cd src-tauri && cargo fmt

fmt-ts:
	@echo "Formatting TypeScript/Svelte..."
	npm run format

check: lint test
	@echo "All checks passed!"

# Pre-commit hook target (fast checks only)
pre-commit: fmt lint

# ─────────────────────────────────────────────────────────────
# Utilities
# ─────────────────────────────────────────────────────────────

gen-types:
	@echo "Generating TypeScript types from Rust..."
	./scripts/gen-types.sh

db-reset:
	@echo "Resetting development database..."
	rm -f ~/.config/curio-reader/curio.db
	rm -f ~/.local/share/curio-reader/curio.db
	@echo "Database reset. Will be recreated on next run."

clean:
	@echo "Cleaning build artifacts..."
	rm -rf src-tauri/target/
	rm -rf build/
	rm -rf .svelte-kit/
	rm -rf node_modules/.vite/
	rm -rf coverage/

ytdlp-update:
	@echo "Updating yt-dlp..."
	./scripts/bundle-ytdlp.sh --force

# ─────────────────────────────────────────────────────────────
# Release
# ─────────────────────────────────────────────────────────────

release:
	npm run tauri build -- --release

release-mac:
	npm run tauri build -- --target universal-apple-darwin

release-win:
	npm run tauri build -- --target x86_64-pc-windows-msvc

release-linux:
	npm run tauri build -- --target x86_64-unknown-linux-gnu

# ─────────────────────────────────────────────────────────────
# Documentation
# ─────────────────────────────────────────────────────────────

docs:
	@echo "Generating Rust documentation..."
	cd src-tauri && cargo doc --no-deps --open

docs-serve:
	@echo "Serving documentation..."
	cd src-tauri && cargo doc --no-deps && python3 -m http.server 8000 -d target/doc
