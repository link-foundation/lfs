//! Variable-size storage: power-of-two size classes from 16 bytes to 1 MiB.
//!
//! Every class is a [`ChunkedArray`] of fixed-size slots, so variable-size
//! objects never fragment the device: they live in the same small,
//! relocatable chunks as links. A slot starts with a `u32` length; length 0
//! marks a free slot (free slots are found again at mount by that marker).
//!
//! Objects are addressed by a [`Handle`] that packs a kind tag, the class and
//! the slot number into one `u64`, so a handle fits in a link-sized cell.

use crate::chunked::{ArrayDescriptor, BlockWrite, ChunkedArray};
use crate::codec::{get_u32, put_u32};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::space::{Fit, Space};
use std::collections::BTreeSet;

/// Smallest class: `2^MIN_CLASS_LOG` bytes.
pub const MIN_CLASS_LOG: u32 = 4;
/// Largest class: `2^MAX_CLASS_LOG` bytes.
pub const MAX_CLASS_LOG: u32 = 20;
/// Number of size classes.
pub const CLASSES: usize = (MAX_CLASS_LOG - MIN_CLASS_LOG + 1) as usize;
/// Bytes taken by the slot length prefix.
pub const PREFIX: usize = 4;

/// A packed reference to a variable-size object.
///
/// Bits 62..=63 hold the kind (0 = no object), bits 57..=61 the class and
/// bits 0..=56 the slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Handle(pub u64);

impl Handle {
    /// The empty handle.
    pub const NONE: Self = Self(0);

    /// Builds a handle.
    #[must_use]
    pub const fn new(kind: u8, class: usize, slot: u64) -> Self {
        Self(((kind as u64) << 62) | ((class as u64) << 57) | slot)
    }

    /// Kind tag (0 means no object).
    #[must_use]
    pub const fn kind(self) -> u8 {
        (self.0 >> 62) as u8
    }

    /// Size class index.
    #[must_use]
    pub const fn class(self) -> usize {
        ((self.0 >> 57) & 0x1f) as usize
    }

    /// Slot within the class.
    #[must_use]
    pub const fn slot(self) -> u64 {
        self.0 & ((1 << 57) - 1)
    }

    /// Whether the handle refers to an object.
    #[must_use]
    pub const fn is_some(self) -> bool {
        self.kind() != 0
    }
}

/// Byte size of class `class`.
#[must_use]
pub const fn class_size(class: usize) -> usize {
    1 << (MIN_CLASS_LOG as usize + class)
}

/// Smallest class whose slots hold `bytes` bytes (prefix included).
#[must_use]
pub fn class_for(bytes: usize) -> Option<usize> {
    (0..CLASSES).find(|&class| class_size(class) >= bytes)
}

/// The variable-size store.
#[derive(Clone, Debug)]
pub struct VarStore {
    classes: Vec<ChunkedArray>,
    free: Vec<BTreeSet<u64>>,
}

impl VarStore {
    /// An empty store.
    #[must_use]
    pub fn new(block_size: usize) -> Self {
        Self {
            classes: (0..CLASSES)
                .map(|class| ChunkedArray::new(class_size(class), block_size))
                .collect(),
            free: vec![BTreeSet::new(); CLASSES],
        }
    }

    /// Loads all classes and rebuilds the free slot sets.
    pub fn load(
        device: &mut dyn BlockDevice,
        descriptors: &[ArrayDescriptor],
        block_size: usize,
        space: &mut Space,
    ) -> Result<Self> {
        let mut store = Self::new(block_size);
        for (class, descriptor) in descriptors.iter().enumerate().take(CLASSES) {
            let array =
                ChunkedArray::load(device, *descriptor, class_size(class), block_size, space)?;
            for slot in 0..array.capacity() {
                if get_u32(array.record(slot), 0) == 0 {
                    store.free[class].insert(slot);
                }
            }
            store.classes[class] = array;
        }
        Ok(store)
    }

    /// Allocates a zeroed object of `len` payload bytes tagged `kind`.
    pub fn allocate(&mut self, kind: u8, len: usize) -> Result<Handle> {
        debug_assert!(kind != 0 && kind < 4);
        let class = class_for(len + PREFIX).ok_or_else(|| {
            Error::Unsupported(format!("object of {len} bytes exceeds the largest class"))
        })?;
        let slot = if let Some(slot) = self.free[class].pop_first() {
            slot
        } else {
            let array = &mut self.classes[class];
            let first = array.capacity();
            array.ensure_capacity(first + 1);
            for extra in first + 1..array.capacity() {
                self.free[class].insert(extra);
            }
            first
        };
        let record = self.classes[class].record_mut(slot);
        record.fill(0);
        put_u32(record, 0, (len as u32).max(1));
        Ok(Handle::new(kind, class, slot))
    }

    /// Releases an object.
    pub fn free(&mut self, handle: Handle) {
        if !handle.is_some() {
            return;
        }
        let record = self.classes[handle.class()].record_mut(handle.slot());
        record.fill(0);
        self.free[handle.class()].insert(handle.slot());
    }

    /// Payload capacity of the object's slot.
    #[must_use]
    pub const fn capacity(handle: Handle) -> usize {
        class_size(handle.class()) - PREFIX
    }

    /// Stored payload length.
    #[must_use]
    pub fn len(&self, handle: Handle) -> usize {
        get_u32(self.classes[handle.class()].record(handle.slot()), 0) as usize
    }

