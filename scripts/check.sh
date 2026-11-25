#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test"
cargo test --workspace

echo "==> ui unit tests"
node --test ui/tests/*.test.mjs

echo "==> publish_eval_targets fail-closed + TBD scaffold"
python3 scripts/test_publish_eval_targets.py

echo "==> v0.7.0 taggable version gate"
./scripts/verify_v06_taggable.sh

echo "==> verify_v06_taggable fail-closed regression"
python3 scripts/test_verify_v06_taggable.py

echo "All checks passed."
