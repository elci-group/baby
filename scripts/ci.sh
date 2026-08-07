#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --all-targets -- -D warnings

echo "==> cargo test"
cargo test --all-targets

echo "==> cargo doc"
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

echo "==> cargo audit"
cargo audit

echo "==> brandi lint"
brandi lint --fail-under 80

echo "==> all checks passed"
