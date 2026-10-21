# Issue #1 research: Links Platform storage, link-cli, Links Notation

Research for [link-foundation/lfs#1](https://github.com/link-foundation/lfs/issues/1)
("Links file system driver for Linux, macOS, Windows"). This document collects
the on-disk formats and APIs that LFS can reuse. The goal is to make LFS
binary-compatible with the existing Links Platform doublets stores and with
the Links Notation binary format.

Conventions:

- **[verified]** means read directly in source at the pinned commit below.
- **[UNVERIFIED]** means inferred, or read only partially. Check it before relying on it.
- All byte offsets assume `T = u64` (8-byte little-endian cells) unless stated otherwise.
  For `u32`, halve every offset and width.

## 0. Sources and pinned commits

All repositories were cloned read-only into `/tmp/research-src/<name>`. No build
tools were run in them.

| Repo | Commit | Commit date | What it gives |
|------|--------|-------------|---------------|
| [linksplatform/doublets-rs](https://github.com/linksplatform/doublets-rs/tree/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0) | `02b3c9bf` | 2026-10-08 | Rust `doublets` 0.8.0: united/split stores |
| [linksplatform/data-rs](https://github.com/linksplatform/data-rs/tree/8d6ce206df647d0f5ac57c998c5b7644359d424c) | `8d6ce206` | — | `LinksConstants`, `Hybrid` (external refs) |
| [linksplatform/mem-rs](https://github.com/linksplatform/mem-rs/tree/c373135da4dec66c946b504bb455af4aec732816) | `c373135d` | — | `FileMapped` (memmap2) |
| [linksplatform/trees-rs](https://github.com/linksplatform/trees-rs/tree/ed83a999d29c7f6bdd2a9e1eca1d4e643aef1fc3) | `ed83a999` | — | iterative SBT, absolute circular list |
| [linksplatform/Data.Doublets](https://github.com/linksplatform/Data.Doublets/tree/f8c521e539935e670f4f484cf451857d2bd3e693) | `f8c521e5` | 2025-07-19 | C#/C++ reference implementation |
| [linksplatform/Data](https://github.com/linksplatform/Data/tree/b73d27124d8ba655dc232d0963525f372c6e59c4) | `b73d2712` | 2025-07-19 | C# `LinksConstants`, `Hybrid` |
| [linksplatform/Memory](https://github.com/linksplatform/Memory/tree/bf31e5d40174919e515bf52917219a3b67bdeac4) | `bf31e5d4` | 2025-07-19 | C# `FileMappedResizableDirectMemory` |
| [linksplatform/Collections](https://github.com/linksplatform/Collections/tree/432deeb6e36d47c2358b9db391203566add35dd4) | `432deeb6` | 2025-07-19 | `BitString` |
| [linksplatform/Data.Doublets.Sequences](https://github.com/linksplatform/Data.Doublets.Sequences/tree/6a6a69fc3ce42b0bd3e421c17c810ec2f37cb12b) | `6a6a69fc` | 2025-07-19 | raw numbers, balanced-variant sequences |
| [linksplatform/RawDoubletsViewer](https://github.com/linksplatform/RawDoubletsViewer/tree/9ce26288b58c9e446418173f5292601a8e4212e1) | `9ce26288` | — | third-party reader of the raw file format |
| [konard/linksplatform](https://github.com/konard/linksplatform/tree/9204dc44eb3773ea9c8b4559dc9e28744eb25cb9) | `9204dc44` | 2025-02-15 | 2010-era prototype (historical only) |
| [link-foundation/link-cli](https://github.com/link-foundation/link-cli/tree/f617e051a8acdb0ed0a1a06a50347c29187bd56b) | `f617e051` | 2026-10-15 | `clink` CLI (C# + Rust), binary notation |
| [link-foundation/links-notation](https://github.com/link-foundation/links-notation/tree/06ee7ea54cadaa82eaa74bfe038527559399f71a) | `06ee7ea5` | 2026-10-19 | LiNo grammar + normative binary spec |

Permalink prefixes used below:

- `DR` = `https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0`
- `DD` = `https://github.com/linksplatform/Data.Doublets/blob/f8c521e539935e670f4f484cf451857d2bd3e693`
- `LC` = `https://github.com/link-foundation/link-cli/blob/f617e051a8acdb0ed0a1a06a50347c29187bd56b`
- `LN` = `https://github.com/link-foundation/links-notation/blob/06ee7ea54cadaa82eaa74bfe038527559399f71a`

`linksplatform/Data.Doublets.Cpp` does not exist ("repository not found"). The
C++ code lives in `Data.Doublets/cpp/`.

---

## 1. Doublets store: the canonical binary format

A *doublet* is a triple `(index: source target)`. `index` is the position of the
record in a flat array. `source` and `target` are references: other indices, or
"external" raw values (see §1.6). Index `0` is `null`. Index-0 storage is
reused for the header.

### 1.1 Header (`LinksHeader<T>`) — 8 cells, stored in slot 0 [verified]

The field order is identical in Rust, C# and C++, and in the independent
RawDoubletsViewer (which reads it as `INT64`):

- Rust: [`DR/doublets/src/mem/header.rs#L5`](https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0/doublets/src/mem/header.rs#L5)
- C#: [`DD/csharp/Platform.Data.Doublets/Memory/LinksHeader.cs`](https://github.com/linksplatform/Data.Doublets/blob/f8c521e539935e670f4f484cf451857d2bd3e693/csharp/Platform.Data.Doublets/Memory/LinksHeader.cs)
- C++: [`DD/cpp/Platform.Data.Doublets/Memory/LinksHeader.h`](https://github.com/linksplatform/Data.Doublets/blob/f8c521e539935e670f4f484cf451857d2bd3e693/cpp/Platform.Data.Doublets/Memory/LinksHeader.h)

```rust
// doublets/src/mem/header.rs  (#[repr(C)])
pub struct LinksHeader<T: LinkReference> {
    pub allocated: T,      pub reserved: T,
    pub free: T,           pub first_free: T,
    pub root_as_source: T, pub root_as_target: T,
    pub last_free: T,      __reserved_8: T,
}
```

| Offset (u64) | Width | Rust name | C# name | Meaning |
|---:|---:|---|---|---|
| 0  | 8 | `allocated` | `AllocatedLinks` | highest index ever handed out and still in range (high-water mark) |
| 8  | 8 | `reserved` | `ReservedLinks` | capacity in links that fits the current mapping |
| 16 | 8 | `free` | `FreeLinks` | number of entries in the unused (free) list |
| 24 | 8 | `first_free` | `FirstFreeLink` | head of the circular free list (0 = empty) |
| 32 | 8 | `root_as_source` | `RootAsSource` | root of the global "by source" SBT (united) or the external-sources tree (split) |
| 40 | 8 | `root_as_target` | `RootAsTarget` | root of the global "by target" SBT (united) or the external-targets tree (split) |
| 48 | 8 | `last_free` | `LastFreeLink` | tail of the free list |
| 56 | 8 | `__reserved_8` | `Reserved8` | unused, always 0 |

The header is exactly one record wide in the united layout: 64 B for u64, 32 B for u32.
`count = allocated - free` is the number of live links.

### 1.2 United ("unit") record (`LinkPart<T>` / `RawLink<T>`) — 8 cells [verified]

- Rust: [`DR/doublets/src/mem/unit/raw_link.rs#L5`](https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0/doublets/src/mem/unit/raw_link.rs#L5)
- C#: [`DD/csharp/Platform.Data.Doublets/Memory/United/RawLink.cs`](https://github.com/linksplatform/Data.Doublets/blob/f8c521e539935e670f4f484cf451857d2bd3e693/csharp/Platform.Data.Doublets/Memory/United/RawLink.cs)
- C++: `DD/cpp/Platform.Data.Doublets/Memory/United/RawLink.h`

```rust
#[repr(C)]
pub struct LinkPart<T> {
    pub source: T, pub target: T,
    pub left_as_source: T, pub right_as_source: T, pub size_as_source: T,
    pub left_as_target: T, pub right_as_target: T, pub size_as_target: T,
}
```

| Offset in record | Width | Field | Role |
|---:|---:|---|---|
| 0  | 8 | `source` | data. In a freed slot this is the `prev` pointer of the free list |
| 8  | 8 | `target` | data. In a freed slot this is the `next` pointer of the free list |
| 16 | 8 | `left_as_source` | left child in the sources tree |
| 24 | 8 | `right_as_source` | right child in the sources tree |
| 32 | 8 | `size_as_source` | subtree size in the sources tree (0 means not in the tree) |
| 40 | 8 | `left_as_target` | left child in the targets tree |
| 48 | 8 | `right_as_target` | right child in the targets tree |
| 56 | 8 | `size_as_target` | subtree size in the targets tree |

**Absolute file offset of link `i` = `64 * i`** for u64 (`32 * i` for u32). Record 0
is the header, so links start at byte 64. In C#,
`GetLinkReference(i) = _links + LinkSizeInBytes * i` and `_header = _links = memory.Pointer`.
In Rust, the header is `transmute` of `mem[0]`. RawDoubletsViewer reads one
`LPFileHeader` (8×INT64) and then `LPItem`s until EOF, which confirms the layout
from outside the main implementations
([`engine.h`](https://github.com/linksplatform/RawDoubletsViewer/blob/9ce26288b58c9e446418173f5292601a8e4212e1/winapi/engine.h)).

So 75% of each united record is index (6 of 8 cells). Only 16 bytes are payload.

### 1.3 Split layout (two files/memories) [verified]

The split layout has a **data** memory of `DataPart` records and an **index**
memory of `IndexPart` records. Both are addressed by the same link index. The
header lives in index slot 0.

- [`DR/doublets/src/mem/split/data_part.rs#L5`](https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0/doublets/src/mem/split/data_part.rs#L5)
- [`DR/doublets/src/mem/split/index_part.rs#L5`](https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0/doublets/src/mem/split/index_part.rs#L5)
- C#: `Memory/Split/RawLinkDataPart.cs` and `RawLinkIndexPart.cs` use the same orders.

DataPart (16 B per link, data file offset `16 * i`):

| Off | W | Field |
|---:|---:|---|
| 0 | 8 | `source` |
| 8 | 8 | `target` |

IndexPart (64 B per link, index file offset `64 * i`; slot 0 is the `LinksHeader`):

| Off | W | Field | Meaning |
|---:|---:|---|---|
| 0  | 8 | `root_as_source` | root of the **internal** tree of links whose source = this link, keyed by target |
| 8  | 8 | `left_as_source` | this link as a node of `IndexPart[self.source].root_as_source`'s tree |
| 16 | 8 | `right_as_source` | " |
| 24 | 8 | `size_as_source` | " |
| 32 | 8 | `root_as_target` | root of the internal tree of links whose target = this link, keyed by source |
| 40 | 8 | `left_as_target` | |
| 48 | 8 | `right_as_target` | |
| 56 | 8 | `size_as_target` | |

Split therefore has **per-link usage trees** (internal) plus **two global trees**
rooted in the header (external) for references outside the internal range.

- C# routes a reference to the external tree when
  `Constants.ExternalReferencesRange.Contains(value)` (`SplitMemoryLinksBase.cs` ~L629–L680).
- Rust split instead uses `is_virtual(link) = is_unused(link)`
  ([`DR/doublets/src/mem/split/store.rs#L345`](https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0/doublets/src/mem/split/store.rs#L345)).
  That means unallocated or freed indices go to the external trees.
- **These two rules differ.** Rust and C# split files are therefore *not
  guaranteed* to be interchangeable when references point at unallocated or
  external values. **[verified divergence; impact on real files UNVERIFIED]**
- C# split has an optional `useLinkedList` mode for the internal sources index.
  In Rust, `USE_LIST = false` and the list path hits `todo!()`.
- C# chooses the smaller of the two internal trees for `(s,t)` lookups
  (`CountUsages(source) > CountUsages(target)`).

### 1.4 Free ("unused") list [verified]

The free list is an absolute circular doubly-linked list
(`trees-rs/src/lists/absolute_circular_linked_list.rs`). It is threaded
**through the freed records themselves**:

- `first = header.first_free`, `last = header.last_free`, `size = header.free`;
- `prev(slot) = slot.source` and `next(slot) = slot.target`;
- see `DR/doublets/src/mem/unit/generic/unused_links.rs`.

A slot is treated as unused when:

```text
is_unused(i) = if i <= allocated && first_free != i {
                   size_as_source == 0 && source != 0     // united
               } else { true }
```

In split, the same check uses `index.size_as_target == 0 && data.source != 0`.
In C#, `IsUnusedLink` is `FirstFreeLink == i || (SizeAsSource == 0 && Source != 0)`.

**Note:** a live point link `(i: i i)` always has `size_as_source >= 1` because
it is in the tree, so the test is unambiguous. A freed slot that is the *only*
free entry has `prev = next = itself`. That is why `first_free == i` is checked
separately.

### 1.5 Create / delete / grow algorithms [verified]

Create:
[`DR/doublets/src/mem/unit/store.rs#L452`](https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0/doublets/src/mem/unit/store.rs#L452)

```rust
let mut free = header.first_free;
if free == constants.null {
    let max_inner = *constants.internal_range.end();
    if header.allocated >= max_inner { return Err(LinksError::LimitReached(max_inner)); }
    if header.allocated >= header.reserved - T::from_byte(1) {
        let new_cap = self.mem.allocated().len() + self.reserve_step;
        let mem = crate::mem::resize_mem(&mut self.mem, new_cap)?.leak();
        self.update_mem(mem);
        let reserved = self.mem.allocated().len();
        let header = self.mut_header();
        header.reserved = T::try_from(reserved).expect("always ok");
    }
    let header = self.mut_header();
    header.allocated = header.allocated + T::from_byte(1);
    free = header.allocated;
} else {
    self.unused.detach(free);   // reuse most recently freed slot (LIFO via attach_as_first)
}
// new link is (free: 0 0); caller then calls update(free, s, t)
```

Delete:
[`store.rs#L543`](https://github.com/linksplatform/doublets-rs/blob/02b3c9bf073e7a182acd24ac7ac6e652bebf7ee0/doublets/src/mem/unit/store.rs#L543)

1. Call `update(index, 0, 0)`, which detaches the link from both trees.
2. If `index < allocated`, call `unused.attach_as_first(index)`.
3. If `index == allocated`, decrement `allocated`, then keep popping trailing
   unused slots off the free list. This means the store **shrinks its
   high-water mark**, and never leaves holes at the end.

Update: detach from the source and target trees (when the old value ≠ 0),
write the fields, then attach to the trees rooted at `header.root_as_source` /
`root_as_target`.

Growth and initialisation:

- **Rust:**
  - `DEFAULT_PAGE_SIZE = 8*1024` items;
  - `SIZE_STEP = 2^20` items, i.e. **64 MiB per step for u64 united**;
  - `init()` resizes to `max(reserve_step, allocated)` and sets `reserved = len - 1`.
    The `create` path sets `reserved = len`, so there is an **off-by-one inconsistency**.
    It is harmless because the check is `allocated >= reserved - 1`.
- **C#:**
  - `DefaultLinksSizeStep = LinkSizeInBytes * 1024 * 1024` bytes, which is 2^20 links
    ([`UnitedMemoryLinksBase.cs#L52`](https://github.com/linksplatform/Data.Doublets/blob/f8c521e539935e670f4f484cf451857d2bd3e693/csharp/Platform.Data.Doublets/Memory/United/Generic/UnitedMemoryLinksBase.cs#L52));
  - `Init`: `UsedCapacity = Allocated*LinkSize + HeaderSize` and
    `ReservedLinks = (ReservedCapacity - HeaderSize) / LinkSize` (L175–L186);
  - on growth: `ReservedLinks = ReservedCapacity / LinkSize` (L529).
- **File size on disk:**
  - C# `FileMappedResizableDirectMemory` sets the file size to `ReservedCapacity`
    while it is open. On dispose it **truncates the file to `UsedCapacity`**,
    i.e. `(allocated + 1) * 64` bytes for u64
    ([`Memory/.../FileMappedResizableDirectMemory.cs`](https://github.com/linksplatform/Memory/blob/bf31e5d40174919e515bf52917219a3b67bdeac4/csharp/Platform.Memory/FileMappedResizableDirectMemory.cs)).
    On open it rounds the reservation up to a multiple of the step.
  - Rust `mem-rs::FileMapped` grows with `set_len` plus a remap, and on drop only
    calls `sync_all`. It **does not truncate**, so Rust files stay at whole multiples
    of the step (≥ 64 MiB).
- **Known Rust persistence bug:** upstream `RawMem::grow_filled` zero-fills the whole
  newly mapped range, including bytes already on disk. Reopening a file-mapped DB
  therefore *zeroes it*. link-cli works around this with `PersistentFileMapped`
  (`LC/rust/src/storage/file_mem.rs`, using `grow_filled_exact`). Upstream report:
  linksplatform/mem-rs#36. **LFS must not use plain `FileMapped` with doublets 0.8.0.**

### 1.6 Reserved constants and external ("hybrid") references [verified]

From `data-rs/src/constants.rs` (`full_new`, L27). The C# `Platform.Data/LinksConstants.cs`
behaves the same way.

| Constant | Value |
|---|---|
| `null` | 0 |
| index/source/target part | 0 / 1 / 2 |
| `continue` | `internal.end` (top of the possible range) |
| `break` | end − 1 |
| `skip` | end − 2 |
| `any` | end − 3 |
| `itself` | end − 4 |
| `error` | end − 5 |
| internal range | `start ..= end − 6` |

- Without external references, the possible internal range is `1 ..= T::MAX`.
- **With external references** (`LinksConstants::external()`, C#
  `enableExternalReferencesSupport: true`), the internal range is `1 ..= half` and the
  external range is `half+1 ..= MAX`, where `half = T::MAX / 2`. For u64 that is
  `0x7FFF_FFFF_FFFF_FFFF`, so the external range starts at `0x8000_0000_0000_0000`.
- The 6 control constants are carved out of the top of the *internal* range.

Hybrid encoding (`data-rs/src/hybrid.rs`; C# `Platform.Data/Hybrid.cs`):

```rust
pub fn external(value: T) -> Self { Self::new((T::MAX - value) + 1) } // = two's-complement negation
pub fn is_internal(&self) -> bool { self.value < Self::half() }
pub fn abs(&self) -> T { self.value + 1 + T::MAX }                     // wrapping
```

C# has a special case: `ExternalZero = half + 1`, which is the encoding of external 0.

What this means for the format:

- **A raw external number `n` is stored as `-n` (mod 2^64).** The high bit set marks it
  as external.
- This is how Links Platform stores raw numbers, bytes and Unicode code points
  *without* creating links for them: `AddressToRawNumberConverter` is
  `new Hybrid(source, isExternal: true)`.

### 1.7 Index trees: size-balanced trees, recursionless and AVL [verified]

United layout:

- The two global trees are rooted in the header.
- Every live link is a node in **both** trees, using cells 2..4 and 5..7.
- Sources tree ordering (`sources_recursionless_size_balanced_tree.rs` L246):
  ```rust
  (first_source < second_source) || (first_source == second_source && first_target < second_target)
  ```
- Targets tree ordering: `(target, source)` lexicographic (`targets_recursionless_size_balanced_tree.rs` L246).
- `size_*` is the classic SBT subtree count. 0 means "not in this tree". `clear_node`
  zeroes left, right and size.
- `count_usages(s)` uses two descents with size arithmetic, O(log n). `search(s,t)` is a
  binary descent. `each_usages(s)` is an in-order range walk.
- Query dispatch in `unit/store.rs`:
  - `(any, s, any)` uses the sources tree;
  - `(any, any, t)` uses the targets tree;
  - `(any, s, t)` uses `sources.search`.
- Rust uses only the **recursionless (iterative) SBT** (`trees-rs/src/trees/iterative_size_balanced_tree.rs`).
  `attach_core` loops with rotations driven by `left_size + 1 > right_size`-style
  comparisons; there is no recursion and no parent pointers.
- C# `IndexTreeType` (`Memory/IndexTreeType.cs`) has `Default=0`, `SizeBalancedTree=1`,
  `RecursionlessSizeBalancedTree=2` and `SizedAndThreadedAVLBalancedTree=3`. In
  `UnitedMemoryLinks` only value 3 selects AVL; **all others, including "Recursionless",
  use the recursive SBT** in the united store (`UnitedMemoryLinks.cs` L102). The tree
  *encoding* is identical for SBT variants, so recursive and iterative are
  file-compatible.
- C# AVL variant (`LinksAvlBalancedTreeMethodsBase.cs` L478–L666) packs extra bits
  into the `Size*` cell, which makes it **not** compatible with SBT files:

  | Bits of `Size*` | Meaning |
  |---|---|
  | 0..2 | balance (signed, 3 bits) |
  | 3 | right-is-child (thread flag) |
  | 4 | left-is-child (thread flag) |
  | 5..63 | subtree size |

### 1.8 Byte map example (u64 united, three links) [derived from §1.1–1.7]

Input: `() ((1 1))`, `() ((2 2))`, `() ((1 2))`.

| File offset | Content |
|---:|---|
| 0x000 | header: allocated=3, reserved=R, free=0, first_free=0, root_as_source=?, root_as_target=?, last_free=0, 0 |
| 0x040 | link 1: src=1 tgt=1, tree cells |
| 0x080 | link 2: src=2 tgt=2, tree cells |
| 0x0C0 | link 3: src=1 tgt=2, tree cells |

The roots depend on SBT rotations, so they are not reproduced here. **[UNVERIFIED]**

---

## 2. Bit strings, sequences and "variable size" storage in Links Platform

### 2.1 `BitString` (Platform.Collections) [verified]

[`Collections/csharp/Platform.Collections/BitString.cs`](https://github.com/linksplatform/Collections/blob/432deeb6e36d47c2358b9db391203566add35dd4/csharp/Platform.Collections/BitString.cs):

- State:
  ```csharp
  private long[] _array;  private long _length;
  private long _minPositiveWord;  private long _maxPositiveWord;  // "borders": first/last non-zero word
  ```
- Words are 64 bits: `GetWordIndexFromIndex(i) = i >> 6` and `GetBitMaskFromIndex(i) = 1L << (i & 63)`.
- Operations:
  - scalar, `Parallel*`, `Vector*` (SIMD) and `ParallelVector*` forms of Not/And/Or/Xor;
  - `GetSetIndices`, `GetFirstSetBitIndex`, `GetLastSetBitIndex`, `CountSetBits` (16-bit lookup table);
  - `HaveCommonBits`, `GetCommonIndices`, `CountCommonBits`.
- The border words let sparse bitmaps skip leading and trailing zero words.
- It is used in `Data.Doublets.Sequences` (`DuplicateSegmentsProvider`,
  `Sequences.Experiments.cs`) as a **set of link indices**. For example,
  `new BitString(links.Count() + 1)` collects all usages and then intersects them with
  `And`. That is an in-memory query accelerator, **not** a persisted index.
- **No persisted bit-string index exists anywhere in the inspected repos.** konard/linksplatform
  holds only a 2010 prototype (`28.03.2010-04.11.2010/Net`) and the
  `Platform.Data.*Server/Terminal` projects, with no bit-string code.
  The issue's "bit string indices as an alternative to binary trees" is therefore a
  **new design** for LFS. `BitString` is the closest prior art for the API shape.

### 2.2 Sequences, raw numbers, bytes and Unicode [verified]

Source: `Data.Doublets.Sequences`.

- `BalancedVariantConverter` turns a list `[a,b,c,d,e]` into a balanced binary tree of
  doublets by repeatedly pairing neighbours (`GetOrCreate(a,b)`, …). Odd tails carry
  over. With deduplication (`GetOrCreate`), identical sub-sequences share links.
- `BytesToRawNumberSequenceConverter`: each byte is turned into a raw number (an external
  reference, §1.6), the list is built into a balanced-variant sequence, and the type
  markers `ByteArrayType`, `ByteArrayLengthType` and `EmptyByteArraySequenceType` are
  created as `(Type, unicodeName)` links.
- Unicode strings: `StringToUnicodeSequenceConverter` maps chars to
  `(UnicodeSymbolMarker, raw(char))` symbols and then to a sequence.
- Other converters exist: `CompressingConverter` and `OptimalVariantConverter`
  (pair-frequency based compression), plus frequency indexes
  (`FrequencyIncrementingSequenceIndex`). They were not read in detail. **[UNVERIFIED semantics]**
- The doublets-rs repo has `sequences/converters.rs` and `unicode.rs`, which were not read. **[UNVERIFIED]**

### 2.3 Thresholds / "variable size indexing"

No code was found that switches index strategy by a usage-% threshold. The only adaptive
choices that exist are:

- C# split picks the smaller internal tree per query;
- `useLinkedList` vs tree for internal sources;
- the binary notation's per-section width selection (§4.4), which is the closest
  analogue to "variable size".

**The threshold mechanism in the issue is new work.**

---

## 3. link-cli (`clink`)

### 3.1 Storage and file naming [verified]

- **C# CLI:**
  - `new UnitedMemoryLinks<uint>(databaseFilename)` (u32, so **32-byte records**), wrapped in
    `DecorateWithAutomaticUniquenessAndUsagesResolution`
    ([`LC/csharp/Foundation.Data.Doublets.Cli.Library/NamedTypesDecorator.cs#L37`](https://github.com/link-foundation/link-cli/blob/f617e051a8acdb0ed0a1a06a50347c29187bd56b/csharp/Foundation.Data.Doublets.Cli.Library/NamedTypesDecorator.cs#L37)).
  - Names live in a second united store with external references enabled (L45–L63):
    ```csharp
    var namesDatabaseFilename = Path.Combine(directory ?? string.Empty, $"{filenameWithoutExtension}.names.links");
    var namesMemory = new FileMappedResizableDirectMemory(namesDatabaseFilename, UnitedMemoryLinks<TLinkAddress>.DefaultLinksSizeStep);
    var namesLinks  = new UnitedMemoryLinks<TLinkAddress>(namesMemory, UnitedMemoryLinks<TLinkAddress>.DefaultLinksSizeStep, namesConstants, IndexTreeType.Default);
    ```
  - A name is stored as `(external(addr) (NameType unicodeSequence))`
    (`NamedLinks.cs` L30–L40). The address is a hybrid external reference into the main DB.
- **Rust CLI default `LinkStorage`:**
  - It is a **text file**, not binary doublets
    ([`LC/rust/src/link_storage.rs`](https://github.com/link-foundation/link-cli/blob/f617e051a8acdb0ed0a1a06a50347c29187bd56b/rust/src/link_storage.rs)).
  - Each line is `(index source target)` or `(index source target "name")`.
  - An optional first line `# unused: a b c` records freed addresses, most recent first (L52).
  - Saving writes `db.links.saving` and then calls `rename` (L356–L402).
    `docs/HOW-IT-WORKS.md` says the rewrite "truncates in place". **That doc and the code
    disagree; the code renames.**
- **Rust `storage::DoubletsStorage`** is the binary path:
  `FileMappedUnitStore<T> = UnitStore<T, PersistentFileMapped<LinkPart<T>>>`
  (`LC/rust/src/storage/doublets_storage.rs#L49`), which is byte-compatible with C# for the same `T`.
- Default DB: `db.links`. Sidecars derived from `<name>`:

| File | Purpose |
|---|---|
| `<name>.links` | main doublets store |
| `<name>.names.links` | names store (external refs on) |
| `<name>.triggers.links` | triggers (C# only) |
| `<name>.transitions.links` | transactions journal (`--transactions`) |
| `<name>.versioncontrol.links` | version control (`--vc`) |
| `<name>.links.lock` | lock file |

### 3.2 Query language: substitution over LiNo [verified]

From [`LC/README.md` L100–L215](https://github.com/link-foundation/link-cli/blob/f617e051a8acdb0ed0a1a06a50347c29187bd56b/README.md#L110).
A query is **two LiNo link lists**: `(restriction-patterns) (substitution-patterns)`.

| Operation | Query | Semantics |
|---|---|---|
| Create | `() ((1 1))` | nothing becomes `(1 1)`. The index is chosen by the store (free list first) |
| Create many | `() ((1 1) (2 2))` | |
| Read | `((1: 1 1)) ((1: 1 1))` | identity substitution acts as a match |
| Read all | `((($i: $s $t)) (($i: $s $t)))` or `((($i:)) (($i:)))` | `$x` are variables |
| Update | `((1: 1 1)) ((1: 1 2))` | same id on both sides, so the link is changed |
| Delete | `((1 1)) ()` | matched link becomes nothing |

- Pattern atoms: numbers (ids), names (resolved through the names store), `$var`,
  `*`/any, nested links, and explicit `id:`.
- Links are unique by `(source, target)`: the uniqueness decorator turns repeated
  creates into no-ops.
- Deletes cascade to the links that use the deleted link (usages resolution).
- `--auto-create-missing-references` creates names as point links, e.g. `(father: father father)`.
- Other options:
  - `--changes`, `--after`, `--before`;
  - `--in`/`--out` to import/export `.lino`;
  - `--export-binary`/`--import-binary` for the store archive (§4.6);
  - `--structure <id>`;
  - triggers `--always`/`--once`/`--never`;
  - `--transactions` with retention `infinite | sized:N | chunked:N:DIR` and commit
    `sync | async`;
  - `--serve`/`--connect` over TCP, text or binary;
  - `--external-references`, `--arity`, `--packed-widths`.
- Dependencies (Rust): `doublets = "0.8.0"`, `links-notation = "0.26.0"`,
  `lino-arguments = "0.5.0"`. License: Unlicense.

---

## 4. Links Notation

### 4.1 Text grammar summary (LiNo) [verified]

Source: [`LN/docs/grammar/GRAMMAR.md`](https://github.com/link-foundation/links-notation/blob/06ee7ea54cadaa82eaa74bfe038527559399f71a/docs/grammar/GRAMMAR.md)
(EBNF v0.24.0), plus `links-notation.ebnf` and `grammar.lino`.

```ebnf
document          = skip_empty_lines, links, whitespace, EOF | whitespace, EOF ;
element           = any_link, PUSH_INDENTATION, links | any_link ;   (* indentation = children *)
any_link          = nested_group, eol | indented_id_link | single_line_any_link ;
nested_group      = "(" ENTER_NESTED_CONTEXT nested_group_body EXIT_NESTED_CONTEXT ;
single_line_link  = hws, reference, hws, ":", single_line_values ;   (* "id: v1 v2 ..." *)
indented_id_link  = reference hws ":" eol ;                          (* "id:" then indented values *)
reference         = reference_literal        (* "~1{hex}" *)
                  | delimited_reference      (* n-quoted with " ' ` *)
                  | simple_reference ;
reference_symbol  = any_char - whitespace - "(" - ":" - ")" ;
```

- A link is `(id: v1 v2 …)` or `(v1 v2 …)` with no id. Values are references or nested links.
- Doublets are `(id: source target)`. Triplets and n-tuples use the same syntax.
- Indentation is equivalent to nesting:
  ```
  3:
    papa
    loves
    mama
  ```
  is `(3: papa loves mama)`.
- Multi-line groups are allowed: `value ( id "1"  label "one" )` reads as
  `(value ((id 1) (label one)))`.
- Comments start with `#` at the start of the document or after whitespace.
- Implementations exist in Rust, C#, JS, Python, Go, Java, PHP and C++.

### 4.2 What "binary links notation" means

There are two specs that share the name. **The one that matters is the links-notation
Binary v1 spec.**

1. **links-notation Binary v1** (normative):
   [`LN/docs/protocol/binary-links-notation.md`](https://github.com/link-foundation/links-notation/blob/06ee7ea54cadaa82eaa74bfe038527559399f71a/docs/protocol/binary-links-notation.md)
   (411 lines).
   - 96 golden vectors live in `docs/protocol/binary-links-notation-vectors.txt`.
   - Codecs exist in 8 languages (`links_notation::binary` in Rust,
     `Link.Foundation.Links.Notation.Binary` in C#, …).
   - link-cli carries an identical copy plus CLI notes (`LC/docs/protocol/binary-links-notation.md`)
     and uses it for `--export-binary` and the binary TCP mode.
2. **links-queue "BLN"** (`link-foundation/links-queue` `docs/BINARY-NOTATION-SPEC.md`, fetched via the GitHub API):
   - an 11-byte frame (`LNKQ` magic, u16 version, flags, u32 BE length);
   - a per-link type byte (SOURCE_IS_ID, TARGET_IS_ID, SELF_REF, HAS_ID, HAS_VALUES, ID_SIZE);
   - typed literals.
   link-cli issue #105 rejected it for stores because it encodes value trees, not
   fixed-width addressed links.

### 4.3 Binary v1: header byte and compact layout [verified]

| Header bits | Meaning |
|---|---|
| 7..4 | version = `0001` |
| 3..2 | log2(width) for the compact layout: 0 = 1 B, 1 = 2 B, 2 = 4 B, 3 = 8 B |
| 1 | 0 = compact, 1 = explicit |
| 0 | external references enabled |

- **Compact layout:** `0x10 | ext | (w << 2)`, then `LEB128 N`, then `N` doublets of 2
  refs each. It is a single section with gap 5 (link ids start at 6) and arity 2.
- The empty packet is `10 00`.
- References are little-endian, of width `w`.

### 4.4 Binary v1: explicit layout (sections and gaps) [verified]

`0x12 | ext`, then `LEB128 S` sections. Each section consists of:

| Field | Encoding |
|---|---|
| shape | LEB128: bits 0–1 = width log2, bit 2 = "gap follows", bit 3 = variable arity, bits 4+ = `min` arity |
| gap | LEB128, optional: number of ids skipped before this section (this is how holes and free slots are kept) |
| extra | LEB128, only when variable arity: 0 = no max, else `max − min` |
| count | LEB128: links in the section |
| links | fixed arity: `min` refs. Variable arity: `LEB128(len − min)` followed by the refs |

- External references at width `w` bits:
  - value `v ≥ 1` is stored as `2^w − v`;
  - external 0 is stored as `2^(w−1)`;
  - any raw value `≥ 2^(w−1)` is external.
  This is the same negation trick as `Hybrid` (§1.6), applied per width.
- Packed widths: the encoder splits a run into sections using a linear DP over 8 states,
  choosing a width per section. The spec guarantees packed output is never larger than
  uniform output.
- Decoder limits: 2^22 links, 2^24 references, 2^22 nodes, 64 MiB of strings, depth 64.

### 4.5 Binary v1: LiNo document mapping [verified]

Marker points:

| Id | Marker |
|---|---|
| 0 | null |
| 1 | One |
| 2 | Number (unary) |
| 3 | String (code points) |
| 4 | List |
| 5 | Identified |

Document links start at id 6.

Example from the spec: `() ((1 1))` encodes to
`10 07 02 01 06 06 07 00 04 08 00 09 0a 00 04 0b`.
The size table gives 13 B as text, 16 B plain and 14 B with external references.

### 4.6 Binary v1: store archive (§10 of the spec) [verified]

An archive is two concatenated packets:

1. **Links packet:** no external references, packed widths, explicit layout. Holes
   (freed or unused indices) are kept as `gap`s, so ids are preserved exactly.
2. **Names packet:** external references on. Each entry is `address code-point…`, i.e.
   a variable-arity link whose first ref is the address and whose remaining refs are
   Unicode code points (external).

- Empty archive: `10 00 11 00`.
- Golden archive:
  `12 02 20 01 24 01 02 01 01 03 03 01 03 13 02 21 02 30 01 ff ff 9f ff fd ff 17 ff fc 9f 9e`.

---

## 5. Implications for LFS

Concrete recommendations, ordered by confidence.

1. **Adopt the united doublets file format byte-for-byte as the LFS "links region".**
   - Header = 8 cells in slot 0 (§1.1). Record = 8 cells
     `source, target, left_s, right_s, size_s, left_t, right_t, size_t` (§1.2).
   - Link `i` lives at `region_base + 64*i` for u64.
   - Keep `Reserved8` = 0 in the base format. LFS needs a version/feature word somewhere;
     put it in a *separate* LFS superblock rather than in `Reserved8`, so unmodified
     `doublets` 0.8 / C# `UnitedMemoryLinks` can still open the region. If space inside
     the header is unavoidable, `Reserved8` is the only free cell, but whether other
     implementations ignore a non-zero value there is **[UNVERIFIED]**.
   - Use `u64` cells. A u32 variant (32 B records) matches what the C# `clink` creates,
     but caps the store at ~2^31 links once external references are on.
   - Enable external references (`half = 2^63 − 1`) so raw numbers (block numbers, byte
     values, sizes, timestamps) can be stored inline as `-n` without consuming links.
     This also matches link-cli's names store and the binary archive.
2. **Use the SBT encoding, not C# AVL.** The `size` cells must hold plain subtree counts.
   Implement the iterative (recursionless) SBT: no recursion in kernel/driver context,
   and it is file-compatible with the recursive C# SBT. Ordering is `(source, target)`
   for the sources tree and `(target, source)` for the targets tree (§1.7).
3. **Keep the free list exactly as in §1.4:**
   - threaded through `source`/`target` of freed slots;
   - LIFO reuse (`attach_as_first` on delete, take `first_free` on create);
   - high-water-mark shrinking on deleting the last link.
   This is the existing "fill the gaps with small links" mechanism *for link slots*. The
   issue's "gaps between files filled with small doublet links" for *data blocks* is
   new; see recommendation 7.
4. **Grow by a configurable step, and shrink on clean unmount like C#.**
   - The reference step is 2^20 links (64 MiB). For an FS, make it a mount/mkfs option
     (the "configurable FS" requirement).
   - On a block device, "grow" means extending the links region into adjacent free
     extents. Preallocating contiguous regions (ext4-style) avoids fragmenting the
     region.
   - Do **not** depend on `mem-rs::FileMapped` as is: it zero-fills on reopen with
     doublets 0.8.0 (mem-rs#36). Use link-cli's `PersistentFileMapped` approach, or LFS's
     own block I/O layer.
5. **Make the journal the links DB itself, not a copy.**
   - link-cli already writes transitions to `<name>.transitions.links`, a doublets store
     of before/after link states (retention `infinite | sized:N | chunked:N:DIR`).
   - For LFS, an optional journal can be a ring of link-state transitions stored *as
     links in the same region* (or a sibling region using the same record format). That
     satisfies "embedded links DB so the journal is not duplicated".
   - Commit ordering (write the record, then the trees, then the header) needs design.
     None of the inspected stores is crash-consistent: they rely on mmap plus
     `sync_all`. **[design gap]**
6. **Use Binary Links Notation v1 as the interchange, dump, debug and wire format,
   not as the live layout.**
   - It is compact, versioned, and has 96 golden vectors and 8 codecs.
   - It preserves ids through gaps, and its store archive (§4.6) is exactly a "links +
     names" snapshot.
   - The issue's "support binary links notation, maybe exactly" is best read as: `lfs
     export`/`import` and any network protocol speak Binary v1 *exactly*, while the
     on-disk region uses the doublets record format. Binary v1 is a variable-width,
     sequential encoding and cannot support in-place random updates.
   - Reuse the `links-notation` crate (`links_notation::binary`) rather than re-implementing it.
7. **Data and files: model content as sequences of raw numbers.**
   - Prior art is `BytesToRawNumberSequenceConverter` with `BalancedVariantConverter`
     (§2.2): content becomes a balanced doublet tree whose leaves are external raw
     values. With 8-byte cells, pack 7 bytes per external leaf (63-bit payload) rather
     than 1 byte per leaf. **[design proposal]**
   - For large files, use extents: a link `(file extent_list)`, where each extent is
     `(start_block length)` with raw-number leaves. File data stays in contiguous
     blocks, ext4-like.
   - The slack after each file's last block, and small files, can hold small doublet
     links, which is the issue's gap-filling idea. That needs an allocator that hands out
     sub-block 64-byte slots. **[new work]**
8. **Bit-string indices are new work.**
   - `BitString` (§2.1) provides the API shape: 64-bit words, min/max non-zero word
     borders, And/Or/Xor, set-index enumeration and population count.
   - A persisted variant could replace the free list and serve as an
     "is used / has usages" bitmap per region.
   - The issue's "switch by usage % threshold" maps naturally to dense bitmap vs sparse
     SBT selection: for example, bitmap when more than X% of a range is set, tree
     otherwise. No existing Links Platform code does this, so the format must reserve a
     per-index "kind" tag. Put it in the LFS superblock, not in the doublets header.
9. **Choose united over split for the mandatory embedded index, at least in v1.**
   - Split gives better data locality: 16 B data records and per-link usage trees.
   - But Rust and C# currently disagree on which references go to the external trees
     (§1.3), and the Rust list mode is unfinished.
   - Split can be offered later as the "variable size indexing" option.
10. **Disk map (GitHub Pages).**
    - RawDoubletsViewer shows the minimal reader: header struct, then `fread` records
      until EOF.
    - A static web page can do the same with `DataView.getBigUint64(off, true)` at
      `64*i` offsets. It can colour records as free (in the list), live, or tree root,
      and overlay data extents from the LFS superblock.
    - Ship test images in both doublets-raw form and Binary v1 archive form.
11. **Licensing.** link-cli and links-notation are Unlicense (Public Domain compatible).
    The licenses of the linksplatform crates `doublets`, `platform-*` and `trees` were
    not checked. **[UNVERIFIED]**

---

## 6. Open questions / unverified items

- The exact tree roots after a given insert sequence (§1.8). This needs a test vector
  generated with `doublets` 0.8 (not run here, per the no-build constraint).
- Whether C# and Rust both tolerate a non-zero `Reserved8`.
- How much Rust split and C# split files diverge in practice when they hold external
  references (§1.3).
- doublets-rs `sequences`/`unicode` modules, C# `CompressingConverter`, and the frequency
  indexes: not read.
- Repos `lino-objects-codec`, `linksql`, `doublets-dev` and `Data.Doublets.Lino` were
  cloned but not analysed beyond file listings.
- Crash-consistency guarantees: none are documented in any inspected store.
