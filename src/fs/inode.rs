//! Inode records: the fixed-size part of a file (attributes and its one
//! contiguous extent).
//!
//! Records live in a [`ChunkedArray`] of 64-byte slots, so the inode table
//! fills the gaps between files like link chunks do. The link side of an
//! inode is the link `(TYPE, $slot)`; the record side is slot `slot`. A slot
//! whose mode is 0 is free; free slots are found again at mount.

use crate::chunked::ChunkedArray;
use crate::codec::{get_i64, get_u32, get_u64, put_i64, put_u32, put_u64};
use std::collections::BTreeSet;

/// Bytes per inode record.
pub const RECORD_SIZE: usize = 64;

/// File type bits of `mode`.
pub const S_IFMT: u32 = 0o170_000;
/// Regular file.
pub const S_IFREG: u32 = 0o100_000;
/// Directory.
pub const S_IFDIR: u32 = 0o040_000;
/// Symbolic link.
pub const S_IFLNK: u32 = 0o120_000;

/// Kind of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link.
    Symlink,
}

impl FileKind {
    /// Kind encoded in `mode`.
    #[must_use]
    pub const fn from_mode(mode: u32) -> Option<Self> {
        match mode & S_IFMT {
            S_IFREG => Some(Self::File),
            S_IFDIR => Some(Self::Dir),
            S_IFLNK => Some(Self::Symlink),
            _ => None,
        }
    }

    /// The `S_IF*` bits of this kind.
    #[must_use]
    pub const fn mode_bits(self) -> u32 {
        match self {
            Self::File => S_IFREG,
            Self::Dir => S_IFDIR,
            Self::Symlink => S_IFLNK,
        }
    }
}

/// Decoded inode record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inode {
    /// Type and permission bits (0: free slot).
    pub mode: u32,
    /// Number of hard links (directories: 2 + subdirectories).
    pub nlink: u32,
    /// Owner.
    pub uid: u32,
    /// Group.
    pub gid: u32,
    /// Size in bytes.
    pub size: u64,
    /// First block of the extent.
    pub start: u64,
    /// Blocks reserved for the extent (may exceed what `size` needs).
    pub blocks: u64,
    /// Modification time, ns since the Unix epoch.
    pub mtime: i64,
    /// Change time.
    pub ctime: i64,
    /// Access time.
    pub atime: i64,
}

impl Inode {
    /// Decodes a record.
    #[must_use]
    pub fn read(bytes: &[u8]) -> Self {
        Self {
            mode: get_u32(bytes, 0),
            nlink: get_u32(bytes, 4),
            uid: get_u32(bytes, 8),
            gid: get_u32(bytes, 12),
            size: get_u64(bytes, 16),
            start: get_u64(bytes, 24),
            blocks: get_u64(bytes, 32),
            mtime: get_i64(bytes, 40),
            ctime: get_i64(bytes, 48),
            atime: get_i64(bytes, 56),
        }
    }

    /// Encodes a record.
    pub fn write(&self, bytes: &mut [u8]) {
        put_u32(bytes, 0, self.mode);
        put_u32(bytes, 4, self.nlink);
        put_u32(bytes, 8, self.uid);
        put_u32(bytes, 12, self.gid);
        put_u64(bytes, 16, self.size);
        put_u64(bytes, 24, self.start);
        put_u64(bytes, 32, self.blocks);
        put_i64(bytes, 40, self.mtime);
        put_i64(bytes, 48, self.ctime);
        put_i64(bytes, 56, self.atime);
    }

    /// The kind of the file.
    #[must_use]
    pub const fn kind(&self) -> Option<FileKind> {
        FileKind::from_mode(self.mode)
    }

    /// Whether the slot holds a file.
    #[must_use]
    pub const fn is_used(&self) -> bool {
        self.mode != 0
    }
}

/// The inode table.
#[derive(Clone, Debug)]
pub struct Inodes {
    /// Records.
    pub array: ChunkedArray,
    free: BTreeSet<u64>,
}

impl Inodes {
    /// An empty table.
    #[must_use]
    pub fn new(block_size: usize) -> Self {
        Self::from_array(ChunkedArray::new(RECORD_SIZE, block_size))
    }

    /// Wraps a loaded array and rebuilds the free slot set.
    #[must_use]
    pub fn from_array(array: ChunkedArray) -> Self {
        let free = (0..array.capacity())
            .filter(|&slot| get_u32(array.record(slot), 0) == 0)
            .collect();
        Self { array, free }
    }

    /// Reads slot `slot` (a free record if out of range).
    #[must_use]
    pub fn get(&self, slot: u64) -> Inode {
        if slot < self.array.capacity() {
            Inode::read(self.array.record(slot))
        } else {
            Inode::default()
        }
    }

    /// Writes slot `slot` if it changed.
    pub fn set(&mut self, slot: u64, inode: &Inode) {
        if self.get(slot) == *inode {
            return;
        }
        inode.write(self.array.record_mut(slot));
        if inode.is_used() {
            self.free.remove(&slot);
        } else {
            self.free.insert(slot);
        }
    }

    /// Takes the lowest free slot, growing the table when needed.
    pub fn allocate(&mut self) -> u64 {
        if let Some(slot) = self.free.pop_first() {
            return slot;
        }
        let first = self.array.capacity();
        self.array.ensure_capacity(first + 1);
        self.free.extend(first + 1..self.array.capacity());
        first
    }

    /// Releases a slot.
    pub fn free(&mut self, slot: u64) {
        self.set(slot, &Inode::default());
    }

    /// Number of used slots.
    #[must_use]
    pub fn used(&self) -> u64 {
        self.array.capacity() - self.free.len() as u64
    }

    /// Number of slots.
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        self.array.capacity()
    }

    /// Every used slot with its record.
    pub fn each(&self) -> impl Iterator<Item = (u64, Inode)> + '_ {
        (0..self.array.capacity())
            .map(|slot| (slot, self.get(slot)))
            .filter(|(_, inode)| inode.is_used())
    }

    /// Drops trailing chunks holding only free slots.
    pub fn shrink(&mut self, space: &mut crate::space::Space) {
        let mut keep = self.array.capacity();
        while keep > 1 && self.free.contains(&(keep - 1)) {
            keep -= 1;
        }
        let per_chunk = self.array.per_chunk() as u64;
        let keep = keep.div_ceil(per_chunk) * per_chunk;
        if keep < self.array.capacity() {
            self.array.truncate_chunks(keep, space);
            self.free.retain(|&slot| slot < keep);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_slots_are_reused() {
        let mut inodes = Inodes::new(1024);
        let a = inodes.allocate();
        let b = inodes.allocate();
        assert_eq!((a, b), (0, 1));
        let inode = Inode {
            mode: S_IFREG | 0o644,
            nlink: 1,
            size: 5,
            start: 9,
            blocks: 1,
            mtime: -3,
            ..Inode::default()
        };
        inodes.set(b, &inode);
        assert_eq!(inodes.get(b), inode);
        assert_eq!(inodes.get(b).kind(), Some(FileKind::File));
        inodes.free(b);
        assert_eq!(inodes.allocate(), 1, "freed slots are reused first");
        inodes.free(a);
        let reloaded = Inodes::from_array(inodes.array.clone());
        assert_eq!(reloaded.used(), 0);
    }
}
