//! Relocatable arrays of fixed-size records.
//!
//! Links, inodes, usage handles and every size class of the variable-size
//! store are [`ChunkedArray`]s: records live in small chunks (one block, or
//! one record when records are larger than a block) that can sit anywhere on
//! the device. Chunks are what fills the gaps between contiguous files, and
//! they are "liquid": the allocator may move a chunk to let a file grow in
//! place. A chunk table (one `u64` block number per chunk) stored in its own
//! contiguous extent locates the chunks.
//!
//! New chunks get their blocks only at commit (delayed allocation), so the
//! allocator sees the final set of new chunks at once and places each of
//! them in the smallest gap.

use crate::codec::{get_u64, put_u64};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::space::{Fit, Space};

/// Block number of a chunk that has not been placed yet.
pub const UNPLACED: u64 = u64::MAX;

/// Zeros returned for out-of-range records (the largest record is 1 MiB).
///
/// Allocated on first use rather than a static array: a 1 MiB zero static
/// is stored verbatim in the WebAssembly data section.
fn zeros(len: usize) -> &'static [u8] {
    static ZEROS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    &ZEROS.get_or_init(|| vec![0; 1 << 20])[..len]
}

/// Where a chunk table lives on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArrayDescriptor {
    /// First block of the chunk table extent.
    pub table_start: u64,
    /// Blocks reserved for the chunk table.
    pub table_blocks: u64,
    /// Number of chunks in use.
    pub chunks: u64,
}

impl ArrayDescriptor {
    /// Encoded size in bytes.
    pub const SIZE: usize = 24;

    /// Encodes the descriptor.
    pub fn write(&self, bytes: &mut [u8]) {
        put_u64(bytes, 0, self.table_start);
        put_u64(bytes, 8, self.table_blocks);
        put_u64(bytes, 16, self.chunks);
    }

    /// Decodes a descriptor.
    #[must_use]
    pub fn read(bytes: &[u8]) -> Self {
        Self {
            table_start: get_u64(bytes, 0),
            table_blocks: get_u64(bytes, 8),
            chunks: get_u64(bytes, 16),
        }
    }
}

#[derive(Clone, Debug)]
struct Chunk {
    block: u64,
    data: Vec<u8>,
    dirty: bool,
}

/// A growable array of `record_size`-byte records stored in relocatable chunks.
#[derive(Clone, Debug)]
pub struct ChunkedArray {
    record_size: usize,
    block_size: usize,
    chunk_blocks: u64,
    per_chunk: usize,
    chunks: Vec<Chunk>,
    table_start: u64,
    table_blocks: u64,
    table_dirty: bool,
}

/// One block that must be written at commit.
pub type BlockWrite = (u64, Vec<u8>);

impl ChunkedArray {
    /// An empty array.
    #[must_use]
    pub fn new(record_size: usize, block_size: usize) -> Self {
        let chunk_bytes = record_size.max(block_size);
        let chunk_blocks = chunk_bytes.div_ceil(block_size) as u64;
        Self {
            record_size,
            block_size,
            chunk_blocks,
            per_chunk: (chunk_blocks as usize * block_size) / record_size,
            chunks: Vec::new(),
            table_start: 0,
            table_blocks: 0,
            table_dirty: false,
        }
    }

    /// Bytes per record.
    #[must_use]
    pub const fn record_size(&self) -> usize {
        self.record_size
    }

    /// Records per chunk.
    #[must_use]
    pub const fn per_chunk(&self) -> usize {
        self.per_chunk
    }

    /// Blocks per chunk.
    #[must_use]
    pub const fn chunk_blocks(&self) -> u64 {
        self.chunk_blocks
    }

