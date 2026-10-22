# Every recorded GitHub idea, and what LFS does with it

This file answers the review request on PR #2: "Double check that all ideas
we ever recorded on github about file system and database with embedded
files are collected". The collection is in
[`data/github-ideas-raw.md`](data/github-ideas-raw.md). It lists 154
primary items found by issue, discussion, code, wiki and gist searches over
`link-foundation`, `linksplatform`, `konard`, `deep-foundation` and
`link-assistant`, and in full dumps of 124 repositories. The method, the
search counts and the known gaps are recorded there. The machine-readable
search results are next to it in `data/*.json`.

Below, each item is matched to what LFS does about it. The numbers are the
item numbers in the raw file. A status is one of these:

- **Done**: LFS implements it. The evidence column names the code, the
  command or the test that shows it.
- **Partly**: LFS covers part of it. The column says which part is missing.
- **Not done**: the idea is about storage, but it is outside issue #1, or
  it is a different product. It is listed so that nothing gets lost.
- **N/A**: the idea is not about a file system or a store (an application,
  a UI or a research question). It is kept for completeness.

Tests are named by function. `src/fs/tests.rs`, `src/links/tests.rs` and
`tests/integration/*.rs` hold most of them. `cargo test --all-features`
runs all of them on Linux, macOS and Windows in CI.

## Summary

| Theme | Items | Done | Partly | Not done | N/A |
|---|---:|---:|---:|---:|---:|
| File system on links | 18 | 10 | 4 | 2 | 2 |
| File contents and raw data | 17 | 4 | 2 | 5 | 6 |
| Indexes | 18 | 8 | 3 | 6 | 1 |
| Journal and transactions | 27 | 10 | 5 | 11 | 1 |
| Memory and storage layout | 47 | 26 | 8 | 8 | 5 |
| Drivers and OS integration | 7 | 0 | 4 | 0 | 3 |
| Visualization | 3 | 2 | 1 | 0 | 0 |
| Other storage | 17 | 7 | 2 | 1 | 7 |
| **Total** | **154** | **67** | **29** | **33** | **25** |

Every idea that issue #1 asks for directly is **Done**: the database is the
file system (1, 4, 5); files are raw block ranges inside the links store,
and the gaps are filled with links (2, 7); there is no fragmentation (7);
there are bitstring and tree indexes and an adaptive switch between them
(37, 39–41, 43, 44); the layouts are united and split (53, 81, 105); there is a
journal with atomic commits (54, 56, 77); `fsck` exists (55, 118); raw
drives and image files both work (5, 113); mount drivers exist for Linux,
macOS and Windows; and there is a disk-space map down to single bytes
(121, 137).

The **Not done** ideas are features of a different scope: version
history and time travel (13, 64, 74), multi-writer isolation (57, 69),
compression (24, 25), distributed storage (67, 114) and address widths
below 64 bits (82). None of them is required by issue #1. Each would be a
format or product extension of its own.

