//! The disk blueprint: a byte-exact, recursive description of the image.
//!
//! [`SpaceMap`](super::SpaceMap) says who owns every block. The blueprint
//! goes further and dissects every byte: the volume splits into the same
//! regions as the space map (superblock, journal, chunk tables, chunks,
//! file extents, free space), chunks split into records, records into
//! fields, variable-size slots into their prefix, tree nodes or bit string
//! bytes, and so on down to single bytes where a field is one byte long.
//!
//! The tree is built lazily: [`Fs::blueprint`] expands it to a chosen depth
//! and [`Fs::blueprint_at`] expands one node found by its path of child
//! indices, so even a large image can be explored node by node. Every node
//! covers exactly the bytes of its children, in address order, with no gaps
//! (unused bytes are nodes of their own), so the leaves partition the image.
//!
//! The blueprint describes the committed image: the device bytes, decoded
//! with the in-memory knowledge of which link, inode or object each record
//! belongs to. A writable volume with pending changes is committed first.

use super::map::{array_name, json_string};
use super::{inode, Fs};
use crate::codec::{get_u32, get_u64};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::journal::COMMIT_MAGIC;
use crate::links::format_ref;
use crate::links::usage::{KIND_BITS, KIND_TREE};
use crate::superblock::ARRAYS;
use crate::varstore::{class_size, Handle};
use std::collections::HashMap;
use std::fmt::Write as _;

mod records;
mod regions;

use records::{bits_children, var_slot_children};
use regions::{ascii, cells, journal_descriptor};

/// Bytes of the header of a usage set (`count: u32`, `root: u32`).
const SET_HEADER: usize = 8;
/// Offset of the array descriptors in the superblock.
const DESCRIPTORS_OFFSET: u64 = 256;
/// Bytes per array descriptor.
const DESCRIPTOR_SIZE: u64 = 24;
/// Bytes of the journal descriptor header (`magic`, `seq`, `count`).
const JOURNAL_HEADER: u64 = 24;

/// What a node stands for, so it can be expanded later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Volume,
    Superblock,
    LinksHeader,
    Descriptors,
    Descriptor(usize),
    Journal,
    JournalDescriptor { first: u64 },
    JournalCommit,
    Table(usize),
    Chunk { array: usize, index: usize },
    Record { array: usize, record: u64 },
    VarSlot { class: usize, slot: u64 },
    UsageSet { class: usize, slot: u64, kind: u8 },
    TreeNode,
    Bits { bit_base: u64 },
    File { ino: u64 },
    FileData { ino: u64 },
    Leaf,
}

/// One node of the blueprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Short name (`superblock`, `link 12`, `source`, ...).
    pub name: String,
    /// Kind, for colouring: the region kind (`superblock`, `journal`,
    /// `table`, `chunk`, `file`, `free`, `pending`) or a part kind (`record`,
    /// `field`, `link`, `tree`, `bits`, `data`, `slack`, `unused`, ...).
    pub kind: &'static str,
    /// First byte of the node in the image.
    pub offset: u64,
    /// Length in bytes.
    pub size: u64,
    /// The decoded value of a field.
    pub value: Option<String>,
    /// What the bytes mean.
    pub note: Option<String>,
    /// The bytes as lower-case hex, when filled in by
    /// [`Fs::blueprint_hex`].
    pub hex: Option<String>,
    /// Children, if expanded.
    pub children: Vec<Self>,
    part: Part,
}

impl Node {
    fn new(name: impl Into<String>, kind: &'static str, offset: u64, size: u64) -> Self {
        Self {
            name: name.into(),
            kind,
            offset,
            size,
            value: None,
            note: None,
            hex: None,
            children: Vec::new(),
            part: Part::Leaf,
        }
    }

    const fn part(mut self, part: Part) -> Self {
        self.part = part;
        self
    }

    fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Whether the node has children (expanded or not).
    #[must_use]
    pub fn expandable(&self) -> bool {
        self.part != Part::Leaf
    }

