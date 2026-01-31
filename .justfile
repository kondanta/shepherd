#!/usr/bin/env -S just --justfile

set quiet := true
set shell := ['bash', '-euo', 'pipefail', '-c']

set dotenv-load := true

CARGO_ALL := "--all-targets --all-features"

# --- Basics ---

default:
    @just -l

[doc('Run the application with env validation')]
run command='serve' *args:
    cargo run -- {{ command }} {{ args }}

[doc('Run with OTLP feature enabled')]
run-otlp command='serve' *args:
    cargo run --features otlp -- {{ command }} {{ args }}

# Optional: Add a 'watch' mode for faster development
[doc('Watch for changes and restart server')]
dev:
    cargo watch -x "run -- serve"

[doc('Clean build artifacts')]
clean:
	cargo clean

[doc('Generate coverage report (HTML) using llvm-cov')]
coverage:
    cargo llvm-cov --all-features --workspace --html

[doc('Quick coverage summary in terminal')]
cov-summary:
    cargo llvm-cov --all-features --workspace

# --- Quality Control ---

[doc('Check Linting, Formatting, and Tests')]
check: lint fmt test

[doc('Lint the codebase')]
lint:
    cargo clippy {{ CARGO_ALL }} -- -D warnings

[doc('Format check')]
fmt check='--check':
    cargo fmt --all {{ check }}

[doc('Run tests')]
test *args:
    cargo test --all {{ args }}

# --- Maintenance ---

[doc('Apply automatic fixes')]
fix:
    cargo clippy {{ CARGO_ALL }} --fix --allow-dirty --allow-staged
    cargo fmt --all

[doc('Update dependencies')]
upd:
    cargo update
    @if command -v cargo-upgrade >/dev/null; then cargo upgrade; fi

# --- Pipelines ---

[doc('Full CI simulation')]
ci:
    -just lint
    -just fmt
    -just test
    -cargo build --release
