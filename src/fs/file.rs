//! File data: one contiguous extent per file.
//!
//! A file that must grow tries, in order:
//!
//! 1. growing in place with `growth-percent` extra room, then exactly;
//! 2. moving the link chunks right after it elsewhere (they are small and
//!    relocatable), then growing in place;
//! 3. moving the whole file to a free extent that fits;
//! 4. compacting the volume (if `auto-compact` is on) and trying again;
//! 5. failing with `ENOSPC`.
//!
//! Extra room is trimmed when the last handle is closed, so a file never
//! keeps more than one partial block of slack on disk.

use super::inode::{FileKind, Inode};
use super::map::Owner;
use super::{commit::arrays_of, control, now, Fs};
use crate::device::BlockDevice;
use crate::error::{Error, Result};

const COPY_BUFFER: usize = 1 << 20;

/// Result of a compaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompactReport {
    /// Files moved.
    pub files: u64,
    /// Chunks and chunk tables moved.
    pub chunks: u64,
    /// Passes (one commit each).
    pub passes: u64,
    /// Largest free extent before.
    pub largest_before: u64,
    /// Largest free extent after.
    pub largest_after: u64,
}

impl<D: BlockDevice> Fs<D> {
    fn file_inode(&self, ino: u64) -> Result<(u64, Inode)> {
        let inode = self.inode(ino)?;
        match inode.kind() {
            Some(FileKind::File) => Ok((Self::slot_of(ino)?, inode)),
            Some(FileKind::Dir) => Err(Error::IsDir),
            _ => Err(Error::invalid("not a regular file")),
        }
    }

    /// Releases an extent and forgets cached overwrites inside it.
    pub(super) fn free_extent(&mut self, start: u64, blocks: u64) {
        self.space.free(start, blocks);
        let mut tail = self.cache.split_off(&start);
        let mut rest = tail.split_off(&(start + blocks));
        self.cache.append(&mut rest);
    }

    /// Reads device bytes, overlaid with cached overwrites.
    fn read_bytes(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.device.read_at(offset, buf)?;
        if buf.is_empty() || self.cache.is_empty() {
            return Ok(());
        }
        let bs = self.block_size() as u64;
        let end = offset + buf.len() as u64;
        for (&block, data) in self.cache.range(offset / bs..end.div_ceil(bs)) {
            let block_start = block * bs;
            let from = offset.max(block_start);
            let to = end.min(block_start + bs);
            buf[(from - offset) as usize..(to - offset) as usize]
                .copy_from_slice(&data[(from - block_start) as usize..(to - block_start) as usize]);
        }
        Ok(())
    }

    /// Writes device bytes; overwrites of committed blocks are cached for
    /// the journal in full mode.
    fn write_bytes(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let journal_data = self.journals_data();
        if !journal_data && self.cache.is_empty() {
            return self.device.write_at(offset, data);
        }
        let bs = self.block_size() as u64;
        let end = offset + data.len() as u64;
        for block in offset / bs..end.div_ceil(bs) {
            let block_start = block * bs;
            let from = offset.max(block_start);
            let to = end.min(block_start + bs);
            let piece = &data[(from - offset) as usize..(to - offset) as usize];
            let cached = self.cache.contains_key(&block);
            if cached || (journal_data && !self.space.is_fresh(block)) {
                if !cached {
                    let mut current = vec![0; bs as usize];
                    self.device.read_at(block_start, &mut current)?;
                    self.cache.insert(block, current);
                }
                let entry = self.cache.get_mut(&block).expect("cached block");
                entry[(from - block_start) as usize..(to - block_start) as usize]
                    .copy_from_slice(piece);
            } else {
                self.device.write_at(from, piece)?;
            }
        }
        Ok(())
    }

    fn zero_bytes(&mut self, offset: u64, len: u64) -> Result<()> {
        let zeros = vec![0u8; COPY_BUFFER.min(len as usize)];
        let mut done = 0;
        while done < len {
            let step = (len - done).min(COPY_BUFFER as u64) as usize;
            self.write_bytes(offset + done, &zeros[..step])?;
            done += step as u64;
        }
        Ok(())
    }

