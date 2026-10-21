//! The optional variable-size usage index.
//!
//! For every link `X` and each role it keeps the set of links that use `X`
//! as their source (or target). A set is stored in the variable-size store
//! either as a size-balanced tree (24 bytes per user) or as a bit string
//! (one bit per link in the store, bit `L` set iff link `L` uses `X`).
//!
//! In adaptive mode the encoding is chosen per set by usage density
//! `users / links`: a bit string costs `links / 8` bytes and a tree
//! `24 * users`, so the break-even density is 1/192. Above the threshold a
//! set becomes a bit string; it goes back to a tree only when the density
//! drops below `threshold * hysteresis` so sets near the boundary do not
//! flip back and forth.

use super::is_internal;
use super::records::{Records, Role};
use crate::chunked::ChunkedArray;
use crate::codec::{get_u32, get_u64, put_u32, put_u64};
use crate::config::IndexMode;
use crate::error::{Error, Result};
use crate::sbt::{self, Key, Nodes, NodesRead};
use crate::varstore::{class_for, class_size, Handle, VarStore, PREFIX};

/// Handle kind of a tree.
pub const KIND_TREE: u8 = 1;
/// Handle kind of a bit string.
pub const KIND_BITS: u8 = 2;
/// Bytes per tree node.
pub const NODE_BYTES: usize = 24;
const HEADER: usize = 8;

/// Statistics of the usage index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageStats {
    /// Sets stored as trees.
    pub trees: u64,
    /// Sets stored as bit strings.
    pub bitstrings: u64,
    /// Bytes of all tree slots.
    pub tree_bytes: u64,
    /// Bytes of all bit string slots.
    pub bitstring_bytes: u64,
}

/// The usage index.
#[derive(Clone, Debug)]
pub struct UsageIndex {
    /// Two handles (source role, target role) per link.
    pub handles: ChunkedArray,
    /// Storage of the sets.
    pub var: VarStore,
    /// Active mode.
    pub mode: IndexMode,
    /// Adaptive threshold (fraction of all links).
    pub threshold: f64,
    /// Hysteresis factor (fraction of the threshold).
    pub hysteresis: f64,
}

struct BlobTree<'a> {
    bytes: &'a mut [u8],
    records: &'a Records,
    role: Role,
}

struct BlobTreeRef<'a> {
    bytes: &'a [u8],
    records: &'a Records,
    role: Role,
}

const fn node_offset(node: u64) -> usize {
    HEADER + (node as usize - 1) * NODE_BYTES
}

fn node_link(bytes: &[u8], node: u64) -> u64 {
    get_u64(bytes, node_offset(node))
}

fn node_cell(bytes: &[u8], node: u64, which: usize) -> u64 {
    u64::from(get_u32(bytes, node_offset(node) + 8 + which * 4))
}

fn blob_key(records: &Records, role: Role, bytes: &[u8], node: u64) -> Key {
    let link = node_link(bytes, node);
    (records.other_end(link, role), link, 0)
}

impl NodesRead for BlobTreeRef<'_> {
    fn left(&self, node: u64) -> u64 {
        node_cell(self.bytes, node, 0)
    }
    fn right(&self, node: u64) -> u64 {
        node_cell(self.bytes, node, 1)
    }
    fn size(&self, node: u64) -> u64 {
        node_cell(self.bytes, node, 2)
    }
    fn key(&self, node: u64) -> Key {
        blob_key(self.records, self.role, self.bytes, node)
    }
}

impl NodesRead for BlobTree<'_> {
    fn left(&self, node: u64) -> u64 {
        node_cell(self.bytes, node, 0)
    }
    fn right(&self, node: u64) -> u64 {
        node_cell(self.bytes, node, 1)
    }
    fn size(&self, node: u64) -> u64 {
        node_cell(self.bytes, node, 2)
    }
    fn key(&self, node: u64) -> Key {
        blob_key(self.records, self.role, self.bytes, node)
    }
}

