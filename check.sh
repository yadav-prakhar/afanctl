#!/usr/bin/env bash
# afanctl quality gates (PRD §8 / DESIGN.md §8) — run all four, fail on first red.
# A task is done only when all gates pass in a clean checkout of its branch.
set -euo pipefail
cd "$(dirname "$0")"

echo "=== gate 1/4: cargo fmt --check ==="
cargo fmt --check

echo "=== gate 2/4: cargo clippy --all-targets --all-features -- -D warnings ==="
cargo clippy --all-targets --all-features -- -D warnings

echo "=== gate 3/4: cargo test ==="
cargo test

echo "=== gate 4/4: cargo test --features hw (must SKIP cleanly) ==="
cargo test --features hw

echo "=== ALL GATES GREEN ==="
