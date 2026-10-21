---
bump: minor
---

### Added
- LFS, the links file system: a block-device file system in Rust whose metadata is an embedded doublets links database (united or split indices, optional adaptive bitstring index, variable-size store with a threshold) and that shares one optional journal with it.
- Files take one contiguous extent each; gaps between files are filled with link chunks, and online compaction keeps the free space in one extent.
- Settings that can change at run time, stored in `/.lfs/config` and set with `lfs config`.
- Links import and export: binary links notation, doublets files and LiNo.
- The `lfs` command line tool: `mkfs`, `fsck`, `map`, `compact`, `put`, `get`, `ls`, `links`, `config` and `mount`.
- Mount drivers: FUSE on Linux, FreeBSD and macOS (macFUSE), and Dokan 2 on Windows x64.
- A disk-space map visualizer on GitHub Pages.
