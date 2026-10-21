# Research: file systems, double journaling, user-space FS frameworks, bitstring indexes

Background research for [issue #1 "Links file system driver for Linux, macOS, Windows"](https://github.com/link-foundation/lfs/issues/1).
Facts were collected on 2026-10-21. Every claim links to its source. Crate versions and licenses
come from the crates.io API (`https://crates.io/api/v1/crates/<name>`) on that date.

Contents:

1. [ext4 and other on-disk designs; cross-platform portability](#1-ext4-and-other-on-disk-designs-cross-platform-portability)
2. [Double journaling and file systems that embed databases](#2-double-journaling-and-file-systems-that-embed-databases)
3. [User-space file system frameworks and Rust crates (per OS, plus CI)](#3-user-space-file-system-frameworks-and-rust-crates)
4. [Indexing: bitstrings/bitmaps vs trees, break-even analysis](#4-indexing-bitstrings-vs-trees)
5. [Existing Rust file system implementations to use as references](#5-existing-rust-file-system-implementations)
6. [Prior art for visualising disk maps](#6-visualisation-prior-art-for-disk-maps)
7. [Implications for LFS](#implications-for-lfs)

---

## 1. ext4 and other on-disk designs; cross-platform portability

### 1.1 How ext4 keeps fragmentation low

Primary source: the Linux kernel ext4 on-disk format documentation,
<https://docs.kernel.org/filesystems/ext4/index.html> (rst sources in
[`Documentation/filesystems/ext4/`](https://github.com/torvalds/linux/tree/master/Documentation/filesystems/ext4)).

| Technique | What it does | Source |
|---|---|---|
| **Extents / extent tree** | ext3's 1:1 indirect block map is replaced by `(logical block, length, physical block)` records. A contiguous run of 1,000 blocks becomes one `struct ext4_extent` with `ee_len = 1000`. The tree's root sits in the inode's 60-byte `i_block`, which holds a 12-byte header plus **4 extents** with no extra metadata blocks. Interior nodes hold `ext4_extent_idx` entries. Every node has a header with magic `0xF30A`, `eh_entries`, `eh_max` and `eh_depth`. | [ifork.rst](https://docs.kernel.org/filesystems/ext4/ifork.html) |
| **Max extent length** | `ee_len` is 16 bits. A value ≤ 32768 means an initialised extent; a value > 32768 marks an *uninitialised* (preallocated, reads as zeros) extent of length `ee_len - 32768`. So an initialised extent is at most **32768 blocks = 128 MiB with 4 KiB blocks**, and an uninitialised one at most 32767 blocks. | [ifork.rst](https://docs.kernel.org/filesystems/ext4/ifork.html) |
| **Block groups** | The disk is cut into 128 MiB groups (one 4 KiB block bitmap = 32768 bits = 32768 × 4 KiB). Each group has a block bitmap, an inode bitmap and an inode table, so ext4's free-space index is a **bitstring**. | [blockgroup.rst](https://docs.kernel.org/filesystems/ext4/blockgroup.html), [allocators.rst](https://docs.kernel.org/filesystems/ext4/allocators.html) |
| **flex_bg** | 2^`s_log_groups_per_flex` groups are merged into one logical group, and their bitmaps and inode tables are packed together at the start. Metadata loads faster, and data runs no longer stop at every 128 MiB group, so a large file can be one long run of extents. | [blockgroup.rst](https://docs.kernel.org/filesystems/ext4/blockgroup.html) |
| **Multi-block allocator (mballoc)** | Allocates many blocks per request (and speculatively 8 KiB on create). A **per-inode preallocation** list keeps a file contiguous. Small files (< `mb_stream_req`, default **16 blocks**) instead use **per-CPU locality-group preallocation**, normalised to `mb_group_prealloc` (default **512 blocks**), so small files sit together. Free space is searched with a **buddy bitmap** cache, which is in memory only and rebuilt from the on-disk bitmaps. With `mb_optimize_scan`, groups are indexed by *largest free order* and *average fragment size* to find a fitting group in O(1). | [allocators.rst](https://docs.kernel.org/filesystems/ext4/allocators.html), [fs/ext4/mballoc.c header comment](https://github.com/torvalds/linux/blob/master/fs/ext4/mballoc.c) |
| **Delayed allocation** | Physical placement waits until writeback (commit timeout, `sync()` or memory pressure). By then the final size is known, so the allocator can request one big extent. | [allocators.rst](https://docs.kernel.org/filesystems/ext4/allocators.html) |
| **Locality heuristics** | Data goes in the same group as its inode, and inodes in the same group as their directory. Top-level directories are spread across the least-loaded groups. | [allocators.rst](https://docs.kernel.org/filesystems/ext4/allocators.html) |
| **HTree directories** | A directory with flag `EXT4_INDEX_FL` (0x1000) is a hashed B-tree keyed by a hash of the name. Interior nodes hide inside "empty" dirents spanning a whole block, which keeps the format read-compatible with ext2. | [directory.rst](https://docs.kernel.org/filesystems/ext4/directory.html) |
| **Inline data** | Files under 60 bytes live in `i_block`. Slightly larger ones go into the in-inode xattr `system.data` (about 160 bytes in a 256-byte inode). Tiny directories can be inline too. This avoids allocating a 4 KiB block for tiny files, which is the main source of internal fragmentation. | [inlinedata.rst](https://docs.kernel.org/filesystems/ext4/inlinedata.html) |
| **bigalloc** | Allocation unit = cluster of 2^n blocks. Fewer bitmap bits are needed, at the cost of more slack. | [bigalloc.rst](https://docs.kernel.org/filesystems/ext4/bigalloc.html) |
| **JBD2 journal** | Transaction = descriptor block + data blocks (or revoke block) + commit block. Transactions with no commit record or a bad checksum are discarded on replay. The journal is normally hidden inode 8, sized up to 10,240,000 blocks and placed mid-disk. It is big-endian, unlike ext4. Modes: **`data=ordered`** (default; only metadata is journaled, and data is flushed before the metadata commits), **`data=journal`** (data and metadata both go through the journal, so everything is written twice; safest and slowest), **`data=writeback`** (metadata only, with no ordering of data). **fast_commit** logs only minimal metadata deltas between full commits. | [journal.rst](https://docs.kernel.org/filesystems/ext4/journal.html) |
| **Metadata checksums** | `metadata_csum` (2012): crc32c over all major ext4 and jbd2 structures. 16-bit fields are used where there is no room, and the `64bit` feature gives full 32-bit checksums. | [checksums.rst](https://docs.kernel.org/filesystems/ext4/checksums.html) |
| **Online defrag** | `e4defrag` moves a file's blocks into a new contiguous donor file through the `EXT4_IOC_MOVE_EXT` ioctl while the file system is mounted. `e4defrag -c` reports a fragmentation score. | [e4defrag(8)](https://man7.org/linux/man-pages/man8/e4defrag.8.html), [allocators.rst: "if all of these mechanisms fail, one can always use e4defrag"](https://docs.kernel.org/filesystems/ext4/allocators.html) |

**What fragmentation remains.** These are heuristics, not guarantees:

- Free space fragments as the volume ages and fills up. `e2freefrag` exists to measure exactly that ([e2freefrag(8)](https://man7.org/linux/man-pages/man8/e2freefrag.8.html)).
- Files that grow slowly in parallel (logs, databases) interleave with each other.
- Small files created in a random order end up scattered.
- Conway et al., FAST'17, *"File Systems Fated for Senescence? Nonsense, Says Science!"* ([USENIX](https://www.usenix.org/conference/fast17/technical-sessions/presentation/conway)):
  - On ext4 and ZFS, a few hundred `git pull`s can halve read performance, and 1,000 pulls can make it up to 30× slower.
  - Changing only the creation order of a few thousand small files slows reads 15–175×, depending on the file system.
  - BetrFS, which is built on write-optimised dictionaries (Bε-trees), showed almost no aging.

So "no fragmentation allowed" needs a stronger mechanism than ext4's heuristics: compaction or relocation, or a layout where data always moves to keep runs contiguous.

### 1.2 Log-structured and copy-on-write approaches

| FS | Key idea | Relevance to LFS | Source |
|---|---|---|---|
| **btrfs** | COW B-trees for all metadata, checksums, snapshots, reflinks. Its free-space tree stores free space as extent items and **switches to bitmaps when extents would take more space than bitmaps**, with a 100-entry hysteresis between the high and low thresholds. | Kernel precedent for the tree↔bitstring switch the issue asks for. | [btrfs docs](https://btrfs.readthedocs.io/), [`fs/btrfs/free-space-tree.c` `btrfs_set_free_space_tree_thresholds`](https://github.com/torvalds/linux/blob/master/fs/btrfs/free-space-tree.c) |
| **ZFS (OpenZFS)** | Transactional COW object store (DMU), Merkle-tree checksums, ZIL intent log for sync writes, variable recordsize. Because writes are COW, partial page writes cannot occur when recordsize matches the DB page size, so DB doublewrite buffers can often be turned off. | A COW tree avoids a separate journal; an intent log covers only fsync latency. | [OpenZFS docs](https://openzfs.github.io/openzfs-docs/), [OpenZFS Workload Tuning: InnoDB (`innodb_doublewrite=0`)](https://openzfs.github.io/openzfs-docs/Performance%20and%20Tuning/Workload%20Tuning.html) |
| **APFS** | COW metadata, clones, snapshots, space sharing between volumes in a container. Everything is a B-tree keyed by object id. | Shows that a single object-map plus B-trees can carry all FS metadata. | [Apple File System Reference (PDF)](https://developer.apple.com/support/downloads/Apple-File-System-Reference.pdf) |
| **F2FS** | Log-structured design for flash. **Six active logs** (hot/warm/cold × node/data) separate data by temperature. A **Node Address Table** stops the "wandering tree" problem (updates cascading up the index). Adaptive logging switches to threaded logging depending on FS state. Has atomic-write ioctls that SQLite can use (§2). | Hot/cold separation; using indirection to stop update cascades. | [docs.kernel.org/filesystems/f2fs](https://docs.kernel.org/filesystems/f2fs.html) |
| **NILFS2** | Log-structured, with continuous checkpoints that can be turned into snapshots. | Its journal *is* the file system, so nothing is journaled twice. | [docs.kernel.org/filesystems/nilfs2](https://docs.kernel.org/filesystems/nilfs2.html) |
| **ReFS** | B+-trees for all metadata, allocate-on-write (COW), 64-bit metadata checksums, optional integrity streams for data. | Same "everything is a B+-tree" approach. | [Wikipedia: ReFS](https://en.wikipedia.org/wiki/ReFS), [Microsoft Learn ReFS overview](https://learn.microsoft.com/en-us/windows-server/storage/refs/refs-overview) |
| **BetrFS** | Bε-tree (write-optimised dictionary) as the FS index; resists aging. | Shows that "file system = key-value database" can work well. | [FAST'17 Conway et al.](https://www.usenix.org/conference/fast17/technical-sessions/presentation/conway) |

### 1.3 Cross-platform portability gap

Native support, out of the box, with no third-party drivers:

| FS | Linux | macOS | Windows | Notes / source |
|---|---|---|---|---|
| FAT32 | RW | RW | RW | 4 GiB max file size; no journal, no permissions. [Apple Disk Utility formats](https://support.apple.com/guide/disk-utility/file-system-formats-dsku19ed921c/mac) |
| exFAT | RW (5.4 staging driver, replaced by Samsung's `fs/exfat` in **5.7**) | RW | RW | Spec published by Microsoft in Aug 2019. No journal (only a "TexFAT" option on WinCE), no ACLs. [Phoronix 5.4](https://www.phoronix.com/news/Linux-5.4-exFAT-Is-Coming), [Phoronix 5.7](https://www.phoronix.com/news/New-exFAT-For-Linux-5.7) |
| NTFS | RW (`ntfs3` from Paragon since **5.15**; before that FUSE `ntfs-3g`) | **read-only** | RW | [Phoronix ntfs3 5.15](https://www.phoronix.com/news/NTFS3-For-Linux-5.15), [Wikipedia NTFS](https://en.wikipedia.org/wiki/NTFS) |
| UDF 2.01 | RW (RO for revisions > 2.01) | RW | RW (Vista+) | Needs tricks: block size must equal the device's logical block size, plus a fake whole-disk MBR. [JElchison/format-udf](https://github.com/JElchison/format-udf#udf-os-support) |
| HFS+ | RW only without journal | RW | – | [Wikipedia HFS Plus](https://en.wikipedia.org/wiki/HFS_Plus) |
| APFS | – (third-party, read-only `apfs-fuse`) | RW | – | [Wikipedia APFS](https://en.wikipedia.org/wiki/Apple_File_System) |
| ext4 | RW | – | – (only through WSL2 `wsl --mount`) | [WSL mount disk](https://learn.microsoft.com/en-us/windows/wsl/wsl2-mount-disk) |
| btrfs | RW | – | – (third-party WinBtrfs) | [WinBtrfs](https://github.com/maharmstone/btrfs) |
| ZFS | out-of-tree module (CDDL) | third-party port | third-party port (beta) | [OpenZFS](https://openzfs.org/) |
| ReFS | – | – | RW (some editions) | [ReFS overview](https://learn.microsoft.com/en-us/windows-server/storage/refs/refs-overview) |
| F2FS | RW | – | – | [f2fs docs](https://docs.kernel.org/filesystems/f2fs.html) |

**Gap:** the only formats that read and write natively on all three OSes are FAT32, exFAT and (with tricks) UDF. None of them has journaling, checksums, COW, snapshots, extents-with-allocator heuristics or a database. Every advanced FS (ext4, btrfs, ZFS, APFS, NTFS, ReFS) is native on at most one or two OSes. This confirms the issue's statement. Because no OS ships LFS in-kernel, LFS also needs a user-space driver on each OS (§3).

---

## 2. Double journaling and file systems that embed databases

### 2.1 Databases journal their own writes

| DBMS | Mechanism | Extra writes | Source |
|---|---|---|---|
| SQLite rollback journal / WAL | Rollback mode copies the original pages to `-journal` before overwriting them. WAL mode appends new pages to `-wal`, and a checkpoint later copies them into the DB file, so every page is written twice. | 2× page writes plus FS metadata journaling of the journal file. | [sqlite.org/wal.html](https://www.sqlite.org/wal.html), [atomic commit](https://www.sqlite.org/atomiccommit.html) |
| PostgreSQL WAL + `full_page_writes` | WAL records, plus a **full page image** on the first change of each page after each checkpoint, to recover from torn pages. Turning it off "might lead to either unrecoverable data corruption, or silent data corruption". | WAL plus a full page image per checkpoint interval. | [runtime-config-wal](https://www.postgresql.org/docs/current/runtime-config-wal.html) |
| MySQL InnoDB redo log + **doublewrite buffer** | Pages are first written to the doublewrite area, then to their real place, to survive partial page writes. Percona: disable it only if the FS/device guarantees no partial page writes. A Percona experiment with ext4 `data=journal` and doublewrite off was later recanted after corruption. | Redo plus 2× page writes; ext4 `data=journal` would make it 3–4×. | [MySQL manual: doublewrite buffer](https://dev.mysql.com/doc/refman/8.0/en/innodb-doublewrite-buffer.html), [Percona: InnoDB double write](https://www.percona.com/blog/innodb-double-write/), [Percona update on ext4](https://www.percona.com/blog/update-on-the-innodb-double-write-buffer-and-ext4-transactions/) |

### 2.2 Measured cost of "journaling of journal" (JOJ)

- **Jeong, Lee, Lee, Son, Won, "I/O Stack Optimization for Smartphones", USENIX ATC'13 (Best Paper)** ([page](https://www.usenix.org/conference/atc13/technical-sessions/presentation/jeong), [PDF](https://www.usenix.org/system/files/conference/atc13/atc13-jeong.pdf)):
  - Named the **JOJ anomaly**: ext4 journals SQLite's own journal writes, and each SQLite `fsync()` triggers bulky ext4 journal commits.
  - Fixes: no unnecessary metadata journaling, an external journal, and polling I/O. SQLite inserts/sec went up 130%. F2FS + WAL went from 39 to 157 inserts/sec (+300%).
- **Kim, Nam, Park, Won, "Resolving Journaling of Journal Anomaly in Android I/O: Multi-Version B-tree with Lazy Split", FAST'14** ([page](https://www.usenix.org/conference/fast14/technical-sessions/presentation/kim-wook-hee)):
  - Recovery information is woven *into the DB B-tree itself*, so no separate journal file is needed.
  - Result: 704 inserts/s vs 416 for WAL (+70%) and 55 for TRUNCATE.
- **Shen, Park, Zhu, "Journaling of Journal Is (Almost) Free", FAST'14** ([page](https://www.usenix.org/conference/fast14/technical-sessions/presentation/shen), [PDF](https://www.cs.rochester.edu/u/kshen/papers/fast2014.pdf)):
  - Adding ext4 journaling under SQLite cost up to **73 %** on a Nexus 7 and **58 %** on an SSD desktop.
  - The authors argue the cost comes from the implementation. With one I/O on the synchronous commit path and an *adaptive per-file journaling mode*, it drops to < 6 % (7 % faster on average).
- **Lee et al., "WALDIO: Eliminating the Filesystem Journaling in Resolving the Journaling of Journal Anomaly", USENIX ATC'15** ([page](https://www.usenix.org/conference/atc15/technical-session/presentation/lee_wongun)):
  - SQLite WAL writes go through direct I/O with preallocation, header embedding and group sync, skipping FS journaling.
  - Result: 5.1× WAL insert throughput and 1/6 of the I/O volume.
- **SQLite batch atomic write on F2FS** ([SQLite check-in](https://sqlite.org/src/info/532bbf1f2b1028db), [source](https://www.sqlite.org/src/info/2e0357c2ed309279)):
  - With `SQLITE_ENABLE_BATCH_ATOMIC_WRITE`, SQLite collects a transaction in memory and commits it with `F2FS_IOC_START_ATOMIC_WRITE`/`COMMIT_ATOMIC_WRITE`, with no rollback journal at all.
  - This is the "FS offers transactions to the DB" approach LFS can generalise.

**Takeaway:** the cost is real (tens of percent up to several times) and can be removed in two ways. Either the DB stores its recovery data inside its own structure, or the FS exposes atomic/transactional writes the DB can use. LFS can do both, because the FS and the links database share one journal or COW commit.

### 2.3 File systems that embed or are databases

| System | What it did | Outcome | Source |
|---|---|---|---|
| **BeFS (BFS, 1997)** | Typed extended attributes, **B+-tree indexes on attributes**, a query language and *live queries*. Mail and contacts were stored as attribute-rich files. | Shipped in BeOS and Haiku. The canonical "FS as database". | Giampaolo, *Practical File System Design with the Be File System* ([free PDF, archive.org](https://ia600404.us.archive.org/29/items/practical-file-system-design/practical-file-system-design.pdf)) |
| **WinFS** | Relational/object store (SQL Server-based) on top of NTFS for Longhorn. | Cancelled in 2006; parts went into SQL Server and ADO.NET. | [Wikipedia: WinFS](https://en.wikipedia.org/wiki/WinFS) |
| **Reiser4** | Plugin architecture (file/dir/hash/security plugins), dancing trees, atomic FS transactions; aimed to blur files and DB records. | Never merged into mainline. | [Wikipedia: Reiser4](https://en.wikipedia.org/wiki/Reiser4) |
| **APFS + Spotlight** | APFS keeps B-trees internally. Search is a *separate* Spotlight metadata index, so it is not part of the FS. | Indexing is outside the FS, hence duplicated. | [APFS reference](https://developer.apple.com/support/downloads/Apple-File-System-Reference.pdf) |
| **ReFS** | Microsoft's internal "Minstore" B+-tree engine used as the FS. | In production, but not exposed as a general-purpose DB. | [Wikipedia: ReFS](https://en.wikipedia.org/wiki/ReFS) |
| **Oracle DBFS** | The *reverse*: a POSIX FS stored inside Oracle DB (files as SecureFiles LOBs), mounted through `dbfs_client` over FUSE (Linux/Solaris only). | Niche, enterprise only. | [Oracle docs: Introducing DBFS](https://docs.oracle.com/en/database/oracle/oracle-database/18/adlob/introducing-database-file-system.html), [Using DBFS](https://docs.oracle.com/en/database/oracle/oracle-database/12.2/adlob/using-DBFS.html) |
| **SQLite "35 % faster than the FS"** | SQLite argues small blobs in a DB beat individual files. | Reverse evidence that FS metadata overhead hurts small objects. | [sqlite.org/fasterthanfs.html](https://www.sqlite.org/fasterthanfs.html) |
| **BetrFS** | Bε-tree key-value store as the FS. | Research system. | [FAST'17](https://www.usenix.org/conference/fast17/technical-sessions/presentation/conway) |

**Gap that LFS targets:** BeFS indexes only attributes, WinFS sat above the FS (two layers, two journals), and DBFS sits below a DB. None makes a *general-purpose, application-visible* database the FS's own metadata store with one shared commit mechanism, and none is cross-platform.

---

## 3. User-space file system frameworks and Rust crates

### 3.1 Summary table

| OS | Framework | Rust crate (version, license) | Build-time system deps | Runtime deps | Notes |
|---|---|---|---|---|---|
| Linux | kernel FUSE (`/dev/fuse`) | **`fuser` 0.18.0, MIT** ([repo](https://github.com/cberner/fuser)) | **None** by default. On Linux the default "pure-rust" mount is used; libfuse and pkg-config are needed only with feature `libfuse`/`libfuse2`/`libfuse3` ([build.rs](https://github.com/cberner/fuser/blob/master/build.rs)). | `/dev/fuse`. Direct `mount(2)` as root, otherwise falls back to the setuid `fusermount3`/`fusermount` binary (fuse3 package) ([fuse_pure.rs](https://github.com/cberner/fuser/blob/master/src/mnt/fuse_pure.rs)). | Rewrite of libfuse in Rust; "most features of libfuse up to 3.10.3". Alternatives: `fuse3` 0.9.0 MIT (async), `fuse-backend-rs` 0.14.0 Apache-2.0/BSD-3 (also virtio-fs), `fuse_mt` 0.6.4 MIT/Apache. |
| macOS | **macFUSE** (kext; FSKit backend from macFUSE 5 on macOS 26) | `fuser` (links libfuse2-compatible `libfuse` from macFUSE 4.x+) | **pkg-config + macFUSE installed** (`brew install --cask macfuse pkgconf`), unless feature `macos-no-mount` (compiles but cannot mount) | macFUSE. The kext needs user approval and, on Apple Silicon, reduced security plus a reboot. Alternatively `-o backend=fskit` on macOS 26 (libfuse3 only). | macFUSE is closed source with a free licence ([macFUSE](https://macfuse.github.io/), [MacRumors/macFUSE 5 FSKit](https://forums.macrumors.com/threads/macfuse-veracrypt.2485302/), [macFUSE 5.5.0 notes](https://newreleases.io/project/github/macfuse/macfuse/release/macfuse-5.5.0)) |
| macOS | **FUSE-T** (no kext) | `fuser` against FUSE-T's libfuse (API-compatible with libfuse2/3) | FUSE-T installed (`brew install macos-fuse-t/homebrew-cask/fuse-t`) | FUSE-T runs a local **NFSv4 or SMB server** (or FSKit on macOS 26) and mounts it with the system tools | Free for personal use, **commercial licence required for redistribution** ([fuse-t.org](https://www.fuse-t.org/), [GitHub](https://github.com/macos-fuse-t/fuse-t)) |
| macOS | **FSKit** (macOS 15.4+) | none mature. The module must be a Swift app extension (`FSUnaryFileSystem` + `FSUnaryFileSystemOperations`); Rust core through FFI. | Xcode, signing, app extension | macOS ≥ 15.4 (practically ≥ 15.6 because of early bugs) | Only "unary" FS (one block device → one volume); no network FS ([Apple FSKit docs](https://developer.apple.com/documentation/fskit), [FSUnaryFileSystem](https://developer.apple.com/documentation/fskit/fsunaryfilesystem), [ExtendFS (ext4 via FSKit)](https://github.com/kthchew/ExtendFS), [Apple DTS forum](https://origin-devforums.apple.com/forums/thread/776322)) |
| Windows | **WinFsp** | **`winfsp` 0.13.2+winfsp-2.1, GPL-3.0** ([winfsp-rs](https://github.com/SnowflakePowered/winfsp-rs)) | `winfsp-sys` runs **bindgen → needs libclang**. Without feature `system` it uses **bundled headers and import libs, so WinFsp need not be installed to build**. With `system` it reads the install dir from the registry. Requires delay-loading via `winfsp::build::winfsp_link_delayload()` ([build.rs](https://github.com/SnowflakePowered/winfsp-rs/blob/master/winfsp-sys/build.rs)). | WinFsp driver installed (`choco install winfsp`, MSI, or `winget`) | **License caveat:** WinFsp is GPLv3 with a FLOSS linking exception for OSI/FSF-licensed software ([License.txt](https://github.com/winfsp/winfsp/blob/master/License.txt)). The Unlicense is OSI-approved, but the **winfsp-rs crate itself is GPL-3.0**, which would encumber a public-domain binary. |
| Windows | **Dokan (dokany)** | **`dokan` 0.3.1+dokan206, MIT** ([dokan-rust](https://github.com/dokan-dev/dokan-rust)) | `dokan-sys` uses `cc` and **builds the import library from bundled Dokan sources** when the Dokan installer's env var is absent, so no bindgen/libclang. Needs MSVC or GCC. | Dokan driver installed (`dokan2.sys`, LGPL; user library `dokan2.dll`, LGPL) | Crate is dated (last release 2022-12) ([dokany licensing](https://github.com/dokan-dev/dokany#licensing)) |
| Windows | **ProjFS** | `windows` crate (`Win32_Storage_ProjectedFileSystem`, MIT/Apache) or `windows-projfs` 0.1.8 (GPL-2.0) | none | optional feature `Client-ProjFS` | Virtualises directory contents *on NTFS* (as in VFS for Git), so it cannot host a block-device FS ([Microsoft Learn ProjFS](https://learn.microsoft.com/en-us/windows/win32/projfs/projected-file-system)) |
| All 3 | **Local NFSv3 server** | **`nfsserve` 0.11.0, BSD-3-Clause** ([huggingface/nfsserve](https://github.com/huggingface/nfsserve)) | **None** (pure Rust) | OS NFS client: Linux `mount.nfs` (nfs-common), macOS built-in `mount_nfs`, Windows "Client for NFS" (**Pro/Enterprise/Server only**, not Home) | Written for "truly cross-platform" user-mode mounts with no kernel drivers. Windows support is "kinda" (needs `READDIR`, not just `READDIRPLUS`). Used in hf-mount and similar to ZeroFS's approach. |

### 3.2 Linux kernel developments relevant to performance

- **FUSE passthrough** (Linux 6.9, Amir Goldstein): the kernel serves read/write directly from a backing file. Privileged servers only at first ([Phoronix](https://www.phoronix.com/news/Linux-6.9-FUSE-Passthrough)).
- **FUSE over io_uring** (Linux 6.14, Bernd Schubert): batches requests, combines commit and fetch, and uses per-CPU/NUMA queues ([Phoronix](https://www.phoronix.com/news/Linux-6.14-FUSE)).
- **In-kernel Rust file systems**:
  - Wedson Almeida Filho's *Rust VFS abstractions* RFC (v2, May 2024) for read-only file systems came with **ext2** and **tarfs** drivers. **PuzzleFS** (Cisco, Ariel Miculas, Apache-2.0, [repo](https://github.com/project-machine/puzzlefs)) was posted as an RFC v3 driver on top.
  - None of these were merged as of the sources. Upstream reviewers wanted traits that follow the C operation tables.
  - Sources: [Phoronix ext2](https://www.phoronix.com/news/Rust-VFS-Linux-V2-Now-With-EXT2), [LWN 958072](https://lwn.net/SubscriberLink/958072/b08250c903a1097b/), [LWN 974016](https://lwn.net/Articles/974016/), [PuzzleFS RFC v3](https://lkml.rescloud.iu.edu/2405.2/00892.html).
  - An in-kernel LFS is therefore a long-term option at best. FUSE is the practical Linux path.
- **bcachefs** (C, plus Rust in `bcachefs-tools`, GPL-2.0) was **removed from mainline in 6.18** and now ships as DKMS ([LWN](https://lwn.net/Articles/1040120), [Linuxiac](https://linuxiac.com/linux-kernel-6-18-to-ship-without-bcachefs/)). This is a cautionary tale for an out-of-tree file system.

### 3.3 What GitHub Actions hosted runners support

| Runner | Building | Mounting in CI | Evidence |
|---|---|---|---|
| `ubuntu-24.04` | `fuser` builds with **no system packages** (pure-rust mount) | **Yes.** It is a full VM with sudo: `sudo modprobe fuse; sudo chmod 666 /dev/fuse; sudo apt-get install -y fuse3`. Note: newer images ship a non-setuid `fusermount3` in `/usr/local/bin` (podman bundle) that breaks unprivileged mounts; rclone deletes it. NFS: `sudo apt-get install nfs-common` then `mount.nfs` against `nfsserve`. | [rclone build.yml](https://github.com/rclone/rclone/blob/master/.github/workflows/build.yml) (lines "Install Libraries on Linux"), [fuser CI runs mount/pjdfstest/xfstests on ubuntu-24.04](https://github.com/cberner/fuser/blob/master/.github/workflows/ci.yml) |
| `macos-15`/`macos-26` (arm64) | `brew install --cask macfuse` installs headers/libs, so `fuser` builds. pkgconf is preinstalled. | **Effectively no** with macFUSE: loading a third-party kext needs interactive approval and, on Apple Silicon, a Recovery-mode security change plus reboot. fuser's own macOS CI job only runs `cargo check --features=macos-no-mount` and `cargo build`, with no mount tests. **Workable alternatives:** `nfsserve` + built-in `mount_nfs` (runner has passwordless sudo), or FUSE-T (NFS-based, no kext). Both still need to be checked experimentally. | [fuser ci.yml mac-ci job](https://github.com/cberner/fuser/blob/master/.github/workflows/ci.yml), [macOS-15 runner image (pkgconf, clang)](https://github.com/actions/runner-images/blob/main/images/macos/macos-15-arm64-Readme.md), [Telepresence issue on kext approval](https://github.com/telepresenceio/telepresence/issues/4203) |
| `windows-2022`/`windows-2025` | LLVM/clang is preinstalled (LLVM 20.1.8 on 2022), so **winfsp-sys bindgen works**. Chocolatey is preinstalled. | `choco install winfsp -y` installs the driver on hosted runners (rclone does this). Chocolatey sometimes "succeeds" without installing anything, so verify the registry key `HKLM\SOFTWARE\WOW6432Node\WinFsp\InstallDir` and retry. NFS client: `Install-WindowsFeature NFS-Client` on Server images (not verified here). | [Windows2022 runner image README](https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md), [rclone build.yml](https://github.com/rclone/rclone/blob/master/.github/workflows/build.yml), [ext4-win-driver install-winfsp.ps1](https://github.com/antimatter-studios/ext4-win-driver/blob/main/scripts/install-winfsp.ps1) |

**Design consequence for CI:** keep the FS core (`lfs-core`: allocator, journal, links store, indexes) free of OS dependencies, and test it on all 3 runners against a file-backed or in-memory block device. Only the thin OS adapters (fuser / WinFsp-or-Dokan / NFS / FSKit) need mount tests, and those can run on Linux (FUSE + NFS) and Windows (WinFsp) in CI.

---

## 4. Indexing: bitstrings vs trees

### 4.1 Current Links Platform layout (the baseline to compare against)

- In `linksplatform/Data.Doublets` (Unlicense), a **united** (unified) link record holds `Source, Target, LeftAsSource, RightAsSource, SizeAsSource, LeftAsTarget, RightAsTarget, SizeAsTarget` ([RawLink.cs](https://github.com/linksplatform/Data.Doublets/blob/main/csharp/Platform.Data.Doublets/Memory/United/RawLink.cs)). That is 8 words; 2 hold data and 6 are index fields.
- So every membership in the "links with source = X" tree costs **3 words (Left, Right, Size)**, and the same again for the "by target" tree.
- The **split** layout moves the index into a separate `RawLinkIndexPart` (`RootAsSource, LeftAsSource, RightAsSource, SizeAsSource, RootAsTarget, …`) ([RawLinkIndexPart.cs](https://github.com/linksplatform/Data.Doublets/blob/main/csharp/Platform.Data.Doublets/Memory/Split/RawLinkIndexPart.cs)).
- Trees are **Size Balanced Trees** (SBT, Chen Qifeng, 2007; balance by subtree size, which gives O(log n) order statistics for free) ([PEGWiki: Size Balanced Tree](https://wcipeg.com/wiki/Size_Balanced_Tree), [OI-Wiki SBT](https://oi-wiki.org/ds/sbt/)) or AVL trees (files `LinksSourcesSizeBalancedTreeMethods`, `LinksSourcesAvlBalancedTreeMethods` in the same repo).
- `linksplatform/Collections` already has a `BitString` (`long[]` words plus tracking of min/max non-zero word for sparse scans) ([BitString.cs](https://github.com/linksplatform/Collections/blob/main/csharp/Platform.Collections/BitString.cs)).

### 4.2 Space break-even derivation

Definitions:

- `N` = size of the link address space (number of link slots).
- `k` = number of links that use link `X` in a given role (for example, as source).
- `ρ = k / N` = usage density of `X`.
- `w` = address width in bits (64, or 32 for compact stores).
- `p` = index words paid per member.

Cost of each representation for the set {links whose source is X}:

| Representation | Bits | Notes |
|---|---|---|
| Embedded binary tree (SBT/AVL) | `p·w·k` with `p = 3` (left, right, size), so **192·k** for w = 64 | plus one root word |
| Sorted array / B-tree leaf of addresses | ≈ `w·k` = **64·k** (B-tree adds about 1/fanout overhead) | |
| Plain bitstring over the address space | **`N`** (or `max−min+1` if the range is tracked, as `BitString` does) | 1 bit per link: "uses X as source?" |

The bitstring is smaller when `N < p·w·k`, which gives

> **ρ\* = k/N = 1 / (p·w)**
>
> - Embedded tree, 64-bit: **ρ\* = 1/192 ≈ 0.52 %**
> - Embedded tree, 32-bit: 1/96 ≈ 1.04 %
> - Sorted array, 64-bit: 1/64 ≈ 1.56 %
> - Within a 2^16 chunk with 16-bit offsets (Roaring): 1/16, i.e. **4096 entries**

So a link referenced by more than about 0.5 % of all links is cheaper to index with a bitstring than with an embedded 64-bit tree.

**Same rule in Roaring.** Roaring splits 32-bit space into 2^16-value chunks. A chunk is an **array container** (sorted `u16`, 16 bits per entry) up to and including **4096** entries, and a **bitmap container** (exactly 8 KiB = 65536 bits) above that. 65536 / 16 = 4096, which is the same formula with `p·w = 16`. Run containers (2 + 4·runs bytes) cover long consecutive ranges ([RoaringFormatSpec](https://github.com/RoaringBitmap/RoaringFormatSpec), [Lemire et al., *Better bitmap performance with Roaring bitmaps*](https://arxiv.org/abs/1402.6407), [*Consistently faster and smaller compressed bitmaps with Roaring*](https://arxiv.org/abs/1603.06549)).

**Same rule, with hysteresis, in btrfs.** The free-space tree converts extents to bitmaps when `extent_count > bitmap_high_thresh = total_bitmap_bytes / sizeof(item)`, and back when below `high − 100` ([free-space-tree.c](https://github.com/torvalds/linux/blob/master/fs/btrfs/free-space-tree.c)).

**Caveats specific to links:**

1. **Embedded tree slots are already paid for in unified mode.** The 6 index words exist in every link record whether used or not. A bitstring therefore only *saves* space if the layout is variable: a heavily-used X is moved to a bitstring index and its referrers' per-role tree slots are dropped or reused. This is the issue's "variable size storage for optional indexing". Otherwise the bitstring is extra cost, justified by speed.
2. **N grows.** A fixed threshold `k > N/192` drifts as the store grows. Use **chunked bitstrings** (Roaring-like): split the address space into chunks of `C = 2^m` slots and choose per chunk between an offset array (`m` bits per entry), which wins below `C/m` entries, and a bitmap (`C` bits). The decision then depends only on local density, not on `N`.
3. **Hysteresis.** Switch to a bitstring at `ρ > 1/(p·w)` and back only at, say, `ρ < 1/(2·p·w)`, to avoid flapping. btrfs uses an absolute margin of 100 items.

### 4.3 Time break-even (why bitstrings may pay off earlier)

| Operation | Tree (k members) | Bitstring (N bits) |
|---|---|---|
| membership test `X→Y?` | O(log k) dependent random reads | O(1): one word |
| count | O(1) using the `Size` field | O(N/64) popcount unless cached |
| enumerate all members | O(k) pointer chases (cache/disk misses) | O(N/64) sequential words + `tzcnt` |
| insert/delete | O(log k) plus rebalancing writes (several records dirtied, which also costs journal volume) | O(1): one word dirtied |

If one random miss costs about `M` sequential word scans (M ≈ 50–200 for DRAM, far more for disk), enumeration is faster with a bitstring once `k·M > N/64`, i.e. **ρ > 1/(64·M)**. That is roughly 1/3200 to 1/12800, which is *much lower* than the space break-even. Fewer dirtied records per update also means a smaller journal. These are estimates and LFS should benchmark them. Make `p`, `w`, the chunk size and both thresholds runtime-configurable, as the issue requests.

### 4.4 Rust crates

| Crate | Version | License | Use |
|---|---|---|---|
| [`roaring`](https://github.com/RoaringBitmap/roaring-rs) | 0.11.5 | MIT OR Apache-2.0 | Compressed 32-bit (`RoaringBitmap`) and 64-bit (`RoaringTreemap`) sets; portable serialisation format. |
| [`bitvec`](https://github.com/bitvecto-rs/bitvec) | 1.1.1 | MIT | Bit-addressed slices; useful for on-disk bitmap views over `&mut [u64]` from mmap. |
| [`fixedbitset`](https://github.com/petgraph/fixedbitset) | 0.5.7 | MIT OR Apache-2.0 | Simple fixed-size bitset with fast set operations. |
| `linksplatform/doublets-rs`, `mem-rs` | – | Unlicense | Existing Rust doublets store and memory-mapped storage, the natural base ([doublets-rs](https://github.com/linksplatform/doublets-rs), [mem-rs](https://github.com/linksplatform/mem-rs)). |

Because LFS must be public domain and wants exact control of the on-disk format, an in-house `u64`-word bitstring with optional Roaring-style chunking is preferable to taking a dependency for the on-disk format. `roaring` is still useful as a reference and for in-memory caches.

---

## 5. Existing Rust file system implementations

| Project | License | What it is | Why look at it | Source |
|---|---|---|---|---|
| **RedoxFS** (`redoxfs` 0.9.1) | **MIT** | Redox OS's default FS: COW, data and metadata checksums (seahash), transparent encryption (AES-XTS, argon2), lz4. Ships `redoxfs-mkfs`, `redoxfs` (mount), `redoxfs-resize`. Default feature **`fuse` → depends on `fuser`**, so the same Rust core runs in the Redox kernel scheme and in Linux/macOS FUSE. | The closest architectural template: one core crate, FUSE frontend, image-file tooling. | [gitlab.redox-os.org/redox-os/redoxfs](https://gitlab.redox-os.org/redox-os/redoxfs), [Cargo.toml](https://gitlab.redox-os.org/redox-os/redoxfs/-/blob/master/Cargo.toml), [Redox book: RedoxFS](https://doc.redox-os.org/book/redoxfs.html) |
| **TFS** (ticki) | MIT | Earlier Redox FS research (cluster-based, COW, "SPECS.md"); abandoned. | Design docs on COW and caching. | [redox-os/tfs](https://github.com/redox-os/tfs) |
| **fatfs** (rust-fatfs) | MIT | FAT12/16/32 read/write library, `no_std`-capable. | Simple cross-platform FS core with a `Read+Write+Seek` device abstraction. | [rafalh/rust-fatfs](https://github.com/rafalh/rust-fatfs) |
| **ext4-view** 1.0.0 | MIT OR Apache-2.0 | Read-only ext2/3/4 library, `no_std`. | Clean parsing of extents, htree and checksums. | [nicholasbishop/ext4-view-rs](https://github.com/nicholasbishop/ext4-view-rs) |
| **ext4_rs** 1.3.3 | MIT | OS-independent ext4 read/write (used by hobby OSes). | Reference ext4 allocator and extent-tree mutation in Rust. | [yuoo655/ext4_rs](https://github.com/yuoo655/ext4_rs) |
| **ext4-win-driver** / rust-fs-ext4 | GPL-3.0 | ext4 on Windows through **WinFsp** in Rust, with auto-mount service and installer bundling WinFsp. CI installs WinFsp with choco plus verification. | Concrete Windows packaging and CI reference. | [antimatter-studios/ext4-win-driver](https://github.com/antimatter-studios/ext4-win-driver) |
| **ntfs** 0.4.0 (Colin Finck) | MIT OR Apache-2.0 | NTFS reader, from firmware up to user mode. | `no_std` FS parsing patterns. | [ColinFinck/ntfs](https://github.com/ColinFinck/ntfs) |
| **ZeroFS** 0.19.1 | **AGPL-3.0** | Log-structured FS over S3, exported via **NFS, 9P and NBD**. | Shows NFS-server mounting works across OSes; do not copy code (AGPL). | [Barre/ZeroFS](https://github.com/Barre/ZeroFS) |
| **tvix / snix castore** | GPL-3.0 | Content-addressed store with FUSE and virtiofs frontends. | Merkle/content-addressed directory model. | [tvlfyi/tvix](https://github.com/tvlfyi/tvix) |
| **bcachefs-tools** | GPL-2.0 | Userspace tools, partly Rust; bcachefs itself is C. | B-tree-everything design, journal. | [koverstreet/bcachefs-tools](https://github.com/koverstreet/bcachefs-tools) |
| **PuzzleFS** | Apache-2.0 | Content-defined-chunked container image FS in Rust, FUSE plus a kernel driver RFC. | Rust FS with both user-space and kernel frontends. | [project-machine/puzzlefs](https://github.com/project-machine/puzzlefs) |
| **backhand** 0.25.5 | MIT OR Apache-2.0 | SquashFS read/write library. | Image builder/reader patterns. | [crates.io/crates/backhand](https://crates.io/crates/backhand) |
| **gotenksfs** | none declared | Toy FUSE FS in Rust (blog series). | Minimal example of a block-group layout on FUSE. | [carlosgaldino/gotenksfs](https://github.com/carlosgaldino/gotenksfs) |
| **fuser examples** (`hello`, `simple`, `passthrough`) | MIT | Example FUSE file systems. | Starting point for the Linux/macOS adapter. | [fuser/examples](https://github.com/cberner/fuser/tree/master/examples) |
| **winfsp-rs `ntptfs`**, **dokan-rust `memfs`** | GPL-3.0 / MIT | Sample Windows file systems. | Starting point for the Windows adapter. | [winfsp-rs](https://github.com/SnowflakePowered/winfsp-rs), [dokan-rust memfs](https://github.com/dokan-dev/dokan-rust/tree/master/dokan/examples/memfs) |

**License note for a public-domain project:** code from MIT/Apache/BSD projects can be studied, and their crates depended on, without restricting LFS's own source, which stays Unlicense. **GPL/AGPL code (winfsp-rs, ZeroFS, tvix, bcachefs) must not be copied.** Depending on GPL crates makes the distributed *binary* GPL-encumbered.

---

## 6. Visualisation prior art for disk maps

| Tool | Visual idiom | License | Source |
|---|---|---|---|
| **btrfs-heatmap** | Whole physical address space as a PNG laid out on a **Hilbert curve**, so nearby addresses stay nearby in 2-D. Colour = data/metadata/system, brightness = fill ratio. Can make time-lapses. Supports Hilbert, snake and linear walks, and extent-level zoom into block groups. | MIT | [knorrie/btrfs-heatmap](https://github.com/knorrie/btrfs-heatmap), [curves doc](https://github.com/knorrie/btrfs-heatmap/blob/master/doc/curves.md) |
| **Windows Disk Defragmenter (95–XP), UltraDefrag, Defraggler, MyDefrag** | Grid of cells, one per cluster range, coloured by state (free, contiguous, fragmented, system, MFT). Animated while defragmenting. | various (UltraDefrag GPL) | [UltraDefrag](https://ultradefrag.net/) |
| **WinDirStat / QDirStat / KDirStat / GrandPerspective** | **Treemaps** of file sizes by directory (logical space, not physical placement). | GPL-3.0 / GPL-2.0 | [windirstat/windirstat](https://github.com/windirstat/windirstat), [shundhammer/qdirstat](https://github.com/shundhammer/qdirstat) |
| **dust**, **parallel-disk-usage (pdu)** | Terminal bar charts of directory sizes, written in Rust. | Apache-2.0 | [bootandy/dust](https://github.com/bootandy/dust), [KSXGitHub/parallel-disk-usage](https://github.com/KSXGitHub/parallel-disk-usage) |
| **filefrag** (e2fsprogs) | Per-file extent list (logical → physical, length, flags) through the FIEMAP ioctl. | GPL-2.0 | [filefrag(8)](https://man7.org/linux/man-pages/man8/filefrag.8.html) |
| **e2freefrag** | Histogram of free-extent sizes, a free-space fragmentation report. | GPL-2.0 | [e2freefrag(8)](https://man7.org/linux/man-pages/man8/e2freefrag.8.html) |
| **debugfs** (`stat`, `blocks`, `ex`, `icheck`, `ffb`) | Interactive inspection of inodes, extent trees and bitmaps. | GPL-2.0 | [debugfs(8)](https://man7.org/linux/man-pages/man8/debugfs.8.html) |
| **dumpe2fs** | Per-group layout (superblock backups, bitmaps, inode tables, free counts). | GPL-2.0 | [dumpe2fs(8)](https://man7.org/linux/man-pages/man8/dumpe2fs.8.html) |

**Ideas for the LFS GitHub Pages viewer:**

1. A **Hilbert-curve block map**, as in btrfs-heatmap: each pixel = one block or cluster, coloured by owner class (superblock, journal, link records, index trees, bitstring indexes, file data, free, *gap-filler doublet links*). Hovering shows block, owner and link id.
2. A **linear defrag-style strip** for the same data.
3. **Extent list per file**, as in `filefrag -v`.
4. **Free-extent histogram**, as in `e2freefrag`.
5. **Treemap** of logical usage.
6. **Time-lapse** of a scripted workload (create/append/delete), to show visually that no fragmentation builds up.

The data can come from a WASM build of the LFS core running a format-and-workload scenario in the browser, or from JSON dumps produced in CI.

---

## Implications for LFS

1. **Architecture: one OS-agnostic core, thin adapters.**
   - The `lfs-core` crate holds the on-disk format, allocator, journal, links store and indexes. It runs on any `Read+Write+Seek` device (file image, raw device, in-memory, WASM).
   - Adapters, in priority order:
     - (a) **Linux/macOS: `fuser`** (MIT). Pure-rust mount on Linux needs no system libs to build. macOS needs macFUSE or FUSE-T at build time unless built with `macos-no-mount`.
     - (b) **All three OSes: `nfsserve`** (BSD-3) localhost NFSv3. No kernel driver and pure Rust; the best path for macOS without a kext and for CI. Windows needs Pro or higher.
     - (c) **Windows: WinFsp** for the best NTFS-like semantics. Because winfsp-rs is GPL-3.0, prefer **hand-written Unlicense FFI bindings to `winfsp-x64.dll`**, which WinFsp's FLOSS exception allows since the Unlicense is OSI-approved, or use **`dokan`** (MIT crate, LGPL DLL, no bindgen).
     - (d) Later: **FSKit** Swift extension wrapping the Rust core (macOS 15.4+), and in-kernel drivers.
   - RedoxFS (MIT, COW, fuser-based) is the closest existing template.
2. **CI**:
   - Core unit and property tests on ubuntu, macos and windows runners.
   - FUSE mount tests on ubuntu (`modprobe fuse`, `apt install fuse3`; remove the non-setuid `/usr/local/bin/fusermount3`).
   - WinFsp mount tests on windows (`choco install winfsp` plus a registry check; clang is preinstalled for bindgen if winfsp-sys is used).
   - On macOS: build-only for FUSE, and NFS (`mount_nfs`) mount tests through `nfsserve`.
3. **Anti-fragmentation techniques worth copying (simplified):**
   - **extents** with a large or configurable max length (ext4 is capped at 128 MiB by a 15-bit length field; LFS can use 32- or 64-bit lengths);
   - **delayed allocation** (allocate at flush, when the size is known);
   - **per-file preallocation windows** plus **shared small-object arenas** (ext4 `mb_stream_req` = 16 blocks, `mb_group_prealloc` = 512);
   - **free-space index by size class** (mballoc's largest-free-order and average-fragment-size lists, or a buddy allocator);
   - **inline data** for tiny files;
   - **flex_bg-like grouping** of metadata;
   - **metadata checksums** (crc32c).
   - ext4 still ages (FAST'17), so the strict "no fragmentation" goal needs either **online compaction/relocation** (like `EXT4_IOC_MOVE_EXT`) or the issue's idea: **fill gaps with small doublet links** so free space stays in a few large runs and small objects pack densely.
4. **Single journal or commit for FS and database.** The JOJ literature (ATC'13, FAST'14 ×2, ATC'15) shows a 58–73 % to 5× penalty from stacked journals. LFS should:
   - expose **transactions on links and file ranges** to applications, generalising F2FS atomic writes and SQLite's batch-atomic mode;
   - make the journal **optional and per-file/per-subtree configurable**, as in the adaptive per-file journaling of Shen et al., with modes like ext4's `ordered`, `journal` and `writeback`, plus "none" for COW regions;
   - keep checksummed commit blocks, as in JBD2.
5. **Index selection rule:** for a link X with k users out of N slots, store its by-source and by-target sets as
   - an **embedded tree** while `k/N < 1/(p·w)` (= 1/192 for 3×64-bit tree fields),
   - a **bitstring** above that, with hysteresis (for example, switch back below 1/384).
   - Better still, decide **per chunk** (Roaring-style: array ≤ C/m entries, bitmap above), so the rule does not drift as N grows.
   - Make `p`, `w`, the chunk size and the thresholds runtime configuration in the superblock or a mount option. Precedents: Roaring's 4096 and btrfs's free-space-tree conversion.
6. **Portability:** no advanced FS is native on all three OSes (only FAT32, exFAT and UDF are), so a Rust user-space LFS with adapters for all three fills a real gap. Ship a `mkfs.lfs`/`lfs-mount` CLI per OS and document driver prerequisites (macFUSE/FUSE-T, WinFsp/Dokan, NFS client).
7. **Viewer:** compile the core to WASM for GitHub Pages. Render a Hilbert-curve block map (btrfs-heatmap), a linear defrag strip, per-file extent lists (filefrag), a free-extent histogram (e2freefrag) and a time-lapse of a workload.
8. **Licensing:** keep LFS Unlicense. Allowed dependencies are MIT, Apache-2.0, BSD and Unlicense (fuser, nfsserve, dokan, roaring, bitvec, fatfs, ext4-view, redoxfs). Avoid GPL/AGPL crates (winfsp-rs, windows-projfs, ZeroFS, tvix) in the shipped binary, or isolate them behind an optional, clearly labelled feature.
