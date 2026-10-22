//! The disk space map: who owns every block.
//!
//! Every block has exactly one owner (superblock, journal, a chunk table, a
//! chunk of one of the arrays, or one file extent), is free, or is pending
//! (freed since the last commit). The map is what the web visualizer draws.

use super::Fs;
use crate::device::BlockDevice;
use crate::varstore::class_size;
use std::fmt::Write;

/// Owner of an extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Block 0.
    Superblock,
    /// The journal region.
    Journal,
    /// The chunk table of array `array` (superblock slot order).
    Table {
        /// Array slot.
        array: usize,
    },
    /// Chunk `index` of array `array`.
    Chunk {
        /// Array slot.
        array: usize,
        /// Chunk number.
        index: usize,
    },
    /// The extent of a file.
    File {
        /// Inode number.
        ino: u64,
    },
    /// Free.
    Free,
    /// Freed since the last commit.
    Pending,
}

/// One extent of the map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extent {
    /// First block.
    pub start: u64,
    /// Length in blocks.
    pub len: u64,
    /// Owner.
    pub owner: Owner,
}

/// The whole map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceMap {
    /// Block size.
    pub block_size: u32,
    /// Total blocks.
    pub total_blocks: u64,
    /// Extents in address order, covering every block once.
    pub extents: Vec<Extent>,
}

/// Name of array slot `array`.
#[must_use]
pub fn array_name(array: usize) -> String {
    match array {
        0 => "links".into(),
        1 => "index".into(),
        2 => "inodes".into(),
        3 => "usage".into(),
        class => format!("var-{}", class_size(class - 4)),
    }
}

const fn kind_name(owner: Owner) -> &'static str {
    match owner {
        Owner::Superblock => "superblock",
        Owner::Journal => "journal",
        Owner::Table { .. } => "table",
        Owner::Chunk { .. } => "chunk",
        Owner::File { .. } => "file",
        Owner::Free => "free",
        Owner::Pending => "pending",
    }
}

pub(super) fn json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

impl<D: BlockDevice> Fs<D> {
    /// Every owned extent, sorted by start.
    #[must_use]
    pub fn owners(&self) -> Vec<(u64, u64, Owner)> {
        let mut owners = vec![(0, 1, Owner::Superblock)];
        if self.journal.blocks > 0 {
            owners.push((self.journal.start, self.journal.blocks, Owner::Journal));
        }
        for (array_index, array) in self.arrays().into_iter().enumerate() {
            let (table_start, table_blocks) = array.table_extent();
            if table_blocks > 0 {
                owners.push((
                    table_start,
                    table_blocks,
                    Owner::Table { array: array_index },
                ));
            }
            for (index, block) in array.chunk_blocks_iter().enumerate() {
                if block != u64::MAX {
                    owners.push((
                        block,
                        array.chunk_blocks(),
                        Owner::Chunk {
                            array: array_index,
                            index,
                        },
                    ));
                }
            }
        }
        for (slot, inode) in self.inodes.each() {
            if inode.blocks > 0 {
                owners.push((inode.start, inode.blocks, Owner::File { ino: slot + 1 }));
            }
        }
        owners.sort_unstable_by_key(|owner| owner.0);
        owners
    }

    /// The full map, free and pending extents included.
    #[must_use]
    pub fn space_map(&self) -> SpaceMap {
        let mut extents: Vec<Extent> = self
            .owners()
            .into_iter()
            .map(|(start, len, owner)| Extent { start, len, owner })
            .chain(self.space.free_extents().map(|(start, len)| Extent {
                start,
                len,
                owner: Owner::Free,
            }))
            .collect();
        extents.sort_unstable_by_key(|extent| extent.start);
        let mut out = Vec::with_capacity(extents.len());
        let mut next = 0;
        for extent in extents {
            if extent.start > next {
                out.push(Extent {
                    start: next,
                    len: extent.start - next,
                    owner: Owner::Pending,
                });
            }
            next = next.max(extent.start + extent.len);
            out.push(extent);
        }
        if next < self.sb.total_blocks {
            out.push(Extent {
                start: next,
                len: self.sb.total_blocks - next,
                owner: Owner::Pending,
            });
        }
        SpaceMap {
            block_size: self.sb.block_size,
            total_blocks: self.sb.total_blocks,
            extents: out,
        }
    }

    /// The map as JSON (the format the web visualizer reads).
    #[must_use]
    pub fn map_json(&self) -> String {
        let map = self.space_map();
        let stats = self.statfs();
        let mut out = String::new();
        let _ = write!(out, "{{\"format\":\"lfs-map\",\"version\":1,\"label\":");
        json_string(&mut out, &self.sb.label);
        let _ = write!(
            out,
            ",\"block_size\":{},\"total_blocks\":{},\"free_blocks\":{},\"largest_free\":{},\
             \"free_extents\":{},\"files\":{},\"links\":{},\"layout\":\"{}\",\"journal\":\"{}\",\
             \"journal_blocks\":{},\"seq\":{},\"extents\":[",
            map.block_size,
            map.total_blocks,
            stats.free,
            stats.largest_free,
            self.space.free_extents().count(),
            stats.files,
            stats.links,
            self.sb.layout.name(),
            self.config.pairs()[0].1,
            self.sb.journal_blocks,
            self.sb.seq,
        );
        for (index, extent) in map.extents.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"start\":{},\"len\":{},\"kind\":\"{}\"",
                extent.start,
                extent.len,
                kind_name(extent.owner)
            );
            match extent.owner {
                Owner::Table { array } => {
                    out.push_str(",\"array\":");
                    json_string(&mut out, &array_name(array));
                }
                Owner::Chunk { array, index } => {
                    out.push_str(",\"array\":");
                    json_string(&mut out, &array_name(array));
                    let _ = write!(out, ",\"chunk\":{index}");
                }
                Owner::File { ino } => {
                    let inode = self.inodes.get(ino - 1);
                    let _ = write!(out, ",\"ino\":{ino},\"size\":{}", inode.size);
                    if let Ok(path) = self.path_of(ino) {
                        out.push_str(",\"path\":");
                        json_string(&mut out, &path);
                    }
                }
                _ => {}
            }
            out.push('}');
        }
        out.push_str("]}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings_are_escaped() {
        let mut out = String::new();
        json_string(&mut out, "a\"b\\c\n");
        assert_eq!(out, "\"a\\\"b\\\\c\\u000a\"");
        assert_eq!(array_name(4), "var-16");
        assert_eq!(array_name(0), "links");
    }
}
