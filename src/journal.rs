//! The optional write-ahead journal shared by the file system and the links
//! database.
//!
//! The journal is a fixed region (blocks `start..start + blocks`) holding at
//! most one transaction at a time, always written from the region start:
//!
//! ```text
//! descriptor block(s)  "LFSJDESC" seq count | target block numbers...
//! data blocks          new contents of each target, in order
//! commit block         "LFSJCMIT" seq count crc32(descriptors + data)
//! ```
//!
//! A transaction is durable once its commit block is on disk. On mount it is
//! replayed if it is valid and its sequence number is the superblock's plus
//! one (or the superblock is torn and the transaction carries block 0).
//! Replaying is idempotent, so a crash during replay is harmless. Because the
//! superblock (block 0) is always checkpointed last, a superblock carrying
//! the new sequence number implies every other block is checkpointed too.

use crate::chunked::BlockWrite;
use crate::codec::{crc32_update, get_u32, get_u64, put_u32, put_u64};
use crate::device::BlockDevice;
use crate::error::{Error, Result};

/// Magic of descriptor blocks.
pub const DESCRIPTOR_MAGIC: &[u8; 8] = b"LFSJDESC";
/// Magic of commit blocks.
pub const COMMIT_MAGIC: &[u8; 8] = b"LFSJCMIT";
const DESCRIPTOR_HEADER: usize = 24;

/// A journal region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Journal {
    /// First block of the region.
    pub start: u64,
    /// Region length in blocks (0: no journal).
    pub blocks: u64,
    /// Block size in bytes.
    pub block_size: usize,
}

/// A transaction read back from the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    /// Sequence number.
    pub seq: u64,
    /// Target blocks and their contents.
    pub writes: Vec<BlockWrite>,
}

impl Transaction {
    /// Whether the transaction rewrites block `block`.
    #[must_use]
    pub fn touches(&self, block: u64) -> bool {
        self.writes.iter().any(|(target, _)| *target == block)
    }
}

/// Reads one block.
pub fn read_block(device: &mut dyn BlockDevice, block: u64, block_size: usize) -> Result<Vec<u8>> {
    let mut data = vec![0; block_size];
    device.read_at(block * block_size as u64, &mut data)?;
    Ok(data)
}

/// Writes one block.
pub fn write_block(device: &mut dyn BlockDevice, block: u64, data: &[u8]) -> Result<()> {
    device.write_at(block * data.len() as u64, data)
}

impl Journal {
    /// Default journal length for a device of `total` blocks.
    #[must_use]
    pub fn default_blocks(total: u64) -> u64 {
        (total / 64).clamp(16, 4096).min(total / 4)
    }

    /// Whether the region exists.
    #[must_use]
    pub const fn exists(&self) -> bool {
        self.blocks >= 3
    }

    const fn targets_per_descriptor(&self) -> u64 {
        ((self.block_size - DESCRIPTOR_HEADER) / 8) as u64
    }

    const fn descriptor_blocks(&self, count: u64) -> u64 {
        if count == 0 {
            1
        } else {
            count.div_ceil(self.targets_per_descriptor())
        }
    }

