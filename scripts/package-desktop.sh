#!/usr/bin/env bash
# Packaging hook for desktop-release.yml: builds the `lfs` tool for one target
# label into <output-dir>, keeping the <target-label> <output-dir> contract.
#
# A mount driver that links a system library makes the binary fail to start
# where that library is missing, so such targets ship two binaries: a plain
# `lfs` and one with the driver (`-fuse` needs macFUSE, `-dokan` needs Dokan 2).
set -euo pipefail

LABEL="${1:?target label is required}"
OUTPUT_DIR="${2:?output directory is required}"
TAG="${DESKTOP_RELEASE_TAG:-dev}"
mkdir -p "$OUTPUT_DIR"

extension=""
if [[ "$LABEL" == windows-* ]]; then
  extension=".exe"
fi

# package <features> <asset-suffix>
package() {
  local features="$1" suffix="$2"
  if [ -n "$features" ]; then
    cargo build --release --features "$features"
  else
    cargo build --release
  fi
  local binary="target/release/lfs${extension}"
  [ -s "$binary" ] || { echo "Expected binary was not built: $binary" >&2; exit 1; }
  local asset="lfs-${LABEL}${suffix}-${TAG}${extension}"
  cp "$binary" "$OUTPUT_DIR/$asset"
  echo "Packaged $OUTPUT_DIR/$asset"
}

case "$LABEL" in
  # The Linux FUSE driver is pure Rust and only needs fusermount3 to mount.
  linux-*) package fuse "" ;;
  macos-*) package "" ""; package fuse "-fuse" ;;
  # dokan-sys builds the Dokan 2 user-mode library from source when no
  # installed copy is configured; the binary then uses the installed dokan2.dll.
  windows-x64) package "" ""; package dokan "-dokan" ;;
  *) package "" "" ;;
esac
