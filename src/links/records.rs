//! Raw link records and their embedded size-balanced trees.
//!
//! * **United** — one 64-byte record per link, exactly the doublets
//!   `UnitedMemoryLinks<u64>` record: `source, target, left_as_source,
//!   right_as_source, size_as_source, left_as_target, right_as_target,
//!   size_as_target`, with the links header in record 0. Two global trees
//!   order all links by `(source, target)` and by `(target, source)`.
//! * **Split** — 16-byte data records (`source, target`) plus 64-byte index
//!   records shaped like the doublets `IndexPart`: `root_as_source,
//!   left_as_source, right_as_source, size_as_source, root_as_target, ...`.
//!   Every link roots its own trees of users (per-link trees keyed by the
//!   other end); references to external values go to two global trees
//!   rooted in the header, following the C# `SplitMemoryLinks` rule.
//! * **Data-only** — 16-byte data records, no embedded index.

use super::is_external;
use crate::chunked::ChunkedArray;
use crate::config::Layout;
use crate::sbt::{Key, Nodes, NodesRead};

/// Which end of a link a tree orders by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Links grouped by their source.
    Source,
    /// Links grouped by their target.
    Target,
}

/// Header cells (record 0 of the united layout).
pub mod header {
    /// Highest allocated link index.
    pub const ALLOCATED: usize = 0;
    /// Reserved link capacity.
    pub const RESERVED: usize = 8;
    /// Number of free slots below `allocated`.
    pub const FREE: usize = 16;
    /// First slot of the free list.
    pub const FIRST_FREE: usize = 24;
    /// Root of the global sources tree.
    pub const ROOT_AS_SOURCE: usize = 32;
    /// Root of the global targets tree.
    pub const ROOT_AS_TARGET: usize = 40;
    /// Last slot of the free list.
    pub const LAST_FREE: usize = 48;
}

/// Link records of every layout.
#[derive(Clone, Debug)]
pub struct Records {
    /// The layout.
    pub layout: Layout,
    /// Data records (united: whole records).
    pub data: ChunkedArray,
    /// Split index records (unused by the other layouts).
    pub index: ChunkedArray,
}

const SOURCE: usize = 0;
const TARGET: usize = 8;

impl Records {
    /// Empty records of `layout`.
    #[must_use]
    pub fn new(layout: Layout, block_size: usize) -> Self {
        let data_size = if layout == Layout::United { 64 } else { 16 };
        Self {
            layout,
            data: ChunkedArray::new(data_size, block_size),
            index: ChunkedArray::new(64, block_size),
        }
    }

    /// Makes room for link indices up to `last`.
    pub fn ensure(&mut self, last: u64) {
        self.data.ensure_capacity(last + 1);
        if self.layout == Layout::Split {
            self.index.ensure_capacity(last + 1);
        }
    }