## File system on links (1–18)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 1 | LinksPlatform#12 | Raw files in the links DB as block ranges. A whole-drive DB becomes a file system. | Done | Files are block extents owned by links (`src/fs/file.rs`). `lfs mkfs /dev/sdX` formats a whole drive. Tests: `files_round_trip_in_every_layout`, `files_round_trip_between_the_host_and_a_volume`. |
| 2 | LinksPlatform#12 (2016 comment) | A file of unknown length is appended at the end. Gaps between files are filled with links, never with smaller files. | Done | A growing file stays one extent and moves to the end when it must (`every_file_stays_one_extent_while_growing`). Link chunks fill the gaps (`gap-fill on`, `best_fit_fills_the_smallest_gap`, `chunks_are_placed_at_commit_and_reload`). |
| 3 | LinksPlatform#12 (2020 comment) | Two bitstrings map the used space (links and raw blocks), or a tree of ranges is used. | Done | There is no bitmap on disk. `Space` (`src/space.rs`) is rebuilt at mount from the block owners and kept as range lists. Test: `space_map_covers_every_block_once`. This is the range-tree alternative, and it has no on-disk state that could disagree with the owners. |
| 4 | LinksPlatform#216 | Links File System: a DB and an FS in one file or one drive, good for small files, with deduplication. | Partly | The DB and the FS are one volume. Names and symlink targets are links, so they take no data blocks, and every name is stored once (`names_share_suffixes_and_are_released`). File *contents* are not deduplicated: each file keeps its own extent. |
| 5 | LinksPlatform#213 | Fixed-size links storage on a whole drive, with no host FS. | Done | `FileDevice` opens block devices as well as files (`src/device.rs`). The volume has a fixed size and changes only on `lfs resize`. |
| 6 | Data.Doublets#508 | Treat references as binary data, so the store can be an FS or an archive. | Done | Raw file data lives beside the links, and references to it are external values (`src/links/mod.rs`, the `Platform.Data.Hybrid` encoding). |
| 7 | Data.Doublets#512 | Free ranges, not single links. Binary ranges have a marker and a length. Gaps are filled with links. No fragmentation: anything too big goes at the end. | Done | One extent per file. The free space is a range list. Link chunks fill gaps. Online compaction (`lfs compact`, `auto-compact`) closes gaps. Tests: `compaction_merges_free_space`, `a_large_file_fits_after_automatic_compaction`, `compaction_closes_gaps_too_small_for_the_files_after_them`. |
| 8 | LinksPlatform#176 | Tag FS: one file, many paths and tags. | Partly | Hard links give one file many paths (`hard_links_and_open_unlinked_files`). User links in the same store can tag inodes (`user_links_share_the_volume_and_survive_remounts`). There is no tag-query view in the mount. |
| 9 | deep documentation#27 | The same link in two parent folders. | Done | Hard links: one inode in several directories, with `nlink` counted (`hard_links_and_open_unlinked_files`). |
| 10 | LinksPlatform#641 | Paths as a recursive links structure. | Partly | Directories are a links tree, and names are suffix-shared link chains. A path is resolved by walking it, not stored as one recursive sequence. |
| 11 | LinksPlatform#699 | File metadata in a semantic-network DB, also for network file systems. | Done | All metadata (inodes, names, entries) is links of the embedded store, and an NFS server exposes it (`src/driver/nfs.rs`, `an_nfs_client_creates_reads_lists_renames_and_removes`). |
| 12 | LinksPlatform wiki, idea 11 | The same idea as 11, in the wiki. | Done | As item 11. |
| 13 | LinksPlatform#484 | Only changes are recorded. Recovery to any point in time. | Not done | LFS keeps the last committed state. The journal only makes each commit atomic. There is no version history. |
| 14 | LinksPlatform#539 | All text files in one DB, with per-file version control. | Partly | All files are in one volume. There is no per-file version history. |
| 15 | LinksPlatform#540 | Code storage with git import and export. | Not done | Out of scope. `lfs put` and `lfs get` copy whole trees in and out. |
| 16 | LinksPlatform#138 | A file-system importer into links. | Done | `lfs put HOST_DIR IMAGE /path` imports a whole host tree (`files_round_trip_between_the_host_and_a_volume`). |
| 17 | LinksPlatform#609 | An FS snapshot stored in Doublets as an intermediate store. | N/A | A translator architecture. |
| 18 | LinksPlatform#669 | Version control over classes or functions. | N/A | A VCS design. |