impl Nodes for BlobTree<'_> {
    fn set_left(&mut self, node: u64, value: u64) {
        put_u32(self.bytes, node_offset(node) + 8, value as u32);
    }
    fn set_right(&mut self, node: u64, value: u64) {
        put_u32(self.bytes, node_offset(node) + 12, value as u32);
    }
    fn set_size(&mut self, node: u64, value: u64) {
        put_u32(self.bytes, node_offset(node) + 16, value as u32);
    }
}

const fn role_offset(role: Role) -> usize {
    match role {
        Role::Source => 0,
        Role::Target => 8,
    }
}

const fn tree_capacity(handle: Handle) -> u64 {
    ((VarStore::capacity(handle) - HEADER) / NODE_BYTES) as u64
}

const fn bits_capacity(handle: Handle) -> u64 {
    ((VarStore::capacity(handle) - HEADER) * 8) as u64
}

const fn tree_bytes_for(count: u64) -> usize {
    HEADER + count as usize * NODE_BYTES
}

const fn bits_bytes_for(last_link: u64) -> usize {
    HEADER + (last_link as usize / 8) + 1
}

impl UsageIndex {
    /// An empty index.
    #[must_use]
    pub fn new(block_size: usize) -> Self {
        Self {
            handles: ChunkedArray::new(16, block_size),
            var: VarStore::new(block_size),
            mode: IndexMode::Off,
            threshold: 1.0 / 192.0,
            hysteresis: 0.5,
        }
    }

