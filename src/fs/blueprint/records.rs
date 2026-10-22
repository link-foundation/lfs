//! Chunks, records, variable-size objects, usage sets and file extents.

use super::regions::cells;
use super::{
    format_time, handle_text, mode_text, Node, Part, SlotOwners, HEADER_FIELDS, SET_HEADER,
    SPLIT_INDEX_FIELDS, UNITED_FIELDS,
};
use crate::codec::{get_u32, get_u64};
use crate::config::Layout;
use crate::device::BlockDevice;
use crate::fs::{inode, Fs, CONFIG, DIR, FILE, RESERVED_POINTS, SYMLINK};
use crate::links::usage::{KIND_BITS, KIND_TREE, NODE_BYTES};
use crate::links::{as_external, format_ref, is_internal};
use crate::superblock::{ARRAY_INDEX, ARRAY_INODES, ARRAY_LINKS, ARRAY_USAGE, ARRAY_VAR};
use crate::varstore::{class_size, PREFIX};

impl<D: BlockDevice> Fs<D> {
    pub(super) fn record_size(&self, array: usize) -> u64 {
        self.arrays()[array].record_size() as u64
    }

    pub(super) fn chunk_children(
        &self,
        node: &Node,
        bytes: &[u8],
        array: usize,
        index: usize,
    ) -> Vec<Node> {
        let size = self.record_size(array);
        let per = node.size / size;
        let first = index as u64 * per;
        let mut out: Vec<Node> = Vec::new();
        let mut run: Option<(u64, u64)> = None;
        let flush = |out: &mut Vec<Node>, run: &mut Option<(u64, u64)>| {
            if let Some((from, to)) = run.take() {
                let name = if from == to {
                    format!("record {from}")
                } else {
                    format!("records {from}..={to}")
                };
                out.push(
                    Node::new(
                        name,
                        "unused",
                        node.offset + (from - first) * size,
                        (to - from + 1) * size,
                    )
                    .note("unused (all zero)"),
                );
            }
        };
        for record in first..first + per {
            let at = ((record - first) * size) as usize;
            let raw = &bytes[at..at + size as usize];
            let unused = raw.iter().all(|&byte| byte == 0) && !self.record_in_use(array, record);
            if unused {
                run = Some(run.map_or((record, record), |(from, _)| (from, record)));
                continue;
            }
            flush(&mut out, &mut run);
            let (title, remark) = self.record_title(array, record, raw);
            let part = if array >= ARRAY_VAR {
                Part::VarSlot {
                    class: array - ARRAY_VAR,
                    slot: record,
                }
            } else {
                Part::Record { array, record }
            };
            out.push(
                Node::new(title, "record", node.offset + (record - first) * size, size)
                    .note(remark)
                    .part(part),
            );
        }
        flush(&mut out, &mut run);
        out
    }

    /// Whether record 0 of `array` holds the links header.
    pub(super) const fn holds_header(&self, array: usize) -> bool {
        matches!(
            (self.sb.layout, array),
            (Layout::United, ARRAY_LINKS) | (Layout::Split, ARRAY_INDEX)
        )
    }

    pub(super) fn record_in_use(&self, array: usize, record: u64) -> bool {
        match array {
            _ if record == 0 => self.holds_header(array),
            ARRAY_LINKS | ARRAY_INDEX => self.links.exists(record),
            ARRAY_INODES => self.inodes.get(record).is_used(),
            _ => false,
        }
    }

    pub(super) fn record_title(&self, array: usize, record: u64, raw: &[u8]) -> (String, String) {
        let layout = self.sb.layout;
        match array {
            _ if record == 0 && self.holds_header(array) => (
                "header".into(),
                "links header (doublets LinksHeader)".into(),
            ),
            ARRAY_LINKS | ARRAY_INDEX | ARRAY_USAGE => {
                let prefix = match array {
                    ARRAY_LINKS => "link",
                    ARRAY_INDEX => "index of link",
                    _ => "usage of link",
                };
                if self.links.exists(record) {
                    (format!("{prefix} {record}"), self.describe_link(record))
                } else {
                    (
                        format!("free slot {record}"),
                        if layout == Layout::United && array == ARRAY_LINKS {
                            "free link slot (unused-links list: previous = source, next = target)"
                                .into()
                        } else {
                            "free link slot".into()
                        },
                    )
                }
            }
            ARRAY_INODES => {
                let inode = inode::Inode::read(raw);
                let ino = record + 1;
                (
                    format!("inode {ino}"),
                    if inode.is_used() {
                        format!(
                            "{} {}",
                            mode_text(inode.mode),
                            self.path_of(ino).unwrap_or_else(|_| "(unlinked)".into())
                        )
                    } else {
                        "free slot".into()
                    },
                )
            }
            class => {
                let len = get_u32(raw, 0);
                (
                    format!("slot {record}"),
                    if len == 0 {
                        "free slot".into()
                    } else {
                        format!(
                            "{len}-byte object in a {}-byte slot",
                            class_size(class - ARRAY_VAR)
                        )
                    },
                )
            }
        }
    }