    /// Number of records that fit in the allocated chunks.
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        (self.chunks.len() * self.per_chunk) as u64
    }

    /// Number of chunks.
    #[must_use]
    pub const fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Blocks needed for chunks that have not been placed yet.
    #[must_use]
    pub fn unplaced_blocks(&self) -> u64 {
        self.chunks
            .iter()
            .filter(|chunk| chunk.block == UNPLACED)
            .count() as u64
            * self.chunk_blocks
            + self.table_growth_blocks()
    }

    const fn table_needed_blocks(&self) -> u64 {
        ((self.chunks.len() * 8).div_ceil(self.block_size)) as u64
    }

    const fn table_growth_blocks(&self) -> u64 {
        let needed = self.table_needed_blocks();
        if needed > self.table_blocks {
            needed.next_power_of_two()
        } else {
            0
        }
    }

    /// Makes room for at least `records` records (new chunks are unplaced).
    pub fn ensure_capacity(&mut self, records: u64) {
        while self.capacity() < records {
            self.chunks.push(Chunk {
                block: UNPLACED,
                data: vec![0; self.chunk_blocks as usize * self.block_size],
                dirty: true,
            });
            self.table_dirty = true;
        }
    }

    /// Drops trailing chunks beyond `records` (their blocks are released).
    pub fn truncate_chunks(&mut self, records: u64, space: &mut Space) {
        let keep = (records as usize).div_ceil(self.per_chunk);
        while self.chunks.len() > keep {
            let chunk = self.chunks.pop().expect("chunk exists");
            if chunk.block != UNPLACED {
                space.free(chunk.block, self.chunk_blocks);
            }
            self.table_dirty = true;
        }
    }

    const fn locate(&self, index: u64) -> (usize, usize) {
        let index = index as usize;
        (
            index / self.per_chunk,
            (index % self.per_chunk) * self.record_size,
        )
    }

    /// Record `index`. Indices at or above [`capacity`](Self::capacity)
    /// (only reachable through corrupt references) read as zeros.
    #[must_use]
    pub fn record(&self, index: u64) -> &[u8] {
        let (chunk, offset) = self.locate(index);
        self.chunks.get(chunk).map_or_else(
            || zeros(self.record_size),
            |chunk| &chunk.data[offset..offset + self.record_size],
        )
    }

    /// Mutable record `index`; marks its chunk dirty.
    pub fn record_mut(&mut self, index: u64) -> &mut [u8] {
        let (chunk, offset) = self.locate(index);
        let chunk = &mut self.chunks[chunk];
        chunk.dirty = true;
        &mut chunk.data[offset..offset + self.record_size]
    }

    /// Reads a `u64` field of a record.
    #[must_use]
    pub fn get(&self, index: u64, field_offset: usize) -> u64 {
        get_u64(self.record(index), field_offset)
    }

    /// Writes a `u64` field of a record.
    pub fn set(&mut self, index: u64, field_offset: usize, value: u64) {
        put_u64(self.record_mut(index), field_offset, value);
    }

    /// Block of each chunk (`UNPLACED` for new ones).
    pub fn chunk_blocks_iter(&self) -> impl Iterator<Item = u64> + '_ {
        self.chunks.iter().map(|chunk| chunk.block)
    }

    /// The chunk table extent `(start, blocks)`.
    #[must_use]
    pub const fn table_extent(&self) -> (u64, u64) {
        (self.table_start, self.table_blocks)
    }

    /// Finds the chunk stored at `block`.
    #[must_use]
    pub fn chunk_at(&self, block: u64) -> Option<usize> {
        self.chunks.iter().position(|chunk| chunk.block == block)
    }

    /// Moves chunk `index` to a new location chosen by the allocator,
    /// avoiding `avoid`. The old blocks are released (pending until commit).
    pub fn relocate(
        &mut self,
        index: usize,
        space: &mut Space,
        fit: Fit,
        avoid: Option<(u64, u64)>,
    ) -> Result<()> {
        let new_block = space
            .allocate_avoiding(self.chunk_blocks, fit, avoid)
            .ok_or(Error::NoSpace)?;
        let chunk = &mut self.chunks[index];
        if chunk.block != UNPLACED {
            space.free(chunk.block, self.chunk_blocks);
        }
        chunk.block = new_block;
        chunk.dirty = true;
        self.table_dirty = true;
        Ok(())
    }

    /// Moves the chunk table to a new extent, releasing the old one.
    pub fn relocate_table(
        &mut self,
        space: &mut Space,
        fit: Fit,
        avoid: Option<(u64, u64)>,
    ) -> Result<()> {
        if self.table_blocks == 0 {
            return Ok(());
        }
        let new_start = space
            .allocate_avoiding(self.table_blocks, fit, avoid)
            .ok_or(Error::NoSpace)?;
        space.free(self.table_start, self.table_blocks);
        self.table_start = new_start;
        self.table_dirty = true;
        Ok(())
    }

    /// Moves chunk `index` to exactly `block` if that range is free.
    pub fn relocate_to(&mut self, index: usize, block: u64, space: &mut Space) -> bool {
        if !space.allocate_at(block, self.chunk_blocks) {
            return false;
        }
        let chunk = &mut self.chunks[index];
        if chunk.block != UNPLACED {
            space.free(chunk.block, self.chunk_blocks);
        }
        chunk.block = block;
        chunk.dirty = true;
        self.table_dirty = true;
        true
    }

    /// Moves the chunk table to exactly `start` if that range is free.
    pub fn relocate_table_to(&mut self, start: u64, space: &mut Space) -> bool {
        if self.table_blocks == 0 || !space.allocate_at(start, self.table_blocks) {
            return false;
        }
        space.free(self.table_start, self.table_blocks);
        self.table_start = start;
        self.table_dirty = true;
        true
    }

    /// Moves every chunk and the table overlapping `[start, start + len)`
    /// out of that range. Returns whether anything moved.
    pub fn evict(&mut self, start: u64, len: u64, space: &mut Space, fit: Fit) -> Result<bool> {
        let overlaps = |block: u64, blocks: u64| block < start + len && block + blocks > start;
        let mut moved = false;
        for index in 0..self.chunks.len() {
            let block = self.chunks[index].block;
            if block != UNPLACED && overlaps(block, self.chunk_blocks) {
                self.relocate(index, space, fit, Some((start, len)))?;
                moved = true;
            }
        }
        if self.table_blocks > 0 && overlaps(self.table_start, self.table_blocks) {
            self.relocate_table(space, fit, Some((start, len)))?;
            moved = true;
        }
        Ok(moved)
    }

    /// Places new chunks and grows the table if needed.
    pub fn place(&mut self, space: &mut Space, fit: Fit) -> Result<()> {
        for index in 0..self.chunks.len() {
            if self.chunks[index].block == UNPLACED {
                let block = space
                    .allocate(self.chunk_blocks, fit)
                    .ok_or(Error::NoSpace)?;
                self.chunks[index].block = block;
                self.table_dirty = true;
            }
        }
        let needed = self.table_needed_blocks();
        if needed > self.table_blocks || (needed == 0 && self.table_blocks > 0) {
            if self.table_blocks > 0 {
                space.free(self.table_start, self.table_blocks);
            }
            if needed == 0 {
                self.table_start = 0;
                self.table_blocks = 0;
            } else {
                let blocks = needed.next_power_of_two();
                self.table_start = space.allocate(blocks, fit).ok_or(Error::NoSpace)?;
                self.table_blocks = blocks;
            }
            self.table_dirty = true;
        }
        Ok(())
    }

    /// Collects every dirty block (chunks and table) and clears dirty flags.
    /// [`place`](Self::place) must have been called first.
    pub fn take_writes(&mut self, out: &mut Vec<BlockWrite>) {
        let block_size = self.block_size;
        for chunk in &mut self.chunks {
            if chunk.dirty {
                debug_assert_ne!(chunk.block, UNPLACED);
                for (index, data) in chunk.data.chunks(block_size).enumerate() {
                    out.push((chunk.block + index as u64, data.to_vec()));
                }
                chunk.dirty = false;
            }
        }
        if self.table_dirty {
            let mut table = vec![0u8; self.table_blocks as usize * block_size];
            for (index, chunk) in self.chunks.iter().enumerate() {
                put_u64(&mut table, index * 8, chunk.block);
            }
            for (index, data) in table.chunks(block_size).enumerate() {
                out.push((self.table_start + index as u64, data.to_vec()));
            }
            self.table_dirty = false;
        }
    }

    /// Number of dirty blocks that overwrite committed (non-fresh) blocks.
    #[must_use]
    pub fn live_dirty_blocks(&self, space: &Space) -> u64 {
        let chunks = self
            .chunks
            .iter()
            .filter(|chunk| chunk.dirty && chunk.block != UNPLACED && !space.is_fresh(chunk.block))
            .count() as u64
            * self.chunk_blocks;
        let table =
            if self.table_dirty && self.table_blocks > 0 && !space.is_fresh(self.table_start) {
                self.table_blocks
            } else {
                0
            };
        chunks + table
    }

    /// Copy-on-write: moves every dirty chunk (and the table when
    /// `with_table`) that sits on committed blocks to fresh blocks, so the
    /// next commit does not overwrite live data in place.
    pub fn cow_live(&mut self, space: &mut Space, fit: Fit, with_table: bool) -> Result<()> {
        for index in 0..self.chunks.len() {
            let chunk = &self.chunks[index];
            if chunk.dirty && chunk.block != UNPLACED && !space.is_fresh(chunk.block) {
                self.relocate(index, space, fit, None)?;
            }
        }
        if with_table
            && self.table_dirty
            && self.table_blocks > 0
            && !space.is_fresh(self.table_start)
        {
            self.relocate_table(space, fit, None)?;
        }
        Ok(())
    }

    /// Whether anything must be written at the next commit.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.table_dirty || self.chunks.iter().any(|chunk| chunk.dirty)
    }

    /// Marks every chunk and the table dirty (used after a full rewrite).
    pub fn mark_all_dirty(&mut self) {
        self.table_dirty = true;
        for chunk in &mut self.chunks {
            chunk.dirty = true;
        }
    }

    /// The descriptor stored in the superblock.
    #[must_use]
    pub const fn descriptor(&self) -> ArrayDescriptor {
        ArrayDescriptor {
            table_start: self.table_start,
            table_blocks: self.table_blocks,
            chunks: self.chunks.len() as u64,
        }
    }

    /// Loads an array from disk and marks its blocks used in `space`.
    pub fn load(
        device: &mut dyn BlockDevice,
        descriptor: ArrayDescriptor,
        record_size: usize,
        block_size: usize,
        space: &mut Space,
    ) -> Result<Self> {
        let mut array = Self::new(record_size, block_size);
        array.table_start = descriptor.table_start;
        array.table_blocks = descriptor.table_blocks;
        if descriptor.table_blocks == 0 {
            if descriptor.chunks > 0 {
                return Err(Error::corrupt("chunks without a chunk table"));
            }
            return Ok(array);
        }
        if descriptor.chunks * 8 > descriptor.table_blocks * block_size as u64 {
            return Err(Error::corrupt("chunk table too small"));
        }
        if !space.mark_used(descriptor.table_start, descriptor.table_blocks) {
            return Err(Error::corrupt(format!(
                "chunk table at block {} overlaps another owner",
                descriptor.table_start
            )));
        }
        let mut table = vec![0u8; descriptor.table_blocks as usize * block_size];
        device.read_at(descriptor.table_start * block_size as u64, &mut table)?;
        for index in 0..descriptor.chunks as usize {
            let block = get_u64(&table, index * 8);
            if !space.mark_used(block, array.chunk_blocks) {
                return Err(Error::corrupt(format!(
                    "chunk at block {block} overlaps another owner"
                )));
            }
            let mut data = vec![0u8; array.chunk_blocks as usize * block_size];
            device.read_at(block * block_size as u64, &mut data)?;
            array.chunks.push(Chunk {
                block,
                data,
                dirty: false,
            });
        }
        Ok(array)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::MemDevice;

    #[test]
    fn chunks_are_placed_at_commit_and_reload() {
        let block_size = 512;
        let mut space = Space::new(64);
        assert!(space.mark_used(0, 1));
        let mut array = ChunkedArray::new(64, block_size);
        array.ensure_capacity(20);
        assert_eq!(array.chunk_count(), 3);
        assert_eq!(array.unplaced_blocks(), 3 + 1);
        array.set(17, 8, 42);
        array.place(&mut space, Fit::Best).unwrap();
        let mut writes = Vec::new();
        array.take_writes(&mut writes);
        assert_eq!(writes.len(), 4);
        assert!(!array.is_dirty());
        let mut device = MemDevice::new(64 * block_size as u64);
        for (block, data) in &writes {
            device.write_at(block * block_size as u64, data).unwrap();
        }
        let mut space2 = Space::new(64);
        assert!(space2.mark_used(0, 1));
        let loaded =
            ChunkedArray::load(&mut device, array.descriptor(), 64, block_size, &mut space2)
                .unwrap();
        assert_eq!(loaded.get(17, 8), 42);
        assert_eq!(space2.free_blocks(), space.free_blocks());
    }

    #[test]
    fn large_records_span_several_blocks() {
        let array = ChunkedArray::new(4096, 1024);
        assert_eq!(array.chunk_blocks(), 4);
        assert_eq!(array.per_chunk(), 1);
    }

    #[test]
    fn relocation_moves_a_chunk_out_of_the_way() {
        let mut space = Space::new(16);
        let mut array = ChunkedArray::new(64, 512);
        array.ensure_capacity(1);
        array.place(&mut space, Fit::First).unwrap();
        let first = array.chunk_blocks_iter().next().unwrap();
        array
            .relocate(0, &mut space, Fit::First, Some((first, 4)))
            .unwrap();
        let moved = array.chunk_blocks_iter().next().unwrap();
        assert!(moved >= first + 4);
        assert_eq!(array.chunk_at(moved), Some(0));
    }
}
