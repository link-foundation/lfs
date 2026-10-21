//! Commit: one atomic transaction for the file system and the links
//! database together.
//!
//! 1. New chunks get blocks (link chunks go into the smallest gaps when
//!    `gap-fill` is on).
//! 2. Blocks allocated since the last commit ("fresh") are written in place:
//!    no committed structure refers to them, so a crash cannot expose them.
//! 3. Overwrites of committed blocks and the new superblock go through the
//!    journal, then are checkpointed in place, superblock last. If they do
//!    not fit, the dirty chunks are first moved to fresh blocks
//!    (copy-on-write), which leaves only the chunk tables and the superblock
//!    to journal.
//! 4. Without a journal every dirty chunk and table is moved to fresh blocks
//!    instead, so only the superblock is overwritten in place.

use super::inode::Inodes;
use super::Fs;
use crate::chunked::{BlockWrite, ChunkedArray};
use crate::config::JournalMode;
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::journal;
use crate::links::Links;

/// Every chunked array of the volume, in superblock slot order.
pub(super) fn arrays_of<'a>(
    links: &'a mut Links,
    inodes: &'a mut Inodes,
) -> Vec<&'a mut ChunkedArray> {
    let mut arrays: Vec<&mut ChunkedArray> = vec![
        &mut links.records.data,
        &mut links.records.index,
        &mut inodes.array,
        &mut links.usage.handles,
    ];
    arrays.extend(links.usage.var.arrays_mut().iter_mut());
    arrays
}

impl<D: BlockDevice> Fs<D> {
    /// Every chunked array, in superblock slot order.
    pub(super) fn arrays(&self) -> Vec<&ChunkedArray> {
        let mut arrays: Vec<&ChunkedArray> = vec![
            &self.links.records.data,
            &self.links.records.index,
            &self.inodes.array,
            &self.links.usage.handles,
        ];
        arrays.extend(self.links.usage.var.arrays().iter());
        arrays
    }

    /// Whether commits go through the journal.
    pub(super) const fn journaling(&self) -> bool {
        self.journal.exists() && !matches!(self.config.journal, JournalMode::Off) && self.sb.seq > 0
    }

    /// Whether overwrites of committed file data are journaled.
    pub(super) const fn journals_data(&self) -> bool {
        self.journaling() && matches!(self.config.journal, JournalMode::Full)
    }

    fn live_dirty(&self) -> u64 {
        self.arrays()
            .iter()
            .map(|array| array.live_dirty_blocks(&self.space))
            .sum()
    }

    fn place_all(&mut self) -> Result<()> {
        let fit = self.config.chunk_fit();
        for array in arrays_of(&mut self.links, &mut self.inodes) {
            array.place(&mut self.space, fit)?;
        }
        Ok(())
    }

    fn cow_all(&mut self, with_table: bool) -> Result<()> {
        let fit = self.config.chunk_fit();
        for array in arrays_of(&mut self.links, &mut self.inodes) {
            array.cow_live(&mut self.space, fit, with_table)?;
        }
        Ok(())
    }

    /// Makes every change durable as one transaction.
    pub fn commit(&mut self) -> Result<()> {
        if !self.writable || (!self.dirty && self.cache.is_empty()) {
            return Ok(());
        }
        self.links.shrink(&mut self.space);
        self.inodes.shrink(&mut self.space);
        self.links.sync_header();
        self.place_all()?;
        let journaling = self.journaling();
        if journaling {
            let capacity = self.journal.capacity();
            if self.live_dirty() + 1 > capacity {
                self.cow_all(false)?;
            }
            if self.live_dirty() + 1 > capacity {
                self.cow_all(true)?;
            }
            if self.live_dirty() + 1 > capacity {
                return Err(Error::NoSpace);
            }
        } else {
            // Without a journal, moving every dirty chunk and table to fresh
            // blocks (shadow paging) leaves the superblock as the only block
            // overwritten in place, so a crash still shows the old or the new
            // state. A volume too full for that falls back to in-place writes.
            let _ = self.cow_all(true);
        }
        let mut sb = self.sb.clone();
        sb.seq += 1;
        sb.links = self.links.header();
        let mut writes: Vec<BlockWrite> = Vec::new();
        let mut descriptors = Vec::new();
        for array in arrays_of(&mut self.links, &mut self.inodes) {
            descriptors.push(array.descriptor());
            array.take_writes(&mut writes);
        }
        sb.arrays.copy_from_slice(&descriptors);
        let superblock = (0, sb.encode());
        let cache: Vec<BlockWrite> = std::mem::take(&mut self.cache).into_iter().collect();
        if journaling {
            let (fresh, mut live): (Vec<BlockWrite>, Vec<BlockWrite>) = writes
                .into_iter()
                .partition(|(block, _)| self.space.is_fresh(*block));
            let device: &mut dyn BlockDevice = &mut self.device;
            for (block, data) in &fresh {
                journal::write_block(device, *block, data)?;
            }
            let capacity = self.journal.capacity() as usize;
            if live.len() + 1 + cache.len() <= capacity {
                live.extend(cache);
            } else {
                // Too much file data for one transaction: journal it in
                // pieces first. Each piece is replayed idempotently.
                for piece in cache.chunks(capacity) {
                    self.journal.commit(device, sb.seq, piece)?;
                    journal::checkpoint(device, piece)?;
                }
            }
            live.push(superblock);
            self.journal.commit(device, sb.seq, &live)?;
            journal::checkpoint(device, &live)?;
            self.journal_clean = false;
        } else {
            if self.journal.exists() && !self.journal_clean {
                self.journal.wipe(&mut self.device)?;
                self.journal_clean = true;
            }
            writes.extend(cache);
            writes.push(superblock);
            journal::checkpoint(&mut self.device, &writes)?;
        }
        self.sb = sb;
        self.space.commit();
        self.dirty = false;
        self.ops = 0;
        Ok(())
    }
}