    /// Number of nodes in the expanded subtree, this one included.
    #[must_use]
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Self::count).sum::<usize>()
    }

    /// The node as JSON (`"more": true` marks a node with children that
    /// are not expanded yet).
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        out.push_str("{\"name\":");
        json_string(out, &self.name);
        let _ = write!(
            out,
            ",\"kind\":\"{}\",\"offset\":{},\"size\":{}",
            self.kind, self.offset, self.size
        );
        if let Some(value) = &self.value {
            out.push_str(",\"value\":");
            json_string(out, value);
        }
        if let Some(note) = &self.note {
            out.push_str(",\"note\":");
            json_string(out, note);
        }
        if let Some(hex) = &self.hex {
            let _ = write!(out, ",\"hex\":\"{hex}\"");
        }
        if self.expandable() {
            out.push_str(",\"expandable\":true");
        }
        if self.expandable() {
            if self.children.is_empty() {
                out.push_str(",\"more\":true");
            } else {
                out.push_str(",\"children\":[");
                for (index, child) in self.children.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    child.write_json(out);
                }
                out.push(']');
            }
        }
        out.push('}');
    }

    /// The node as indented text, one line per node.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write_text(&mut out, 0);
        out
    }

    fn write_text(&self, out: &mut String, depth: usize) {
        let _ = write!(
            out,
            "{:#010x} {:>9}  {}{}",
            self.offset,
            self.size,
            "  ".repeat(depth),
            self.name
        );
        if let Some(value) = &self.value {
            let _ = write!(out, " = {value}");
        }
        if let Some(note) = &self.note {
            let _ = write!(out, "  ({note})");
        }
        if self.expandable() && self.children.is_empty() {
            out.push_str(" …");
        }
        out.push('\n');
        for child in &self.children {
            child.write_text(out, depth + 1);
        }
    }
}

