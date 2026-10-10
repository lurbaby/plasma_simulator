#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"

echo "=== Building WASM physics engine ==="
export PATH="$HOME/.cargo/bin:$PATH"
# wasm-pack >= 0.14 takes the cargo passthrough flags as positional
# EXTRA_OPTIONS; the older `-- --no-default-features` form now makes it hand
# a bare `--` to `cargo build`, which errors out.
wasm-pack build "$REPO_ROOT/crates/tok-sym-core" \
  --target web \
  --out-dir "$REPO_ROOT/web/src/wasm" \
  --no-default-features \
  --features wasm

echo ""
echo "=== Building fusionsim (datasets, test runs, API server) ==="
cargo build --release -p fusion-cli --manifest-path "$REPO_ROOT/Cargo.toml"

echo ""
echo "=== Building frontend ==="
cd "$REPO_ROOT/web"
npm run build

echo ""
echo "=== Done! Output in web/dist/ ==="
echo "Run:  ./target/release/fusionsim serve --models-dir ../models   →  http://127.0.0.1:9999/"
