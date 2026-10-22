#!/usr/bin/env bash
# Mounts a fresh volume with FUSE, uses it through ordinary tools, unmounts it
# with Ctrl-C (SIGINT) and checks it offline.
#   cargo build --features fuse && experiments/fuse_smoke.sh
set -euo pipefail
lfs=${LFS:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/lfs}
work=$(mktemp -d)
mnt="$work/mnt"
trap 'fusermount3 -u "$mnt" 2>/dev/null || umount "$mnt" 2>/dev/null || true; rm -rf "$work"' EXIT
mkdir "$mnt"
"$lfs" mkfs "$work/img.lfs" --size 16M
"$lfs" mount "$work/img.lfs" "$mnt" 2> "$work/mount.log" &
pid=$!
for _ in $(seq 50); do mountpoint -q "$mnt" && break; sleep 0.1; done
mountpoint -q "$mnt"
set -x
echo hello > "$mnt/a.txt"
mkdir -p "$mnt/d/e"
cp "$0" "$mnt/d/e/script"
cmp "$0" "$mnt/d/e/script"
ln -s a.txt "$mnt/s"
test "$(cat "$mnt/s")" = hello
ln "$mnt/a.txt" "$mnt/h"
mv "$mnt/h" "$mnt/d/h2"
test "$(stat -c %h "$mnt/a.txt")" = 2
truncate -s 3 "$mnt/a.txt"
test "$(cat "$mnt/d/h2")" = hel
head -c 3000000 /dev/urandom > "$work/big"
cp "$work/big" "$mnt/big"
cmp "$work/big" "$mnt/big"
rm "$mnt/d/h2"
if rmdir "$mnt/d" 2>/dev/null; then exit 1; fi
echo "(atime: on)" > "$mnt/.lfs/config"
grep -q "(atime: on)" "$mnt/.lfs/config"
cat "$mnt/.lfs/stats"
df -h "$mnt"
ls -la "$mnt" "$mnt/d"
set +x
kill -INT "$pid"
wait "$pid"
cat "$work/mount.log"
if mountpoint -q "$mnt"; then exit 1; fi
"$lfs" fsck "$work/img.lfs"
"$lfs" ls -R -l "$work/img.lfs"
"$lfs" config "$work/img.lfs" | grep "(atime: on)"
"$lfs" get "$work/img.lfs" /big "$work/big.out"
cmp "$work/big" "$work/big.out"
echo "FUSE smoke test passed"