/// Lower-case hex of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A UTC date for nanoseconds since the Unix epoch.
#[must_use]
pub fn format_time(ns: i64) -> String {
    let secs = ns.div_euclid(1_000_000_000);
    let nanos = ns.rem_euclid(1_000_000_000);
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{nanos:09} UTC",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

fn mode_text(mode: u32) -> String {
    let kind = match inode::FileKind::from_mode(mode) {
        Some(inode::FileKind::File) => "file",
        Some(inode::FileKind::Dir) => "directory",
        Some(inode::FileKind::Symlink) => "symlink",
        None => "unknown",
    };
    let mut perm = String::new();
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        perm.push(if bits & 4 != 0 { 'r' } else { '-' });
        perm.push(if bits & 2 != 0 { 'w' } else { '-' });
        perm.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    format!("{mode:#o} ({kind}, {perm})")
}

fn handle_text(raw: u64) -> String {
    let handle = Handle(raw);
    match handle.kind() {
        0 => "none".into(),
        kind => format!(
            "{} in var-{} slot {}",
            match kind {
                KIND_TREE => "tree",
                KIND_BITS => "bit string",
                _ => "object",
            },
            class_size(handle.class()),
            handle.slot()
        ),
    }
}

/// Names of the link record fields of a layout's data array.
const UNITED_FIELDS: [(&str, &str); 8] = [
    ("source", "link"),
    ("target", "link"),
    ("left_as_source", "tree"),
    ("right_as_source", "tree"),
    ("size_as_source", "tree"),
    ("left_as_target", "tree"),
    ("right_as_target", "tree"),
    ("size_as_target", "tree"),
];
const SPLIT_INDEX_FIELDS: [(&str, &str); 8] = [
    ("root_as_source", "tree"),
    ("left_as_source", "tree"),
    ("right_as_source", "tree"),
    ("size_as_source", "tree"),
    ("root_as_target", "tree"),
    ("left_as_target", "tree"),
    ("right_as_target", "tree"),
    ("size_as_target", "tree"),
];
const HEADER_FIELDS: [(&str, &str); 8] = [
    ("allocated", "highest allocated link index"),
    ("reserved", "reserved link capacity"),
    ("free", "free slots below allocated"),
    ("first_free", "first slot of the free list"),
    ("root_as_source", "root of the global sources tree"),
    ("root_as_target", "root of the global targets tree"),
    ("last_free", "last slot of the free list"),
    ("unused", "not used by the header"),
];
const SUPERBLOCK_HEADER_FIELDS: [(&str, &str); 6] = [
    ("allocated", "highest allocated link index"),
    ("free", "free slots below allocated"),
    ("first_free", "first slot of the free list"),
    ("root_as_source", "root of the global sources tree"),
    ("root_as_target", "root of the global targets tree"),
    ("last_free", "last slot of the free list"),
];

/// Who uses a variable-size slot: `(link, role, kind)`.
type SlotOwners = HashMap<(usize, u64), (u64, &'static str, u8)>;

impl<D: BlockDevice> Fs<D> {
    /// Reads `len` bytes of the image at byte `offset`.
    pub fn read_image(&mut self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let end = offset
            .checked_add(len)
            .filter(|&end| end <= self.device.len_bytes())
            .ok_or_else(|| Error::invalid(format!("bytes {offset}+{len} are outside the image")))?;
        let mut data = vec![0; (end - offset) as usize];
        self.device.read_at(offset, &mut data)?;
        Ok(data)
    }

    /// Fills in [`Node::hex`] for every node of the expanded tree up to
    /// `limit` bytes long.
    pub fn blueprint_hex(&mut self, node: &mut Node, limit: u64) -> Result<()> {
        if node.size <= limit {
            node.hex = Some(hex(&self.read_image(node.offset, node.size)?));
        }
        for child in &mut node.children {
            self.blueprint_hex(child, limit)?;
        }
        Ok(())
    }

    /// Size of the image described by the blueprint (whole blocks).
    #[must_use]
    pub fn image_len(&self) -> u64 {
        self.sb.total_blocks * u64::from(self.sb.block_size)
    }

    /// The blueprint expanded `depth` levels below the volume node
    /// (`usize::MAX`: everything).
    pub fn blueprint(&mut self, depth: usize) -> Result<Node> {
        self.blueprint_at(&[], depth)
    }

    /// The node at `path` (child indices from the volume node), expanded
    /// `depth` levels.
    pub fn blueprint_at(&mut self, path: &[usize], depth: usize) -> Result<Node> {
        if self.writable && self.dirty {
            self.commit()?;
        }
        let owners = self.slot_owners();
        let mut node = Node::new(
            if self.sb.label.is_empty() {
                "volume".to_string()
            } else {
                format!("volume {:?}", self.sb.label)
            },
            "volume",
            0,
            self.image_len(),
        )
        .note(format!(
            "{} blocks of {} bytes, {} layout",
            self.sb.total_blocks,
            self.sb.block_size,
            self.sb.layout.name()
        ))
        .part(Part::Volume);
        for (level, &index) in path.iter().enumerate() {
            let mut children = self.expand(&node, &owners)?;
            if index >= children.len() {
                return Err(Error::invalid(format!(
                    "blueprint path {:?} has no child {index} at level {level}",
                    &path[..level]
                )));
            }
            node = children.swap_remove(index);
        }
        self.expand_to(&mut node, depth, &owners)?;
        Ok(node)
    }

    /// The path of child indices to the deepest node containing byte
    /// `offset`.
    pub fn blueprint_locate(&mut self, offset: u64) -> Result<Vec<usize>> {
        if offset >= self.image_len() {
            return Err(Error::invalid(format!(
                "byte {offset} is outside the image"
            )));
        }
        let mut node = self.blueprint(0)?;
        let owners = self.slot_owners();
        let mut path = Vec::new();
        while node.expandable() {
            let children = self.expand(&node, &owners)?;
            let Some(index) = children
                .iter()
                .position(|child| (child.offset..child.offset + child.size).contains(&offset))
            else {
                break;
            };
            path.push(index);
            node = children.into_iter().nth(index).unwrap_or(node);
        }
        Ok(path)
    }

    fn expand_to(&mut self, node: &mut Node, depth: usize, owners: &SlotOwners) -> Result<()> {
        if depth == 0 || !node.expandable() {
            return Ok(());
        }
        node.children = self.expand(node, owners)?;
        for child in &mut node.children {
            self.expand_to(child, depth - 1, owners)?;
        }
        Ok(())
    }

    fn slot_owners(&self) -> SlotOwners {
        let usage = &self.links.usage;
        let mut owners = SlotOwners::new();
        for link in 0..usage.handles.capacity() {
            for (offset, role) in [(0, "source"), (8, "target")] {
                let handle = Handle(usage.handles.get(link, offset));
                if handle.is_some() {
                    owners.insert((handle.class(), handle.slot()), (link, role, handle.kind()));
                }
            }
        }
        owners
    }

    fn bs(&self) -> u64 {
        u64::from(self.sb.block_size)
    }

    fn expand(&mut self, node: &Node, owners: &SlotOwners) -> Result<Vec<Node>> {
        let bytes = self.read_image(node.offset, node.size)?;
        let mut out = match node.part {
            Part::Volume => self.volume_children(),
            Part::Superblock => self.superblock_children(&bytes),
            Part::LinksHeader => cells(node.offset, &bytes, &SUPERBLOCK_HEADER_FIELDS, |raw| {
                raw.to_string()
            }),
            Part::Descriptors => (0..ARRAYS)
                .map(|array| {
                    let offset = node.offset + array as u64 * DESCRIPTOR_SIZE;
                    Node::new(array_name(array), "descriptor", offset, DESCRIPTOR_SIZE)
                        .note(format!("array slot {array}"))
                        .part(Part::Descriptor(array))
                })
                .collect(),
            Part::Descriptor(_) => cells(
                node.offset,
                &bytes,
                &[
                    ("table_start", "first block of the chunk table"),
                    ("table_blocks", "blocks reserved for the chunk table"),
                    ("chunks", "chunks in use"),
                ],
                |raw| raw.to_string(),
            ),
            Part::Journal => self.journal_children(node, &bytes),
            Part::JournalDescriptor { first } => journal_descriptor(node, &bytes, first),
            Part::JournalCommit => vec![
                Node::new("magic", "field", node.offset, 8)
                    .value(ascii(&bytes[..8]))
                    .note(if &bytes[..8] == COMMIT_MAGIC {
                        "commit block"
                    } else {
                        "no commit block: the transaction is torn and ignored"
                    }),
                Node::new("seq", "field", node.offset + 8, 8).value(get_u64(&bytes, 8).to_string()),
                Node::new("count", "field", node.offset + 16, 8)
                    .value(get_u64(&bytes, 16).to_string()),
                Node::new("crc32", "field", node.offset + 24, 4)
                    .value(format!("{:#010x}", get_u32(&bytes, 24)))
                    .note("CRC-32 of the descriptor and data blocks"),
            ],
            Part::Table(array) => self.table_children(node, &bytes, array),
            Part::Chunk { array, index } => self.chunk_children(node, &bytes, array, index),
            Part::Record { array, record } => self.record_children(node, &bytes, array, record),
            Part::VarSlot { class, slot } => var_slot_children(node, &bytes, class, slot, owners),
            Part::UsageSet { kind, .. } => Self::usage_set_children(node, &bytes, kind),
            Part::TreeNode => {
                let link = get_u64(&bytes, 0);
                vec![
                    Node::new("link", "link", node.offset, 8)
                        .value(format_ref(link))
                        .note(self.describe_link(link)),
                    Node::new("left", "tree", node.offset + 8, 4)
                        .value(get_u32(&bytes, 8).to_string()),
                    Node::new("right", "tree", node.offset + 12, 4)
                        .value(get_u32(&bytes, 12).to_string()),
                    Node::new("size", "tree", node.offset + 16, 4)
                        .value(get_u32(&bytes, 16).to_string()),
                ]
            }
            Part::Bits { bit_base } => bits_children(node, &bytes, bit_base),
            Part::File { ino } => self.file_children(node, ino),
            Part::FileData { .. } => (0..node.size.div_ceil(self.bs()))
                .map(|block| {
                    let offset = node.offset + block * self.bs();
                    let size = self.bs().min(node.offset + node.size - offset);
                    Node::new(
                        format!("block {}", offset / self.bs()),
                        "data",
                        offset,
                        size,
                    )
                    .note(format!(
                        "file bytes {}..{}",
                        offset - node.offset,
                        offset - node.offset + size
                    ))
                })
                .collect(),
            Part::Leaf => Vec::new(),
        };
        fill_gaps(node, &mut out);
        Ok(out)
    }
}

/// Adds `unused` nodes for the bytes of `node` its children do not cover.
fn fill_gaps(node: &Node, children: &mut Vec<Node>) {
    if !node.expandable() {
        return;
    }
    children.sort_by_key(|child| child.offset);
    let mut out = Vec::with_capacity(children.len() + 2);
    let mut next = node.offset;
    let end = node.offset + node.size;
    for child in children.drain(..) {
        if child.offset > next {
            out.push(unused(next, child.offset - next));
        }
        next = next.max(child.offset + child.size);
        out.push(child);
    }
    if next < end {
        out.push(unused(next, end - next));
    }
    *children = out;
}

fn unused(offset: u64, size: u64) -> Node {
    Node::new("unused", "unused", offset, size).note("not used by this structure")
}

#[cfg(test)]
mod tests;