    pub(super) fn record_children(
        &self,
        node: &Node,
        bytes: &[u8],
        array: usize,
        record: u64,
    ) -> Vec<Node> {
        let at = node.offset;
        match array {
            _ if record == 0 && self.holds_header(array) => {
                cells(at, bytes, &HEADER_FIELDS, |raw| raw.to_string())
            }
            ARRAY_LINKS => {
                let fields: &[(&str, &str)] = if self.sb.layout == Layout::United {
                    &UNITED_FIELDS
                } else {
                    &UNITED_FIELDS[..2]
                };
                fields
                    .iter()
                    .enumerate()
                    .map(|(index, (name, kind))| {
                        let raw = get_u64(bytes, index * 8);
                        let mut field = Node::new(*name, kind, at + index as u64 * 8, 8);
                        if *kind == "link" {
                            field = field.value(format_ref(raw));
                            if raw != 0 && self.links.exists(record) {
                                field = field.note(self.describe_link(raw));
                            }
                        } else {
                            field = field.value(raw.to_string());
                        }
                        field
                    })
                    .collect()
            }
            ARRAY_INDEX => SPLIT_INDEX_FIELDS
                .iter()
                .enumerate()
                .map(|(index, (name, kind))| {
                    Node::new(*name, kind, at + index as u64 * 8, 8)
                        .value(get_u64(bytes, index * 8).to_string())
                })
                .collect(),
            ARRAY_INODES => {
                let inode = inode::Inode::read(bytes);
                vec![
                    Node::new("mode", "field", at, 4).value(mode_text(inode.mode)),
                    Node::new("nlink", "field", at + 4, 4).value(inode.nlink.to_string()),
                    Node::new("uid", "field", at + 8, 4).value(inode.uid.to_string()),
                    Node::new("gid", "field", at + 12, 4).value(inode.gid.to_string()),
                    Node::new("size", "field", at + 16, 8).value(inode.size.to_string()),
                    Node::new("start", "field", at + 24, 8)
                        .value(inode.start.to_string())
                        .note("first block of the extent"),
                    Node::new("blocks", "field", at + 32, 8)
                        .value(inode.blocks.to_string())
                        .note("blocks reserved for the extent"),
                    Node::new("mtime", "field", at + 40, 8).value(format_time(inode.mtime)),
                    Node::new("ctime", "field", at + 48, 8).value(format_time(inode.ctime)),
                    Node::new("atime", "field", at + 56, 8).value(format_time(inode.atime)),
                ]
            }
            ARRAY_USAGE => vec![
                Node::new("as_source", "field", at, 8)
                    .value(handle_text(get_u64(bytes, 0)))
                    .note(format!("links whose source is {record}")),
                Node::new("as_target", "field", at + 8, 8)
                    .value(handle_text(get_u64(bytes, 8)))
                    .note(format!("links whose target is {record}")),
            ],
            _ => Vec::new(),
        }
    }

    pub(super) fn usage_set_children(node: &Node, bytes: &[u8], kind: u8) -> Vec<Node> {
        let count = u64::from(get_u32(bytes, 0));
        let mut out = vec![Node::new("count", "field", node.offset, 4)
            .value(count.to_string())
            .note("number of users")];
        if kind == KIND_TREE {
            out.push(
                Node::new("root", "tree", node.offset + 4, 4)
                    .value(get_u32(bytes, 4).to_string())
                    .note("root node of the size-balanced tree"),
            );
            let fit = ((node.size - SET_HEADER as u64) / NODE_BYTES as u64).min(count);
            for index in 0..fit {
                out.push(
                    Node::new(
                        format!("node {}", index + 1),
                        "tree",
                        node.offset + SET_HEADER as u64 + index * NODE_BYTES as u64,
                        NODE_BYTES as u64,
                    )
                    .note(format!(
                        "user {}",
                        format_ref(get_u64(bytes, SET_HEADER + index as usize * NODE_BYTES))
                    ))
                    .part(Part::TreeNode),
                );
            }
        } else {
            out.push(Node::new("reserved", "unused", node.offset + 4, 4));
            out.push(
                Node::new(
                    "bits",
                    "bits",
                    node.offset + SET_HEADER as u64,
                    node.size - SET_HEADER as u64,
                )
                .note("bit L is set when link L is a user")
                .part(Part::Bits { bit_base: 0 }),
            );
        }
        out
    }

    pub(super) fn file_children(&self, node: &Node, ino: u64) -> Vec<Node> {
        let size = self.inodes.get(ino - 1).size.min(node.size);
        let mut out = Vec::new();
        if size > 0 {
            out.push(
                Node::new("data", "data", node.offset, size)
                    .note(format!("{size} bytes of file contents"))
                    .part(Part::FileData { ino }),
            );
        }
        if size < node.size {
            out.push(
                Node::new("slack", "slack", node.offset + size, node.size - size)
                    .note("reserved room after the end of the file (growth or block rounding)"),
            );
        }
        out
    }