## File contents as links, sequences and raw data (19–35)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 19 | LinksPlatform#675 | Links plus binary data between them, with optional indexes. | Done | `--layout data-only` keeps only the link records on disk. The indexes are rebuilt in RAM at mount (`rebuild_ram` in `src/links/mod.rs`). The usage index is optional (`index off`). Test: `files_round_trip_in_every_layout`. |
| 20 | Data.Doublets#207 | Convert `byte[]` to sequences and back. | Not done | File bytes stay raw blocks, not link sequences. That choice keeps reads at disk speed (see README §4). |
| 21 | Data.Doublets#201 | Bit-by-bit addressing of sequences. | Not done | Research on sequences. |
| 22 | LinksPlatform#555 | Logs as deduplicated doublets. | N/A | An application. |
| 23 | LinksPlatform#304 | A fragment-id service against duplication. | Not done | No content deduplication. |
| 24 | LinksPlatform#182 | Index pi to encode content (pifs). | Not done | Research. |
| 25 | LinksPlatform#95 | Compression with a global dictionary. | Not done | No compression. |
| 26 | Data.Doublets#97 | Deduplicate sequences on close. | Partly | Names are deduplicated as they are written (suffix sharing). There are no other sequences. |
| 27 | LinksPlatform#567 | Variable-length fragments in the same file as the links. | Done | The variable-size store (`src/varstore.rs`) keeps trees, bit strings and large records in the volume (`large_records_span_several_blocks`, `objects_store_resize_and_free`). |
| 28 | LinksPlatform#205 | Map any binary file onto a schema and diff it. | Partly | For LFS images, `lfs blueprint` maps every byte onto named fields (`the_blueprint_partitions_every_byte_of_every_configuration`). Other files and the diff are not covered. |
| 29 | LinksPlatform#549 | Strings as sequences of links. | Done | File names are links-notation strings: `(STRING chain)`, with a char or a packed encoding (`src/fs/names.rs`, `encodings_round_trip`). |
| 30 | LinksPlatform#561 | i18n data in one links file. | N/A | An application. |
| 31 | deeplinks#257 | Deep packager files as base64 values. | N/A | Deep. |
| 32 | deeplinks#296 | URIs for AsyncFile values. | N/A | Deep. |
| 33 | deepcase#25 | Binary files become AsyncFile links; text files become SyncTextFile links. | N/A | A Deep UI feature. In LFS every file is an inode link with raw content, whatever its type. |
| 34 | deeplinks#142 | File as a value kind of links. | N/A | A Deep type-system feature. The LFS analog is an inode link that owns its content extent. |
| 35 | meta-expression#63 | A durable doublets-backed store with Unicode strings as character links. | Done | Names with `--names chars` are one link per code point, and they persist (`names_share_suffixes_and_are_released`, `encodings_round_trip`). |

## Indexes (36–53)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 36 | LinksPlatform#5 | Test other index forms (bitstring, trees, lists, ...). | Partly | Size-balanced trees, bit strings and an adaptive mix are implemented and measured (`index_modes_switch_at_runtime`). Skip lists, matrices and hash tables are not. |
| 37 | LinksPlatform#566 | An extra indexing layer of bit strings. | Done | `index bitstring` (`src/links/usage.rs`). |
| 38 | LinksPlatform#663 | Hierarchical bit strings (a summary bitmap). | Not done | Bit strings are flat. |
| 39 | Data.Doublets#185 | Bit-string indexes per link, intersected. | Done | Usage sets per link and per role are bit strings in bitstring or adaptive mode (`usage_sets_show_tree_nodes_and_bits`). |
| 40 | Data.Doublets#240 | A BitString index decorator kept in sync on every CRUD operation, to compare with trees. | Done | The usage index is updated on create, update and delete. `random_operations_match_a_model` checks it against a model. |
| 41 | Data.Doublets#396 | Bit strings as an alternative or an addition to trees. | Done | `index tree|bitstring|adaptive`. |
| 42 | Data.Doublets#398 | A bit-string map of deleted links. | Not done | Free links are a list (united layout) or an in-RAM set rebuilt at mount (split and data-only layouts). |
| 43 | Data.Doublets#395 | Optional tree indexes. | Done | `index off`, and `--layout data-only` (no on-disk tree). |
| 44 | Data.Doublets#239 | A basic index that can be switched. | Done | `lfs config IMAGE index ...` at run time (`index_modes_switch_at_runtime`). |
| 45 | LinksPlatform#15 | Switch between list and tree with a marker in the file. | Done | The layout is in the superblock, and the index mode is in `/.lfs/config`. A volume is never opened with the wrong one. |
| 46 | LinksPlatform#522 | Per-N-links subtrees. | Not done | |
| 47 | LinksPlatform#526 | Two matrices as indexes. | Not done | |
| 48 | LinksPlatform#313 | Bit-by-bit sequence indexing. | Not done | |
| 49 | LinksPlatform#327 | Deep-dependency sets. | Not done | |
| 50 | LinksPlatform#307 | Benchmark HashSet against BitString intersection. | Partly | The adaptive threshold is the measured break-even density, 1/192 (`auto_threshold_is_the_break_even_density`). HashSet is not compared. |
| 51 | Collections#57 | Chained BitString operations. | N/A | A collections library. |
| 52 | deeplinks#28 | Dynamic indexes from usage statistics. | Partly | Adaptive mode picks tree or bit string per set from its density, with hysteresis (`adaptive_index_switches_to_bitstrings`). It uses data density, not query statistics. |
| 53 | LinksPlatform#4 | Store values apart from the index (the origin of split). | Done | `--layout split` (`split_layout_with_large_blocks`). |