    /// Whether sets are maintained.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.mode != IndexMode::Off
    }

    /// The handle of `link`'s users in `role`.
    #[must_use]
    pub fn handle(&self, link: u64, role: Role) -> Handle {
        if link >= self.handles.capacity() {
            return Handle::NONE;
        }
        Handle(self.handles.get(link, role_offset(role)))
    }

    fn set_handle(&mut self, link: u64, role: Role, handle: Handle) {
        self.handles.ensure_capacity(link + 1);
        self.handles.set(link, role_offset(role), handle.0);
    }

    /// Number of users of `link` in `role`.
    #[must_use]
    pub fn count(&self, link: u64, role: Role) -> u64 {
        let handle = self.handle(link, role);
        if handle.is_some() {
            u64::from(get_u32(self.var.bytes(handle), 0))
        } else {
            0
        }
    }

    fn wants_bits(&self, users: u64, total: u64) -> bool {
        match self.mode {
            IndexMode::Off | IndexMode::Tree => false,
            IndexMode::Bitstring => true,
            IndexMode::Adaptive => users as f64 >= self.threshold * total.max(1) as f64,
        }
    }

    fn wants_tree(&self, users: u64, total: u64) -> bool {
        match self.mode {
            IndexMode::Off | IndexMode::Tree => true,
            IndexMode::Bitstring => false,
            IndexMode::Adaptive => {
                (users as f64) < self.threshold * self.hysteresis * total.max(1) as f64
            }
        }
    }

    /// Records that `link` (with its current ends) uses its `role` end.
    pub fn add(&mut self, records: &Records, link: u64, role: Role, total: u64) -> Result<()> {
        let end = records.end(link, role);
        if !self.enabled() || !is_internal(end) {
            return Ok(());
        }
        let mut handle = self.handle(end, role);
        if !handle.is_some() {
            handle = if self.wants_bits(1, total)
                && class_for(PREFIX + bits_bytes_for(total.max(link))).is_some()
            {
                self.var
                    .allocate(KIND_BITS, bits_bytes_for(total.max(link)))?
            } else {
                self.var.allocate(KIND_TREE, tree_bytes_for(1))?
            };
        }
        handle = match handle.kind() {
            KIND_TREE => self.tree_insert(records, handle, link, role)?,
            _ => self.bits_set(handle, link, true)?,
        };
        self.set_handle(end, role, handle);
        self.adapt(records, end, role, total)
    }

    /// Records that `link` (with its current ends) no longer uses its `role` end.
    pub fn remove(&mut self, records: &Records, link: u64, role: Role, total: u64) -> Result<()> {
        let end = records.end(link, role);
        if !self.enabled() || !is_internal(end) {
            return Ok(());
        }
        let handle = self.handle(end, role);
        if !handle.is_some() {
            return Err(Error::corrupt(format!(
                "usage index of {end} misses link {link}"
            )));
        }
        let handle = match handle.kind() {
            KIND_TREE => self.tree_remove(records, handle, link, role)?,
            _ => self.bits_set(handle, link, false)?,
        };
        if handle.is_some() && self.count_handle(handle) == 0 {
            self.var.free(handle);
            self.set_handle(end, role, Handle::NONE);
            return Ok(());
        }
        self.set_handle(end, role, handle);
        self.adapt(records, end, role, total)
    }

    fn count_handle(&self, handle: Handle) -> u64 {
        u64::from(get_u32(self.var.bytes(handle), 0))
    }

    fn adapt(&mut self, records: &Records, end: u64, role: Role, total: u64) -> Result<()> {
        let handle = self.handle(end, role);
        if !handle.is_some() {
            return Ok(());
        }
        let users = self.count_handle(handle);
        let convert = match handle.kind() {
            KIND_TREE => {
                self.wants_bits(users, total) && class_for(PREFIX + bits_bytes_for(total)).is_some()
            }
            _ => {
                self.wants_tree(users, total) && class_for(PREFIX + tree_bytes_for(users)).is_some()
            }
        };
        if !convert {
            return Ok(());
        }
        let mut users_list = Vec::with_capacity(users as usize);
        self.each(records, end, role, &mut |link| {
            users_list.push(link);
            true
        });
        let kind = if handle.kind() == KIND_TREE {
            KIND_BITS
        } else {
            KIND_TREE
        };
        self.var.free(handle);
        let last = users_list.iter().copied().max().unwrap_or(0).max(total);
        let mut new = if kind == KIND_BITS {
            self.var.allocate(KIND_BITS, bits_bytes_for(last))?
        } else {
            self.var.allocate(KIND_TREE, tree_bytes_for(users))?
        };
        for link in users_list {
            new = if kind == KIND_BITS {
                self.bits_set(new, link, true)?
            } else {
                self.tree_insert(records, new, link, role)?
            };
        }
        self.set_handle(end, role, new);
        Ok(())
    }

    fn tree_insert(
        &mut self,
        records: &Records,
        handle: Handle,
        link: u64,
        role: Role,
    ) -> Result<Handle> {
        let count = self.count_handle(handle);
        let mut handle = handle;
        if count + 1 > tree_capacity(handle) {
            if class_for(PREFIX + tree_bytes_for(count + 1)).is_none() {
                // The tree cannot grow any more: switch this set to a bit string.
                return self.tree_to_bits_forced(records, handle, link, role);
            }
            handle = self.var.resize(
                handle,
                tree_bytes_for((count + 1).next_power_of_two()),
                tree_bytes_for(count),
            )?;
        }
        let bytes = self.var.bytes_mut(handle);
        let node = count + 1;
        put_u64(bytes, node_offset(node), link);
        let root = u64::from(get_u32(bytes, 4));
        let mut tree = BlobTree {
            bytes,
            records,
            role,
        };
        let root = sbt::insert(&mut tree, root, node);
        put_u32(tree.bytes, 4, root as u32);
        put_u32(tree.bytes, 0, node as u32);
        Ok(handle)
    }

    fn tree_to_bits_forced(
        &mut self,
        records: &Records,
        handle: Handle,
        link: u64,
        role: Role,
    ) -> Result<Handle> {
        let bytes = self.var.bytes(handle).to_vec();
        let count = u64::from(get_u32(&bytes, 0));
        let mut links: Vec<u64> = (1..=count).map(|node| node_link(&bytes, node)).collect();
        links.push(link);
        let last = links.iter().copied().max().unwrap_or(0);
        if class_for(PREFIX + bits_bytes_for(last)).is_none() {
            return Err(Error::Unsupported(format!(
                "link used by more than {count} links in role {role:?} exceeds the optional index limits"
            )));
        }
        let _ = records;
        self.var.free(handle);
        let mut new = self.var.allocate(KIND_BITS, bits_bytes_for(last))?;
        for link in links {
            new = self.bits_set(new, link, true)?;
        }
        Ok(new)
    }

    fn tree_remove(
        &mut self,
        records: &Records,
        handle: Handle,
        link: u64,
        role: Role,
    ) -> Result<Handle> {
        let bytes = self.var.bytes_mut(handle);
        let count = u64::from(get_u32(bytes, 0));
        let root = u64::from(get_u32(bytes, 4));
        let key = (records.other_end(link, role), link, 0);
        let mut tree = BlobTree {
            bytes,
            records,
            role,
        };
        let mut found = 0;
        sbt::each_in_range(&tree, root, key, key, &mut |node| {
            found = node;
            false
        });
        if found == 0 {
            return Err(Error::corrupt(format!("usage tree misses link {link}")));
        }
        let mut root = sbt::remove(&mut tree, root, key);
        if found != count {
            // Move the last node into the hole and repoint its parent.
            let last_key = tree.key(count);
            let start = node_offset(count);
            let moved: Vec<u8> = tree.bytes[start..start + NODE_BYTES].to_vec();
            let hole = node_offset(found);
            tree.bytes[hole..hole + NODE_BYTES].copy_from_slice(&moved);
            if root == count {
                root = found;
            } else {
                let mut parent = root;
                loop {
                    let next = if last_key < tree.key(parent) {
                        tree.left(parent)
                    } else {
                        tree.right(parent)
                    };
                    if next == count {
                        if last_key < tree.key(parent) {
                            tree.set_left(parent, found);
                        } else {
                            tree.set_right(parent, found);
                        }
                        break;
                    }
                    parent = next;
                }
            }
        }
        put_u32(tree.bytes, 4, root as u32);
        put_u32(tree.bytes, 0, (count - 1) as u32);
        let needed = PREFIX + tree_bytes_for(count - 1);
        if count > 1 && needed * 4 <= class_size(handle.class()) {
            return self
                .var
                .resize(handle, tree_bytes_for(count - 1), tree_bytes_for(count - 1));
        }
        Ok(handle)
    }

    fn bits_set(&mut self, handle: Handle, link: u64, value: bool) -> Result<Handle> {
        let mut handle = handle;
        if link >= bits_capacity(handle) {
            if !value {
                return Err(Error::corrupt(format!("bit string misses link {link}")));
            }
            let len = bits_bytes_for(link * 2);
            let len = if class_for(PREFIX + len).is_some() {
                len
            } else {
                bits_bytes_for(link)
            };
            let keep = VarStore::capacity(handle);
            handle = self.var.resize(handle, len, keep)?;
        }
        let bytes = self.var.bytes_mut(handle);
        let byte = HEADER + (link / 8) as usize;
        let mask = 1u8 << (link % 8);
        let was = bytes[byte] & mask != 0;
        if was == value {
            return Err(Error::corrupt(format!(
                "bit of link {link} is already {value}"
            )));
        }
        bytes[byte] ^= mask;
        let count = get_u32(bytes, 0);
        put_u32(bytes, 0, if value { count + 1 } else { count - 1 });
        Ok(handle)
    }

    /// Visits the users of `end` in `role`; returns `false` if stopped.
    pub fn each(
        &self,
        records: &Records,
        end: u64,
        role: Role,
        visit: &mut dyn FnMut(u64) -> bool,
    ) -> bool {
        let handle = self.handle(end, role);
        if !handle.is_some() {
            return true;
        }
        let bytes = self.var.bytes(handle);
        if handle.kind() == KIND_TREE {
            let root = u64::from(get_u32(bytes, 4));
            let tree = BlobTreeRef {
                bytes,
                records,
                role,
            };
            let mut visit_link = |node| visit(node_link(bytes, node));
            return sbt::each_in_range(
                &tree,
                root,
                (0, 0, 0),
                (u64::MAX, u64::MAX, u64::MAX),
                &mut visit_link,
            );
        }
        for (index, &byte) in bytes[HEADER..].iter().enumerate() {
            let mut byte = byte;
            while byte != 0 {
                let bit = byte.trailing_zeros();
                byte &= byte - 1;
                if !visit(index as u64 * 8 + u64::from(bit)) {
                    return false;
                }
            }
        }
        true
    }

    /// Finds the user of `end` in `role` whose other end is `other`.
    #[must_use]
    pub fn find(&self, records: &Records, end: u64, role: Role, other: u64) -> Option<u64> {
        let handle = self.handle(end, role);
        if !handle.is_some() {
            return None;
        }
        let mut found = None;
        if handle.kind() == KIND_TREE {
            let bytes = self.var.bytes(handle);
            let root = u64::from(get_u32(bytes, 4));
            let tree = BlobTreeRef {
                bytes,
                records,
                role,
            };
            sbt::each_in_range(
                &tree,
                root,
                (other, 0, 0),
                (other, u64::MAX, u64::MAX),
                &mut |node| {
                    found = Some(node_link(bytes, node));
                    false
                },
            );
        } else {
            self.each(records, end, role, &mut |link| {
                if records.other_end(link, role) == other {
                    found = Some(link);
                    return false;
                }
                true
            });
        }
        found
    }

    /// Statistics.
    #[must_use]
    pub fn stats(&self) -> UsageStats {
        let mut stats = UsageStats::default();
        for link in 0..self.handles.capacity() {
            for role in [Role::Source, Role::Target] {
                let handle = Handle(self.handles.get(link, role_offset(role)));
                let bytes = if handle.is_some() {
                    class_size(handle.class()) as u64
                } else {
                    0
                };
                match handle.kind() {
                    KIND_TREE => {
                        stats.trees += 1;
                        stats.tree_bytes += bytes;
                    }
                    KIND_BITS => {
                        stats.bitstrings += 1;
                        stats.bitstring_bytes += bytes;
                    }
                    _ => {}
                }
            }
        }
        stats
    }

    /// Checks every set against the records (used by `fsck`).
    pub fn check(
        &self,
        records: &Records,
        live: &dyn Fn(u64) -> bool,
    ) -> std::result::Result<(), String> {
        for link in 1..self.handles.capacity() {
            for role in [Role::Source, Role::Target] {
                let handle = self.handle(link, role);
                if !handle.is_some() {
                    continue;
                }
                let mut seen = 0u64;
                let mut error = None;
                self.each(records, link, role, &mut |user| {
                    seen += 1;
                    if !live(user) || records.end(user, role) != link {
                        error = Some(format!(
                            "usage set of {link} ({role:?}) lists link {user} wrongly"
                        ));
                        return false;
                    }
                    true
                });
                if let Some(error) = error {
                    return Err(error);
                }
                if seen != self.count(link, role) {
                    return Err(format!("usage set of {link} ({role:?}) has a wrong count"));
                }
                if handle.kind() == KIND_TREE {
                    let bytes = self.var.bytes(handle);
                    let tree = BlobTreeRef {
                        bytes,
                        records,
                        role,
                    };
                    sbt::check(&tree, u64::from(get_u32(bytes, 4)), None, None)?;
                }
            }
        }
        Ok(())
    }
}
