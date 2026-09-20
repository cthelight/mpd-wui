.DEFAULT_GOAL := help

.PHONY: help fmt fmt-check clippy test build run release docker up down clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "%-12s %s\n", $$1, $$2}'

fmt: ## Format Rust code and syntax-check JavaScript
	cargo fmt
	@for f in web/js/*.js; do node --check "$$f"; done

fmt-check: ## Check formatting and JavaScript syntax
	cargo fmt --check
	@for f in web/js/*.js; do node --check "$$f"; done

clippy: ## Run Clippy with warnings denied
	cargo clippy --workspace --all-targets -- -D warnings

test: ## Run JavaScript syntax checks and Rust tests
	@for f in web/js/*.js; do node --check "$$f"; done
	cargo test --workspace

build: ## Build the debug binary
	cargo build -p mpd-wui

run: ## Run the debug binary
	cargo run -p mpd-wui

MUSL_TARGET ?= x86_64-unknown-linux-musl

release: ## Build the static release binary (musl, ships in the Docker image)
	cargo build --release --locked --target $(MUSL_TARGET) -p mpd-wui

docker: ## Build the Docker image
	docker build -t mpd-wui:latest .

up: ## Build and start Docker Compose
	docker compose up --build

down: ## Stop Docker Compose
	docker compose down

clean: ## Remove build artifacts
	cargo clean