    /// What a link means in the file system schema.
    #[must_use]
    pub fn describe_link(&self, link: u64) -> String {
        const MARKERS: [&str; 5] = ["ONE", "NUMBER", "STRING", "LIST", "IDENTIFIED"];
        if let Some(value) = as_external(link) {
            return format!("external value {value}");
        }
        if link == 0 {
            return "null".into();
        }
        if !self.links.exists(link) {
            return "not a live link".into();
        }
        if (1..=5).contains(&link) {
            return format!(
                "{} marker point of binary links notation",
                MARKERS[link as usize - 1]
            );
        }
        match link {
            FILE => return "FILE type point".into(),
            DIR => return "DIR type point".into(),
            SYMLINK => return "SYMLINK type point".into(),
            CONFIG => return "CONFIG marker point".into(),
            _ if link <= RESERVED_POINTS => return "reserved point".into(),
            _ => {}
        }
        let source = self.links.source(link);
        let target = self.links.target(link);
        if let Some(ino) = self.ino_of_link(link) {
            let path = self.path_of(ino).unwrap_or_else(|_| "(unlinked)".into());
            return format!("inode {ino} {path}");
        }
        if source == links_notation::binary::packet::STRING {
            return crate::fs::names::read(&self.links, link, self.sb.name_encoding)
                .map_or_else(|_| "string".into(), |text| format!("string {text:?}"));
        }
        if source == CONFIG {
            return "persisted runtime configuration".into();
        }
        if let Some(value) = as_external(source) {
            if target == 0 || (is_internal(target) && self.links.exists(target)) {
                return format!("string chain cell {}", self.chain_cell(value));
            }
        }
        if let Some(ino) = self.ino_of_link(source) {
            if is_internal(target) && crate::fs::names::is_string(&self.links, target) {
                let name = crate::fs::names::read(&self.links, target, self.sb.name_encoding)
                    .unwrap_or_default();
                let kind = self.inodes.get(ino - 1).kind();
                return if kind == Some(inode::FileKind::Symlink) {
                    format!("symlink {ino} points at {name:?}")
                } else {
                    format!("entry {name:?} in directory {ino}")
                };
            }
        }
        if is_internal(source) && self.links.exists(source) {
            if let (Some(dir), Some(ino)) = (
                self.ino_of_link(self.links.source(source)),
                self.ino_of_link(target),
            ) {
                return format!("binding: entry {source} of directory {dir} → inode {ino}");
            }
        }
        "user link".into()
    }

    pub(super) fn chain_cell(&self, value: u64) -> String {
        match self.sb.name_encoding {
            crate::config::NameEncoding::Chars => u32::try_from(value)
                .ok()
                .and_then(char::from_u32)
                .map_or_else(|| format!("${value}"), |c| format!("{c:?}")),
            crate::config::NameEncoding::Packed => {
                let len = (value >> 56).min(7) as usize;
                let bytes: Vec<u8> = (0..len).map(|index| (value >> (index * 8)) as u8).collect();
                format!("{:?}", String::from_utf8_lossy(&bytes))
            }
        }
    }
}

pub(super) fn var_slot_children(
    node: &Node,
    bytes: &[u8],
    class: usize,
    slot: u64,
    owners: &SlotOwners,
) -> Vec<Node> {
    let len = u64::from(get_u32(bytes, 0));
    let mut out = vec![Node::new("length", "field", node.offset, PREFIX as u64)
        .value(len.to_string())
        .note("payload length (0: free slot)")];
    let payload = node.size - PREFIX as u64;
    match owners.get(&(class, slot)) {
        Some(&(link, role, kind)) if kind == KIND_TREE || kind == KIND_BITS => out.push(
            Node::new(
                if kind == KIND_TREE {
                    "tree"
                } else {
                    "bit string"
                },
                if kind == KIND_TREE { "tree" } else { "bits" },
                node.offset + PREFIX as u64,
                payload,
            )
            .note(format!("links whose {role} is {link}"))
            .part(Part::UsageSet { class, slot, kind }),
        ),
        _ if len > 0 => out.push(
            Node::new("payload", "data", node.offset + PREFIX as u64, payload)
                .note(format!("{len} bytes")),
        ),
        _ => {}
    }
    out
}

pub(super) fn bits_children(node: &Node, bytes: &[u8], bit_base: u64) -> Vec<Node> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let first = bit_base + index as u64 * 8;
        let byte = bytes[index];
        if byte == 0 {
            let run = bytes[index..].iter().take_while(|&&b| b == 0).count();
            out.push(
                Node::new(
                    format!("links {first}..{}", first + run as u64 * 8),
                    "bits",
                    node.offset + index as u64,
                    run as u64,
                )
                .value("0")
                .note("none of these links is a user"),
            );
            index += run;
            continue;
        }
        let users: Vec<String> = (0..8)
            .filter(|bit| byte & (1 << bit) != 0)
            .map(|bit| (first + bit).to_string())
            .collect();
        out.push(
            Node::new(
                format!("links {first}..{}", first + 8),
                "bits",
                node.offset + index as u64,
                1,
            )
            .value(format!("{byte:#010b}"))
            .note(format!("users {}", users.join(", "))),
        );
        index += 1;
    }
    out
}