## Journal, transactions, recovery and versioning (54–80)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 54 | LinksPlatform#37 | A transaction log for durability. | Done | A redo journal of blocks (`src/journal.rs`, `journal metadata|full`). It logs block images, not link transitions. |
| 55 | LinksPlatform#39 | Check consistency and recover. | Done | `lfs fsck` checks every invariant. Mount replays the journal. Tests: `crash_everywhere`, `crashes_never_corrupt_metadata`, `corrupted_images_never_hang`. |
| 56 | LinksPlatform#40 | Atomic transactions. | Done | One commit is all or nothing (`full_journal_makes_overwrites_atomic`, `a_torn_transaction_is_ignored`). |
| 57 | LinksPlatform#8 | Isolation and multi-threaded transactions. | Not done | One writer per volume, now enforced by an image lock (`one_writer_or_many_readers_open_an_image`). Drivers serialize the calls. |
| 58 | LinksPlatform#46 | Push a snapshot to the log. | Not done | |
| 59 | LinksPlatform#47 | Compact the log. | N/A | LFS keeps no history log to compact: the journal is circular and is emptied at every checkpoint. |
| 60 | LinksPlatform#45 | Log to a GEXF dynamic graph. | Not done | |
| 61 | LinksPlatform#101 | Undo and revert transactions. | Not done | |
| 62 | LinksPlatform#103 | Log timestamps are in the past. | Partly | Journal records carry a sequence number and a CRC (`crc32_matches_reference_vector`). Old or torn records are ignored. There are no timestamps. |
| 63 | LinksPlatform#133 | The log as a lazy event loop. | Not done | |
| 64 | LinksPlatform#466 | A create-only mode with full history. | Not done | |
| 65 | LinksPlatform#474 | The DB is a log, and the snapshot lives in memory. | Not done | LFS is snapshot plus journal. |
| 66 | LinksPlatform#603 | Copy-on-write, with a pointer swap after commit. | Done | `journal off` uses shadow paging: dirty chunks move to fresh blocks and the superblock is the only in-place write (`src/fs/commit.rs`, `volumes_grow_and_shrink_in_every_journal_mode`). |
| 67 | LinksPlatform#594 | Distributed log storage. | Not done | |
| 68 | LinksPlatform#647 | LinksChain (hash chains). | Not done | Only CRCs on the superblock and the journal. |
| 69 | LinksPlatform#435 | Per-link locking. | Not done | Whole-volume lock. |
| 70 | Data.Doublets#243 | An in-memory transaction layer. | Partly | Changes collect in memory until a commit (`commit-every`). There is no rollback API. |
| 71 | Data.Doublets#250 | Snapshot plus change sets, merged on read. | Partly | Reads see the dirty cache over the committed blocks. |
| 72 | Data.Doublets#208 | Automatic recovery by rebuilding indexes. | Done | Free space is rebuilt from the owners. The data-only indexes are rebuilt in RAM. Orphans are removed at mount (`orphans_are_removed_at_mount`). |
| 73 | Data.Doublets#206 | Shared access by several processes. | Partly | An exclusive lock for a writer, a shared lock for readers (flock on Unix, LockFileEx on Windows). A second writer gets "in use" (`one_writer_or_many_readers_open_an_image`, `a_volume_in_use_refuses_other_processes`). Several writers at once are not supported; NFS (`lfs serve`) is the way to share one volume. |
| 74 | link-cli#94 | Reversible transactions and a version-control decorator. | Not done | |
| 75 | link-cli#98 | A reusable doublets-backed transactional store. | Done | The `links-fs` library exposes the links store with atomic commits (`Fs::links`, `user_links_share_the_volume_and_survive_remounts`). |
| 76 | link-cli#129 | An atomic import. | Done | `lfs import-links` checks the whole input before changing anything and commits once. A bad last line leaves the volume as it was (`the_links_database_is_usable_from_the_command_line`). |
| 77 | doublets-rs#61 | A durable log with replay. | Done | Mount replays the newest complete transaction (`a_torn_transaction_is_ignored`, `crash_everywhere`). |
| 78 | Crawler#10 | A log that limits the damage. | Done | As items 54 and 56. |
| 79 | molt#43 | A crash-consistent FS: banked journal, generation commits. | Done (own design) | Superblock sequence numbers plus a journal with CRCs (`superblock_round_trips_and_detects_damage`). |
| 80 | formal-ai associative stack | A transactions decorator, an fsynced log and a readable `.lino` projection. | Partly | Commits are atomic and fsynced. `/.lfs/links.lino` is a read-only LiNo projection. There is no rollback API. |