    /// Payload bytes (whole slot capacity).
    #[must_use]
    pub fn bytes(&self, handle: Handle) -> &[u8] {
        &self.classes[handle.class()].record(handle.slot())[PREFIX..]
    }

    /// Mutable payload bytes (whole slot capacity).
    pub fn bytes_mut(&mut self, handle: Handle) -> &mut [u8] {
        &mut self.classes[handle.class()].record_mut(handle.slot())[PREFIX..]
    }

    /// Sets the stored payload length.
    pub fn set_len(&mut self, handle: Handle, len: usize) {
        put_u32(
            self.classes[handle.class()].record_mut(handle.slot()),
            0,
            (len as u32).max(1),
        );
    }

    /// Stores `data` as a new object.
    pub fn store(&mut self, kind: u8, data: &[u8]) -> Result<Handle> {
        let handle = self.allocate(kind, data.len())?;
        self.bytes_mut(handle)[..data.len()].copy_from_slice(data);
        self.set_len(handle, data.len());
        Ok(handle)
    }

    /// Reads an object stored with [`store`](Self::store).
    #[must_use]
    pub fn load_bytes(&self, handle: Handle) -> Vec<u8> {
        self.bytes(handle)[..self.len(handle)].to_vec()
    }

    /// Moves an object into a slot of at least `len` payload bytes, keeping
    /// the first `keep` bytes.
    pub fn resize(&mut self, handle: Handle, len: usize, keep: usize) -> Result<Handle> {
        let new = self.allocate(handle.kind(), len)?;
        let keep = keep.min(Self::capacity(handle)).min(Self::capacity(new));
        let old = self.bytes(handle)[..keep].to_vec();
        self.bytes_mut(new)[..keep].copy_from_slice(&old);
        self.free(handle);
        Ok(new)
    }

    /// Releases every object.
    pub fn clear(&mut self, space: &mut Space) {
        for class in 0..CLASSES {
            self.classes[class].truncate_chunks(0, space);
            self.free[class].clear();
        }
    }

    /// Drops trailing chunks that hold only free slots.
    pub fn shrink(&mut self, space: &mut Space) {
        for class in 0..CLASSES {
            let array = &mut self.classes[class];
            let mut keep = array.capacity();
            while keep > 0 && self.free[class].contains(&(keep - 1)) {
                keep -= 1;
            }
            let per_chunk = array.per_chunk() as u64;
            let keep = keep.div_ceil(per_chunk) * per_chunk;
            if keep < array.capacity() {
                array.truncate_chunks(keep, space);
                self.free[class].retain(|&slot| slot < keep);
            }
        }
    }

    /// The size-class arrays.
    #[must_use]
    pub fn arrays(&self) -> &[ChunkedArray] {
        &self.classes
    }

    /// Mutable size-class arrays.
    pub fn arrays_mut(&mut self) -> &mut [ChunkedArray] {
        &mut self.classes
    }

    /// Places new chunks.
    pub fn place(&mut self, space: &mut Space, fit: Fit) -> Result<()> {
        for array in &mut self.classes {
            array.place(space, fit)?;
        }
        Ok(())
    }

    /// Collects dirty blocks.
    pub fn take_writes(&mut self, out: &mut Vec<BlockWrite>) {
        for array in &mut self.classes {
            array.take_writes(out);
        }
    }

    /// Number of live objects and bytes they occupy (slot sizes).
    #[must_use]
    pub fn usage(&self) -> (u64, u64) {
        let mut objects = 0;
        let mut bytes = 0;
        for (class, array) in self.classes.iter().enumerate() {
            let live = array.capacity() - self.free[class].len() as u64;
            objects += live;
            bytes += live * class_size(class) as u64;
        }
        (objects, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_pack_kind_class_and_slot() {
        let handle = Handle::new(2, 16, 12345);
        assert_eq!(handle.kind(), 2);
        assert_eq!(handle.class(), 16);
        assert_eq!(handle.slot(), 12345);
        assert!(!Handle::NONE.is_some());
        assert_eq!(class_for(17), Some(1));
        assert_eq!(class_for((1 << 20) + 1), None);
    }

    #[test]
    fn objects_store_resize_and_free() {
        let mut store = VarStore::new(1024);
        let a = store.store(1, b"hello").unwrap();
        assert_eq!(a.class(), 0);
        assert_eq!(store.load_bytes(a), b"hello");
        let b = store.resize(a, 100, 5).unwrap();
        assert_eq!(b.class(), 3);
        assert_eq!(&store.bytes(b)[..5], b"hello");
        let c = store.store(1, b"x").unwrap();
        assert_eq!(c.slot(), a.slot(), "freed slot is reused");
        assert_eq!(store.usage().0, 2);
        store.free(b);
        store.free(c);
        let mut space = Space::new(100);
        store.shrink(&mut space);
        assert_eq!(store.usage(), (0, 0));
        assert!(store.arrays().iter().all(|array| array.chunk_count() == 0));
    }

    #[test]
    fn big_objects_take_multi_block_slots() {
        let mut store = VarStore::new(1024);
        let handle = store.allocate(2, 60_000).unwrap();
        assert_eq!(class_size(handle.class()), 65_536);
        assert_eq!(store.arrays()[handle.class()].chunk_blocks(), 64);
    }
}
