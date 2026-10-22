---
bump: minor
---

### Added
- LFS, the links file system: a block-device file system in Rust whose metadata is an embedded doublets links database (united or split indices, optional adaptive bitstring index, variable-size store with a threshold) and that shares one optional journal with it.
- Files take one contiguous extent each; gaps between files are filled with link chunks, and online compaction keeps the free space in one extent.
- Settings that can change at run time, stored in `/.lfs/config` and set with `lfs config`.
- Links import and export: binary links notation, doublets files and LiNo.
- The `lfs` command line tool: `mkfs`, `fsck`, `map`, `compact`, `resize`, `put`, `get`, `ls`, `links`, `config`, `blueprint`, `demo`, `mount` and `serve`.
- Mount drivers: FUSE on Linux, FreeBSD and macOS (macFUSE), Dokan 2 on Windows x64, and an NFS version 3 server (`nfs` feature, `lfs mount --nfs` and `lfs serve`) that macOS mounts with its built-in `mount_nfs` and Linux with `mount -t nfs`.
- A disk-space map visualizer on GitHub Pages, and a byte-level blueprint page that runs LFS as WebAssembly to format, open and walk any configuration's volume down to single bytes.
- Image files are locked while open: one writer or many readers, so two processes never write one volume at once.
- `LFS_DOKAN_TRACE=1` prints every Dokan callback with its arguments and result.
- The issue #1 case study in `docs/case-studies/issue-1`: requirements, research, design and the evidence for each requirement.