## Memory and storage layout (81–127)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 81 | LinksPlatform#74 | All fields in one file, or one file per field. | Done | `--layout united|split|data-only`. `lfs demo` builds each one, and `lfs blueprint` compares them byte by byte. |
| 82 | LinksPlatform#48 | Address widths of 8, 16 and 32 bits. | Not done | Links are 64-bit (`UnitedMemoryLinks<u64>`). A narrower width would be a new format version. |
| 83 | LinksPlatform#208 | Free memory as a list of ranges. | Done | `Space` keeps ranges by start and by length (`src/space.rs`). |
| 84 | LinksPlatform#207 | Create a link at any chosen address. | Partly | Blocks can be taken at a chosen place (`allocate_at_and_avoid`, `relocation_moves_a_chunk_out_of_the_way`). New links take the next free index; they cannot be placed at a chosen index. |
| 85 | LinksPlatform#209 | A configurable number of link parts. | N/A | LFS stores doublets, as issue #1 asks. |
| 86 | LinksPlatform#219 | Pointers or indices. | N/A | LFS uses indices everywhere, because pointers cannot be stored on disk. |
| 87 | LinksPlatform#22 | Startup information in the first link. | Done | The superblock (block 0) holds the links header, the geometry and the roots (`superblock_round_trips_and_detects_damage`). |
| 88 | LinksPlatform#465 | Virtual point links. | Not done | |
| 89 | LinksPlatform#475 | A compact snapshot with small addresses. | Partly | Export formats: binary links notation, doublets and LiNo (`every_format_round_trips_every_layout`). They have no variable-width addresses. |
| 90 | LinksPlatform#591 | An append-only editable array. | Not done | |
| 91 | LinksPlatform#664 | Sectorized lists of page-sized blocks. | Done | Link records, inodes and the variable store live in block-sized chunks (`src/chunked.rs`). |
| 92 | LinksPlatform#430 | LinksOptions and LinksFactory. | Done | `FormatOptions` (`lfs mkfs` flags) plus `RuntimeConfig` (12 keys). |
| 93 | LinksPlatform#187 | Flexibility and performance at once. | Partly | Settings change at run time without a rebuild. There are no precompiled specialised variants. |
| 94 | LinksPlatform#17 | Out of disk space. | Done | `ENOSPC` with the volume clean (`a_full_volume_reports_no_space`). |
| 95 | LinksPlatform#36 | Pack and unpack (backup). | Partly | `lfs compact` packs the space, `export-links` and `import-links` move the links, and `lfs get /` copies all files. There is no single backup command. |
| 96 | LinksPlatform#29 | A binary links protocol with variable-length numbers. | Done | The binary links notation archive (`the_golden_archive_matches_link_cli`). |
| 97 | LinksPlatform#194 | A universal links string format. | Done | LiNo export and import (`every_format_round_trips_every_layout`, `/.lfs/links.lino`). |
| 98 | LinksPlatform#320 | A binary sequences file format. | Not done | |
| 99 | LinksPlatform#645 | Mutable data in RAM, immutable data on disk. | Not done | |
| 100 | LinksPlatform#599 | Tables as virtual links. | N/A | |
| 101 | LinksPlatform#85 | Virtual or computed links. | Partly | `/.lfs/*` files are computed from the volume (`control_files`). |
| 102 | LinksPlatform#545 | Base data and optional data. | Done | Base: the link records. Optional: the usage index (`index off`). |
| 103 | LinksPlatform#199 | Managed-memory-only backend. | Done | `MemDevice`, in RAM (it is used by the WASM page and the tests). |
| 104 | LinksPlatform#7 | Bare metal; reconsider mmap. | Partly | All I/O goes through `BlockDevice` reads and writes, never mmap, and the library builds for `wasm32-unknown-unknown`. It still needs `std`, so it does not run on BareMetal OS. |
| 105 | Data.Doublets#179 | A header in split memory, so a lost index can be rebuilt. | Done | The superblock records the layout and the links header. The data-only layout rebuilds its indexes at every mount. |
| 106 | Data.Doublets#172 | Optional gap filling. | Done | `gap-fill on|off`. |
| 107 | Data.Doublets#131 | Preallocate space. | Done | `growth-percent` reserves room at the end of growing extents. The volume size is fixed at `mkfs`. |
| 108 | Data.Doublets#372 | Resize in link units. | Partly | `lfs resize` changes the size in blocks, in one commit (`resize_grows_and_shrinks_an_image`, `a_crash_while_resizing_keeps_the_old_or_the_new_size`). Link chunks grow block by block. Sizes are not given in links. |
| 109 | Data.Doublets#356 | Header metadata in `size_type`. | N/A | |
| 110 | Data.Doublets#317 | Do not shrink files on their own. | Done | Images shrink only on `lfs resize --size`. |
| 111 | Data.Doublets#155 | External references by default. | Done | Every value that is not a link index is an external reference (Hybrid encoding). |
| 112 | Data.Doublets#168 | One key field per split tree. | Done | The split layout keeps a tree per role, sources and targets, and per-link trees for internal ends (`tree_for` in `src/links/mod.rs`). |
| 113 | Memory#48 | Non-resizable memory over the whole drive. | Done | Item 5. |
| 114 | Memory#21 | Memory spread over drives. | Not done | |
| 115 | Memory#11 | A virtual-memory backend. | Not done | Not needed: no mmap. |
| 116 | Memory#24 | Growth based on the page size. | Done | The block size is chosen at `mkfs`, and chunks grow in blocks. |
| 117 | Data#77 | A section per N links, one thread each. | Not done | |
| 118 | Triplets.Kernel#11 | A consistency check of holes and links. | Done | `fsck` checks free slots, owners, names and counts (`every_failure_is_reported_not_just_the_first`). |
| 119 | mem-rs#36 | File-mapped growth wipes a reopened store. | Done (avoided) | LFS does not use mem-rs. Reopening is tested everywhere. `doublets_file_mapped_stores_do_not_survive_a_reopen` records the upstream bug. |
| 120 | mem-rs#26 | Async memory access. | N/A | |
| 121 | doublets-rs#69 | An exact memory layout of Link. | Done | Every on-disk field has a fixed little-endian offset, and `lfs blueprint` shows each byte (`the_superblock_and_journal_are_dissected_field_by_field`, `link_records_show_the_ends_of_every_live_link`). |
| 122 | SQLiteVSDoublets#29 | The split DB size is computed correctly. | Done | `lfs map` and `lfs blueprint` account for every block and byte (`space_map_covers_every_block_once`). |
| 123 | LinksPlatform wiki Home | Storage size by link count. | Partly | `lfs demo` builds a volume per configuration, and `lfs blueprint` shows the bytes of each. There is no size table by link count. |
| 124 | gist 9c8115c4 | A resizable in-memory store. | Done | `MemDevice`. |
| 125 | router#357 | Keep one long-lived store per process. | Done | Mount and serve hold one `Fs` for the session. |
| 126 | router#557 | A full store must not panic. | Done | `NoSpace` errors (item 94), `lfs resize`, and `a_panic_closes_the_volume_without_committing`. |
| 127 | router#374 | Format versioning: migrate or refuse. | Done | `superblock::VERSION`. Any other version is refused at mount. Version 1 is the only one, so there is nothing to migrate yet. |