    /// Number of link slots (index 0 included) the records can hold.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        if self.layout == Layout::Split {
            self.data.capacity().min(self.index.capacity())
        } else {
            self.data.capacity()
        }
    }

    /// Source of link `link`.
    #[must_use]
    pub fn source(&self, link: u64) -> u64 {
        self.data.get(link, SOURCE)
    }

    /// Target of link `link`.
    #[must_use]
    pub fn target(&self, link: u64) -> u64 {
        self.data.get(link, TARGET)
    }

    /// Writes both ends of a link.
    pub fn set_link(&mut self, link: u64, source: u64, target: u64) {
        self.data.set(link, SOURCE, source);
        self.data.set(link, TARGET, target);
    }

    /// The end of `link` a role groups by.
    #[must_use]
    pub fn end(&self, link: u64, role: Role) -> u64 {
        match role {
            Role::Source => self.source(link),
            Role::Target => self.target(link),
        }
    }

    /// The other end.
    #[must_use]
    pub fn other_end(&self, link: u64, role: Role) -> u64 {
        match role {
            Role::Source => self.target(link),
            Role::Target => self.source(link),
        }
    }

    /// Reads a header cell (united record 0 or split index record 0).
    #[must_use]
    pub fn header(&self, cell: usize) -> u64 {
        match self.layout {
            Layout::United if self.data.capacity() > 0 => self.data.get(0, cell),
            Layout::Split if self.index.capacity() > 0 => self.index.get(0, cell),
            _ => 0,
        }
    }

    /// Writes a header cell (ignored by the data-only layout).
    pub fn set_header(&mut self, cell: usize, value: u64) {
        match self.layout {
            Layout::United => {
                self.data.ensure_capacity(1);
                if self.data.get(0, cell) != value {
                    self.data.set(0, cell, value);
                }
            }
            Layout::Split => {
                self.index.ensure_capacity(1);
                if self.index.get(0, cell) != value {
                    self.index.set(0, cell, value);
                }
            }
            Layout::DataOnly => {}
        }
    }

    const fn tree_offset(&self, role: Role) -> usize {
        match (self.layout, role) {
            (Layout::United, Role::Source) => 16,
            (_, Role::Target) => 40,
            (_, Role::Source) => 8,
        }
    }

    fn tree_array(&self) -> &ChunkedArray {
        if self.layout == Layout::United {
            &self.data
        } else {
            &self.index
        }
    }

    fn tree_array_mut(&mut self) -> &mut ChunkedArray {
        if self.layout == Layout::United {
            &mut self.data
        } else {
            &mut self.index
        }
    }

    /// Reads tree cell `which` (0 left, 1 right, 2 size) of `link`.
    #[must_use]
    pub fn tree_cell(&self, link: u64, role: Role, which: usize) -> u64 {
        self.tree_array()
            .get(link, self.tree_offset(role) + which * 8)
    }

    /// Writes tree cell `which` of `link`.
    pub fn set_tree_cell(&mut self, link: u64, role: Role, which: usize, value: u64) {
        let offset = self.tree_offset(role) + which * 8;
        self.tree_array_mut().set(link, offset, value);
    }

    /// Root of the per-link tree of users of `link` (split layout).
    #[must_use]
    pub fn link_root(&self, link: u64, role: Role) -> u64 {
        let offset = if role == Role::Source { 0 } else { 32 };
        self.index.get(link, offset)
    }

    /// Sets the root of a per-link tree.
    pub fn set_link_root(&mut self, link: u64, role: Role, value: u64) {
        let offset = if role == Role::Source { 0 } else { 32 };
        self.index.set(link, offset, value);
    }

    /// Which tree a link belongs to for `role`, if any.
    #[must_use]
    pub fn tree_of(&self, link: u64, role: Role) -> Option<TreeId> {
        let end = self.end(link, role);
        if end == 0 || !self.layout.has_embedded_index() {
            return None;
        }
        Some(if self.layout == Layout::Split && !is_external(end) {
            TreeId::PerLink(end)
        } else {
            TreeId::Global
        })
    }
}

/// Identifies one embedded tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeId {
    /// A header-rooted tree over all (or all external) ends.
    Global,
    /// The per-link tree of the users of the given link.
    PerLink(u64),
}

/// Read-only view of one embedded tree.
pub struct Tree<'a> {
    /// Records.
    pub records: &'a Records,
    /// Role.
    pub role: Role,
    /// Whether keys omit the shared end (per-link trees).
    pub per_link: bool,
}

/// Mutable view of one embedded tree.
pub struct TreeMut<'a> {
    /// Records.
    pub records: &'a mut Records,
    /// Role.
    pub role: Role,
    /// Whether keys omit the shared end (per-link trees).
    pub per_link: bool,
}

fn key_of(records: &Records, role: Role, per_link: bool, node: u64) -> Key {
    let end = records.end(node, role);
    let other = records.other_end(node, role);
    if per_link {
        (other, node, 0)
    } else {
        (end, other, node)
    }
}

impl NodesRead for Tree<'_> {
    fn left(&self, node: u64) -> u64 {
        self.records.tree_cell(node, self.role, 0)
    }
    fn right(&self, node: u64) -> u64 {
        self.records.tree_cell(node, self.role, 1)
    }
    fn size(&self, node: u64) -> u64 {
        self.records.tree_cell(node, self.role, 2)
    }
    fn key(&self, node: u64) -> Key {
        key_of(self.records, self.role, self.per_link, node)
    }
}

impl NodesRead for TreeMut<'_> {
    fn left(&self, node: u64) -> u64 {
        self.records.tree_cell(node, self.role, 0)
    }
    fn right(&self, node: u64) -> u64 {
        self.records.tree_cell(node, self.role, 1)
    }
    fn size(&self, node: u64) -> u64 {
        self.records.tree_cell(node, self.role, 2)
    }
    fn key(&self, node: u64) -> Key {
        key_of(self.records, self.role, self.per_link, node)
    }
}

impl Nodes for TreeMut<'_> {
    fn set_left(&mut self, node: u64, value: u64) {
        self.records.set_tree_cell(node, self.role, 0, value);
    }
    fn set_right(&mut self, node: u64, value: u64) {
        self.records.set_tree_cell(node, self.role, 1, value);
    }
    fn set_size(&mut self, node: u64, value: u64) {
        self.records.set_tree_cell(node, self.role, 2, value);
    }
}
