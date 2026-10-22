#!/usr/bin/env bash
# Issue #1: type-checks the Windows (Dokan) and macOS (macFUSE) drivers from
# Linux, without either SDK.
#   bash experiments/issue-1-cross-check.sh
# Windows: dokan-sys builds the Dokan C library in its build script, which
# needs a Windows C toolchain, so its build script is swapped for one that
# only writes the constants the bindings include. macOS: fuser's
# `macos-no-mount` feature skips the pkg-config probe for macFUSE.
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"
rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin >/dev/null

cargo fetch --target x86_64-pc-windows-gnu >/dev/null
registry="$(dirname "$(cargo metadata --format-version 1 --all-features --filter-platform x86_64-pc-windows-gnu \
  | python3 -I -c 'import json,sys; print(next(p["manifest_path"] for p in json.load(sys.stdin)["packages"] if p["name"]=="dokan-sys"))')")"
stub="$(mktemp -d)"
trap 'rm -rf "$stub"' EXIT
cp -r "$registry" "$stub/dokan-sys"
cat > "$stub/dokan-sys/build.rs" <<'RS'
// Type-checking stub: writes the constants the real build script generates.
fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    std::fs::write(
        format!("{out}/version.rs"),
        "pub const DOKAN_VERSION: u32 = 206;\npub const DOKAN_MINIMUM_COMPATIBLE_VERSION: u32 = 200;\npub const DOKAN_DRIVER_NAME: &str = \"dokan2.sys\";\npub const DOKAN_NP_NAME: &str = \"Dokan2\";\npub const DOKAN_MAJOR_API_VERSION: &str = \"2\";\n",
    )
    .unwrap();
}
RS
# A separate target directory keeps the stubbed build out of the real one.
CARGO_TARGET_DIR="$stub/target" cargo clippy --target x86_64-pc-windows-gnu --all-features --all-targets \
  --config "patch.crates-io.dokan-sys.path=\"$stub/dokan-sys\"" -- -D warnings
echo "windows: ok"

cargo clippy --target aarch64-apple-darwin --features fuse,fuser/macos-no-mount --all-targets -- -D warnings
echo "macos: ok"