## Drivers, OS and host file-system integration (128–134)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 128 | LinksPlatform#215 | Links OS. | N/A | |
| 129 | deep-packages#31 | Two-way sync of Deep links with a folder in any OS. | Partly | An LFS volume is a folder in any OS: FUSE on Linux and macOS, Dokan on Windows, NFS on macOS and Linux. There is no Deep sync. |
| 130 | deep-packages#114 | Sync one link with one file. | Partly | Each file is a link in the volume. There is no sync to a separate host file. |
| 131 | deep-packages#30 | Deep and VS Code sync. | Partly | A mounted volume can be opened in any editor. |
| 132 | deep README | File-system watch and sync. | Partly | Mount drivers. There is no watcher. |
| 133 | hasura#13 | A storage location per file. | N/A | |
| 134 | LinksPlatform#601 | A shadow-mode replica. | N/A | |

## Visualization (135–137)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 135 | LinksPlatform#617 | Visualize memory page access. | Partly | The visualizer and the blueprint show the layout of every block and byte. Access patterns are not recorded. |
| 136 | links-visuals#23 | A grid of links in the blueprint style. | Done | The blueprint page's hex grid colors each byte by its field (`the_blueprint_page_styles_every_kind_and_runs_lfs`). |
| 137 | pngfs-priv#2 | An FS core in WASM on GitHub Pages. | Done | `docs/visualizer/blueprint.html` runs LFS as WebAssembly to format, open and walk any configuration in the browser. |