    /// The largest number of blocks one transaction can carry.
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        if !self.exists() {
            return 0;
        }
        let mut count = self.blocks - 2;
        while count > 0 && self.descriptor_blocks(count) + count + 1 > self.blocks {
            count -= 1;
        }
        count
    }

    /// Writes and commits a transaction (two syncs: before and after the
    /// commit block).
    pub fn commit(
        &self,
        device: &mut dyn BlockDevice,
        seq: u64,
        writes: &[BlockWrite],
    ) -> Result<()> {
        let count = writes.len() as u64;
        if count > self.capacity() {
            return Err(Error::invalid(format!(
                "transaction of {count} blocks exceeds the journal capacity {}",
                self.capacity()
            )));
        }
        let descriptors = self.descriptor_blocks(count);
        let per = self.targets_per_descriptor() as usize;
        let mut crc = 0;
        for index in 0..descriptors {
            let mut block = vec![0u8; self.block_size];
            block[..8].copy_from_slice(DESCRIPTOR_MAGIC);
            put_u64(&mut block, 8, seq);
            put_u64(&mut block, 16, count);
            let first = index as usize * per;
            for (slot, (target, _)) in writes.iter().skip(first).take(per).enumerate() {
                put_u64(&mut block, DESCRIPTOR_HEADER + slot * 8, *target);
            }
            crc = crc32_update(crc, &block);
            write_block(device, self.start + index, &block)?;
        }
        for (index, (_, data)) in writes.iter().enumerate() {
            debug_assert_eq!(data.len(), self.block_size);
            crc = crc32_update(crc, data);
            write_block(device, self.start + descriptors + index as u64, data)?;
        }
        device.sync()?;
        let mut block = vec![0u8; self.block_size];
        block[..8].copy_from_slice(COMMIT_MAGIC);
        put_u64(&mut block, 8, seq);
        put_u64(&mut block, 16, count);
        put_u32(&mut block, 24, crc);
        write_block(device, self.start + descriptors + count, &block)?;
        device.sync()
    }

    /// Reads the committed transaction, if the region holds a valid one.
    pub fn read(&self, device: &mut dyn BlockDevice) -> Result<Option<Transaction>> {
        if !self.exists() {
            return Ok(None);
        }
        let first = read_block(device, self.start, self.block_size)?;
        if &first[..8] != DESCRIPTOR_MAGIC {
            return Ok(None);
        }
        let seq = get_u64(&first, 8);
        let count = get_u64(&first, 16);
        if count > self.capacity() {
            return Ok(None);
        }
        let descriptors = self.descriptor_blocks(count);
        let per = self.targets_per_descriptor() as usize;
        let mut crc = 0;
        let mut targets = Vec::with_capacity(count as usize);
        for index in 0..descriptors {
            let block = if index == 0 {
                first.clone()
            } else {
                read_block(device, self.start + index, self.block_size)?
            };
            if &block[..8] != DESCRIPTOR_MAGIC
                || get_u64(&block, 8) != seq
                || get_u64(&block, 16) != count
            {
                return Ok(None);
            }
            crc = crc32_update(crc, &block);
            let remaining = count as usize - targets.len();
            for slot in 0..remaining.min(per) {
                targets.push(get_u64(&block, DESCRIPTOR_HEADER + slot * 8));
            }
        }
        let mut writes = Vec::with_capacity(targets.len());
        for (index, target) in targets.into_iter().enumerate() {
            let data = read_block(
                device,
                self.start + descriptors + index as u64,
                self.block_size,
            )?;
            crc = crc32_update(crc, &data);
            writes.push((target, data));
        }
        let commit = read_block(device, self.start + descriptors + count, self.block_size)?;
        let valid = &commit[..8] == COMMIT_MAGIC
            && get_u64(&commit, 8) == seq
            && get_u64(&commit, 16) == count
            && get_u32(&commit, 24) == crc;
        Ok(valid.then_some(Transaction { seq, writes }))
    }

    /// Clears the region so no stale transaction survives a reformat.
    pub fn wipe(&self, device: &mut dyn BlockDevice) -> Result<()> {
        if self.blocks > 0 {
            write_block(device, self.start, &vec![0u8; self.block_size])?;
        }
        Ok(())
    }
}

/// Writes blocks in place, block 0 last with a sync before it, so a new
/// superblock implies every other block is already on disk.
pub fn checkpoint(device: &mut dyn BlockDevice, writes: &[BlockWrite]) -> Result<()> {
    let mut superblock = None;
    for (target, data) in writes {
        if *target == 0 {
            superblock = Some(data);
        } else {
            write_block(device, *target, data)?;
        }
    }
    if let Some(data) = superblock {
        device.sync()?;
        write_block(device, 0, data)?;
    }
    device.sync()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{CrashDevice, MemDevice};

    fn journal() -> Journal {
        Journal {
            start: 1,
            blocks: 8,
            block_size: 512,
        }
    }

    #[test]
    fn transactions_round_trip() {
        let mut device = MemDevice::new(512 * 32);
        let journal = journal();
        assert_eq!(journal.read(&mut device).unwrap(), None);
        let writes: Vec<BlockWrite> = (0..5).map(|i| (20 + i, vec![i as u8 + 1; 512])).collect();
        journal.commit(&mut device, 9, &writes).unwrap();
        let tx = journal.read(&mut device).unwrap().unwrap();
        assert_eq!(tx.seq, 9);
        assert_eq!(tx.writes, writes);
        assert!(journal
            .commit(&mut device, 10, &vec![(1, vec![0; 512]); 7])
            .is_err());
        assert_eq!(journal.capacity(), 6);
    }

    #[test]
    fn many_targets_span_several_descriptors() {
        let journal = Journal {
            start: 0,
            blocks: 200,
            block_size: 512,
        };
        let mut device = MemDevice::new(512 * 400);
        let capacity = journal.capacity();
        assert!(capacity > journal.targets_per_descriptor());
        let writes: Vec<BlockWrite> = (0..capacity)
            .map(|i| (300 + i % 50, vec![(i % 251) as u8; 512]))
            .collect();
        journal.commit(&mut device, 1, &writes).unwrap();
        assert_eq!(journal.read(&mut device).unwrap().unwrap().writes, writes);
    }

    #[test]
    fn a_torn_transaction_is_ignored() {
        let journal = journal();
        let writes: Vec<BlockWrite> = (0..3).map(|i| (20 + i, vec![7; 512])).collect();
        for budget in 0..5 {
            let mut device = CrashDevice::new(vec![0; 512 * 32], Some(budget));
            journal.commit(&mut device, 3, &writes).unwrap();
            assert_eq!(journal.read(&mut device).unwrap(), None, "budget {budget}");
        }
        let mut device = CrashDevice::new(vec![0; 512 * 32], Some(5));
        journal.commit(&mut device, 3, &writes).unwrap();
        assert!(journal.read(&mut device).unwrap().is_some());
    }
}