    /// Reads up to `size` bytes at `offset`.
    pub fn read(&mut self, ino: u64, offset: u64, size: usize) -> Result<Vec<u8>> {
        if control::is_control(ino) {
            return self.control_read(ino, offset, size);
        }
        let (slot, mut inode) = self.file_inode(ino)?;
        if offset >= inode.size {
            return Ok(Vec::new());
        }
        let len = (inode.size - offset).min(size as u64) as usize;
        let mut buf = vec![0; len];
        let bs = self.block_size() as u64;
        self.read_bytes(inode.start * bs + offset, &mut buf)?;
        if self.config.atime && self.is_writable() {
            inode.atime = now();
            self.inodes.set(slot, &inode);
            self.dirty = true;
        }
        Ok(buf)
    }

    /// Writes `data` at `offset`, growing the file as needed.
    pub fn write(&mut self, ino: u64, offset: u64, data: &[u8]) -> Result<usize> {
        if control::is_control(ino) {
            return self.control_write(ino, offset, data);
        }
        self.check_writable()?;
        let (slot, inode) = self.file_inode(ino)?;
        if data.is_empty() {
            return Ok(0);
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| Error::invalid("offset overflow"))?;
        if end > inode.size {
            self.ensure_capacity(slot, end, true)?;
        }
        let mut inode = self.inodes.get(slot);
        let bs = self.block_size() as u64;
        if offset > inode.size {
            self.zero_bytes(inode.start * bs + inode.size, offset - inode.size)?;
        }
        self.write_bytes(inode.start * bs + offset, data)?;
        inode.size = inode.size.max(end);
        let time = now();
        inode.mtime = time;
        inode.ctime = time;
        self.inodes.set(slot, &inode);
        self.modified()?;
        Ok(data.len())
    }

    /// Sets the size of a file (zero-filling an extension).
    pub(super) fn truncate(&mut self, ino: u64, size: u64) -> Result<()> {
        let (slot, inode) = self.file_inode(ino)?;
        let bs = self.block_size() as u64;
        if size > inode.size {
            self.ensure_capacity(slot, size, false)?;
            let inode = self.inodes.get(slot);
            self.zero_bytes(inode.start * bs + inode.size, size - inode.size)?;
        }
        let mut inode = self.inodes.get(slot);
        inode.size = size;
        let time = now();
        inode.mtime = time;
        inode.ctime = time;
        self.inodes.set(slot, &inode);
        self.trim(ino)?;
        Ok(())
    }

    /// Releases the blocks a file reserves beyond its size.
    pub(super) fn trim(&mut self, ino: u64) -> Result<bool> {
        let slot = Self::slot_of(ino)?;
        let mut inode = self.inode(ino)?;
        let needed = inode.size.div_ceil(self.block_size() as u64);
        if inode.blocks <= needed {
            return Ok(false);
        }
        self.free_extent(inode.start + needed, inode.blocks - needed);
        inode.blocks = needed;
        if needed == 0 {
            inode.start = 0;
        }
        self.inodes.set(slot, &inode);
        Ok(true)
    }

    fn ensure_capacity(&mut self, slot: u64, bytes: u64, slack: bool) -> Result<()> {
        let bs = self.block_size() as u64;
        let needed = bytes.div_ceil(bs);
        if needed <= self.inodes.get(slot).blocks {
            return Ok(());
        }
        if needed > self.sb.total_blocks {
            return Err(Error::NoSpace);
        }
        let want = if slack {
            (needed + needed * u64::from(self.config.growth_percent) / 100)
                .min(self.sb.total_blocks)
        } else {
            needed
        };
        if self.try_grow(slot, needed, want)? {
            return Ok(());
        }
        if self.config.auto_compact {
            self.compact_except(Some(slot + 1))?;
            if self.try_grow(slot, needed, want)? {
                return Ok(());
            }
        }
        Err(Error::NoSpace)
    }