## Other storage ideas (138–154)

| # | Source | Idea | Status | Evidence in LFS |
|---:|---|---|---|---|
| 138 | start#42 | Dual storage: a binary DB plus LiNo text. | Done | The binary store plus `/.lfs/links.lino` and `export-links --format lino`. |
| 139 | vietnam-accomodation-search#4 | A transactional binary mirror of LiNo. | Done | Item 138, with atomic commits. |
| 140 | links-logs-vault#1 | Text and binary stores. | Done | Item 138. |
| 141 | links-queue#13 | Pluggable storage backends. | Done | The `BlockDevice` trait: `FileDevice`, `MemDevice` and the crash-test device. |
| 142 | links-notation#325 | One binary links notation. | Done | The archive matches link-cli byte for byte (`the_golden_archive_matches_link_cli`). |
| 143 | meta-language#54 | A doublets backend behind a trait. | Done | Item 141. The links store reads and writes doublets files (`doublets_reads_and_searches_our_files`). |
| 144 | meta-language#14 | Persistent shared snapshots. | Not done | |
| 145 | meta-sovereign#6 | Backups and encryption. | Partly | Backups: image copy and `export-links`. There is no encryption (use dm-crypt, BitLocker or FileVault under the image). |
| 146 | LinksPlatform#227 | Own your data. | N/A | |
| 147 | LinksPlatform#303 | Global data version control. | N/A | |
| 148 | LinksPlatform#703 | Video metadata in links. | N/A | |
| 149 | LinksPlatform#700 | An editor with a meta layer. | N/A | |
| 150 | cbr-key-rate-monitor#4 | Doublets as an app DB. | N/A | Another application. The `links-fs` library could serve it. |
| 151 | router#148 | A binary doublets store plus LiNo export. | Done | Item 138. |
| 152 | router#235 | All state as LiNo next to the binary store, with lock files. | Partly | LiNo projection plus image lock. Applications are out of scope. |
| 153 | CorrelationCenterConcept#5 | Storage on the Links DB. | N/A | |
| 154 | deep-packages#69 | Load doublets into Deep. | N/A | A Deep package. LFS exports doublets files that such a loader could read (`every_format_exports_the_whole_store`). |

## Gaps that remain

- **Counted by status:** 67 items are Done and 29 are Partly. The Partly
  items lack something that issue #1 does not ask for: version history,
  content deduplication, a tag-query view, Deep sync or encryption.
- **Not done (33 items):** these are listed above with a reason. The most
  likely next steps for LFS are:
  - snapshots or time travel (13, 64, 74, 144);
  - content deduplication (4, 23);
  - narrower addresses (82);
  - multi-writer access (57, 69, 73).
- **The search has known limits**, recorded in the raw file:
  - some repositories have issues disabled;
  - one referenced issue (`deep-foundation/dev#125`) was deleted;
  - gist pages 121–989 were only spot-checked.
