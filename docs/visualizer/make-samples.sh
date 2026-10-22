#!/usr/bin/env bash
# Regenerates the visualizer's sample maps from a real volume:
#   bash docs/visualizer/make-samples.sh [path/to/lfs]
# sample-map.json has gaps left by deleted files; sample-compacted.json is
# the same volume after `lfs compact`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
lfs="${1:-$repo/target/release/lfs}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
image="$work/sample.lfs"

"$lfs" mkfs "$image" --size 1M --block-size 1024 --label sample
"$lfs" put "$image" "$repo/src" /src
"$lfs" put "$image" "$repo/docs/case-studies/issue-1" /docs
head -c 100000 /dev/zero | tr '\0' 'x' | "$lfs" write "$image" /big.bin
"$lfs" ln "$image" -s /src/lib.rs /lib.rs
"$lfs" links "$image" create 1 2 >/dev/null
"$lfs" links "$image" create 2 1 >/dev/null
# Deleting every other file leaves free gaps between the survivors.
"$lfs" rm "$image" -r /src/fs/interchange /src/links/packet.rs /src/cli /src/driver/fuse.rs /src/journal.rs
"$lfs" fsck "$image"
"$lfs" map "$image" -o "$here/sample-map.json"
"$lfs" compact "$image"
"$lfs" fsck "$image"
"$lfs" map "$image" -o "$here/sample-compacted.json"
