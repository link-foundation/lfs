#!/usr/bin/env bash
# Builds docs/visualizer/lfs.wasm, the WebAssembly build of LFS used by
# blueprint.html (see src/wasm.rs):
#   bash docs/visualizer/build-wasm.sh
# Needs the target: rustup target add wasm32-unknown-unknown
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
cd "$repo"
cargo rustc --lib --crate-type cdylib --target wasm32-unknown-unknown --release \
  --no-default-features -- -C opt-level=s -C strip=symbols
cp target/wasm32-unknown-unknown/release/links_fs.wasm "$here/lfs.wasm"
echo "built $here/lfs.wasm ($(wc -c < "$here/lfs.wasm") bytes)"