    fn try_grow(&mut self, slot: u64, needed: u64, want: u64) -> Result<bool> {
        let mut inode = self.inodes.get(slot);
        let fit = self.config.alloc;
        let sizes = if want > needed {
            vec![want, needed]
        } else {
            vec![needed]
        };
        if inode.blocks == 0 {
            for &len in &sizes {
                if let Some(start) = self.space.allocate(len, fit) {
                    inode.start = start;
                    inode.blocks = len;
                    self.inodes.set(slot, &inode);
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let end = inode.start + inode.blocks;
        for &len in &sizes {
            if self.space.allocate_at(end, len - inode.blocks) {
                inode.blocks = len;
                self.inodes.set(slot, &inode);
                return Ok(true);
            }
        }
        if self.evict_after(slot, needed)? {
            let mut inode = self.inodes.get(slot);
            if self.space.allocate_at(end, needed - inode.blocks) {
                inode.blocks = needed;
                self.inodes.set(slot, &inode);
                return Ok(true);
            }
        }
        for &len in &sizes {
            if let Some(start) = self.space.allocate(len, fit) {
                let inode = self.inodes.get(slot);
                self.move_file(slot, start, len)?;
                debug_assert_eq!(self.inodes.get(slot).size, inode.size);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Moves the chunks and chunk tables that block in-place growth of a
    /// file to `needed` blocks. Returns whether the blocks are now free.
    fn evict_after(&mut self, slot: u64, needed: u64) -> Result<bool> {
        let inode = self.inodes.get(slot);
        let from = inode.start + inode.blocks;
        let to = inode.start + needed;
        if to > self.sb.total_blocks {
            return Ok(false);
        }
        let blocked = self.owners().into_iter().any(|(start, len, owner)| {
            start < to
                && start + len > from
                && !matches!(owner, Owner::Chunk { .. } | Owner::Table { .. })
        });
        if blocked {
            return Ok(false);
        }
        let fit = self.config.chunk_fit();
        for array in arrays_of(&mut self.links, &mut self.inodes) {
            array.evict(inode.start, needed, &mut self.space, fit)?;
        }
        if !self.space.is_free(from, to - from) {
            // The moved chunks were committed: their blocks free up once
            // the move is committed.
            self.dirty = true;
            self.commit()?;
        }
        Ok(self.space.is_free(from, to - from))
    }

    /// Copies a file to `[start, start + blocks)` (already allocated) and
    /// releases its old extent.
    fn move_file(&mut self, slot: u64, start: u64, blocks: u64) -> Result<()> {
        let mut inode = self.inodes.get(slot);
        let bs = self.block_size() as u64;
        let mut buf = vec![0u8; COPY_BUFFER];
        let mut done = 0;
        while done < inode.size {
            let step = (inode.size - done).min(COPY_BUFFER as u64) as usize;
            self.read_bytes(inode.start * bs + done, &mut buf[..step])?;
            self.write_bytes(start * bs + done, &buf[..step])?;
            done += step as u64;
        }
        if inode.blocks > 0 {
            self.free_extent(inode.start, inode.blocks);
        }
        inode.start = start;
        inode.blocks = blocks;
        self.inodes.set(slot, &inode);
        self.dirty = true;
        Ok(())
    }

    /// How high the free space sits: the sum of the addresses of the free
    /// blocks. Moving data down raises it, so compaction stops as soon as
    /// a round fails to raise it, which bounds the work.
    fn free_height(&self) -> u128 {
        self.space
            .free_extents()
            .map(|(start, len)| {
                let (start, len) = (u128::from(start), u128::from(len));
                start * len + len * len.saturating_sub(1) / 2
            })
            .sum()
    }

    /// Moves the `len` blocks at `start`, owned by `owner`, to the already
    /// free `target`. Returns whether they moved.
    fn move_owner(
        &mut self,
        owner: Owner,
        len: u64,
        target: u64,
        report: &mut CompactReport,
    ) -> Result<bool> {
        let done = match owner {
            Owner::File { ino } => {
                let slot = Self::slot_of(ino)?;
                if !self.space.allocate_at(target, len) {
                    return Ok(false);
                }
                self.move_file(slot, target, len)?;
                report.files += 1;
                return Ok(true);
            }
            Owner::Chunk { array, index } => {
                let arrays = arrays_of(&mut self.links, &mut self.inodes);
                arrays
                    .into_iter()
                    .nth(array)
                    .is_some_and(|array| array.relocate_to(index, target, &mut self.space))
            }
            Owner::Table { array } => {
                let arrays = arrays_of(&mut self.links, &mut self.inodes);
                arrays
                    .into_iter()
                    .nth(array)
                    .is_some_and(|array| array.relocate_table_to(target, &mut self.space))
            }
            Owner::Superblock | Owner::Journal | Owner::Free | Owner::Pending => false,
        };
        report.chunks += u64::from(done);
        Ok(done)
    }

    /// Moves everything into the lowest free extent below it that fits, one
    /// commit per pass, until nothing moves.
    fn settle(&mut self, skip: Option<u64>, report: &mut CompactReport) -> Result<()> {
        for _ in 0..256 {
            let mut moved = false;
            for (start, len, owner) in self.owners() {
                if matches!(owner, Owner::File { ino } if Some(ino) == skip) {
                    continue;
                }
                let target = self
                    .space
                    .free_extents()
                    .take_while(|&(free, _)| free < start)
                    .find(|&(_, free_len)| free_len >= len)
                    .map(|(free, _)| free);
                if let Some(target) = target {
                    moved |= self.move_owner(owner, len, target, report)?;
                }
            }
            if !moved {
                break;
            }
            report.passes += 1;
            self.dirty = true;
            self.commit()?;
        }
        Ok(())
    }

    /// Unsticks the lowest gap that is smaller than the object right after
    /// it by moving that object up into the last free extent, so the gap
    /// grows by the object's place and the next settle fills it (copying
    /// twice instead of overlapping, which a crash could tear). Returns
    /// whether something moved.
    fn bounce(&mut self, skip: Option<u64>, report: &mut CompactReport) -> Result<bool> {
        let free: Vec<(u64, u64)> = self.space.free_extents().collect();
        let Some((&(tail, tail_len), gaps)) = free.split_last() else {
            return Ok(false);
        };
        let owners = self.owners();
        for &(gap, gap_len) in gaps {
            let Some(&(_, len, owner)) =
                owners.iter().find(|&&(start, _, _)| start == gap + gap_len)
            else {
                continue;
            };
            if len > tail_len || matches!(owner, Owner::File { ino } if Some(ino) == skip) {
                continue;
            }
            if self.move_owner(owner, len, tail, report)? {
                report.passes += 1;
                self.dirty = true;
                self.commit()?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Moves every file, chunk and chunk table into the lowest free extent
    /// that fits, until nothing moves, and moves objects stuck behind a gap
    /// smaller than them out of the way, so all free space ends up in one
    /// extent at the end of the volume. Without a journal every commit moves
    /// the chunks it changes to fresh blocks, which leaves a few small gaps.
    pub fn compact(&mut self) -> Result<CompactReport> {
        self.compact_except(None)
    }

    fn compact_except(&mut self, skip: Option<u64>) -> Result<CompactReport> {
        self.check_writable()?;
        let mut report = CompactReport {
            largest_before: self.space.largest_free(),
            ..CompactReport::default()
        };
        let files: Vec<u64> = self
            .inodes
            .each()
            .filter(|(_, inode)| inode.kind() == Some(FileKind::File))
            .map(|(slot, _)| slot + 1)
            .filter(|&ino| Some(ino) != skip)
            .collect();
        for ino in files {
            self.dirty |= self.trim(ino)?;
        }
        self.dirty = true;
        self.commit()?;
        self.settle(skip, &mut report)?;
        let mut height = self.free_height();
        for _ in 0..256 {
            if self.space.free_extents().nth(1).is_none() || !self.bounce(skip, &mut report)? {
                break;
            }
            self.settle(skip, &mut report)?;
            let raised = self.free_height();
            if raised <= height {
                break;
            }
            height = raised;
        }
        report.largest_after = self.space.largest_free();
        Ok(report)
    }
}
