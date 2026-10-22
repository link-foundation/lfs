//! Regions of the volume: superblock, journal and chunk tables.

use super::{format_time, hex, Node, Part, DESCRIPTORS_OFFSET, DESCRIPTOR_SIZE, JOURNAL_HEADER};
use crate::codec::{get_i64, get_u32, get_u64};
use crate::device::BlockDevice;
use crate::fs::map::{array_name, Owner};
use crate::fs::Fs;
use crate::journal::DESCRIPTOR_MAGIC;
use crate::links::format_ref;
use crate::superblock::{ARRAYS, MAGIC};

impl<D: BlockDevice> Fs<D> {
    pub(super) fn volume_children(&self) -> Vec<Node> {
        let bs = self.bs();
        self.space_map()
            .extents
            .into_iter()
            .map(|extent| {
                let offset = extent.start * bs;
                let size = extent.len * bs;
                let blocks = if extent.len == 1 {
                    format!("block {}", extent.start)
                } else {
                    format!("blocks {}..{}", extent.start, extent.start + extent.len)
                };
                match extent.owner {
                    Owner::Superblock => Node::new("superblock", "superblock", offset, size)
                        .note(blocks)
                        .part(Part::Superblock),
                    Owner::Journal => Node::new("journal", "journal", offset, size)
                        .note(blocks)
                        .part(Part::Journal),
                    Owner::Table { array } => Node::new(
                        format!("{} chunk table", array_name(array)),
                        "table",
                        offset,
                        size,
                    )
                    .note(blocks)
                    .part(Part::Table(array)),
                    Owner::Chunk { array, index } => Node::new(
                        format!("{} chunk {index}", array_name(array)),
                        "chunk",
                        offset,
                        size,
                    )
                    .note(blocks)
                    .part(Part::Chunk { array, index }),
                    Owner::File { ino } => {
                        let path = self.path_of(ino).unwrap_or_else(|_| format!("#{ino}"));
                        Node::new(path, "file", offset, size)
                            .note(format!("inode {ino}, {blocks}"))
                            .part(Part::File { ino })
                    }
                    Owner::Free => Node::new("free", "free", offset, size).note(blocks),
                    Owner::Pending => Node::new("pending", "pending", offset, size)
                        .note(format!("{blocks}, freed since the last commit")),
                }
            })
            .collect()
    }

    pub(super) fn superblock_children(&self, b: &[u8]) -> Vec<Node> {
        let size = b.len() as u64;
        let u64_at = |offset: usize| get_u64(b, offset).to_string();
        vec![
            Node::new("magic", "field", 0, 8)
                .value(ascii(&b[..8]))
                .note(if &b[..8] == MAGIC {
                    "LFS image"
                } else {
                    "bad magic"
                }),
            Node::new("version", "field", 8, 4).value(get_u32(b, 8).to_string()),
            Node::new("block_size", "field", 12, 4).value(get_u32(b, 12).to_string()),
            Node::new("total_blocks", "field", 16, 8).value(u64_at(16)),
            Node::new("journal_start", "field", 24, 8).value(u64_at(24)),
            Node::new("journal_blocks", "field", 32, 8).value(u64_at(32)),
            Node::new("layout", "field", 40, 1)
                .value(b[40].to_string())
                .note(self.sb.layout.name()),
            Node::new("name_encoding", "field", 41, 1)
                .value(b[41].to_string())
                .note(self.sb.name_encoding.name()),
            Node::new("seq", "field", 48, 8)
                .value(u64_at(48))
                .note("sequence number of the last commit"),
            Node::new("links header", "header", 56, 48).part(Part::LinksHeader),
            Node::new("root", "link", 104, 8)
                .value(u64_at(104))
                .note("link of the root directory inode"),
            Node::new("config", "link", 112, 8)
                .value(format_ref(get_u64(b, 112)))
                .note("link of the persisted runtime configuration"),
            Node::new("volume_id", "field", 120, 16).value(hex(&b[120..136])),
            Node::new("label", "field", 136, 32).value(format!("{:?}", self.sb.label)),
            Node::new("created", "field", 168, 8).value(format_time(get_i64(b, 168))),
            Node::new("index_mode", "field", 176, 1)
                .value(b[176].to_string())
                .note(match b[176] {
                    0 => "off",
                    1 => "tree",
                    2 => "bitstring",
                    3 => "adaptive",
                    _ => "unknown",
                }),
            Node::new("index_threshold", "field", 184, 8)
                .value(f64::from_bits(get_u64(b, 184)).to_string())
                .note("bit string density threshold (f64)"),
            Node::new("index_hysteresis", "field", 192, 8)
                .value(f64::from_bits(get_u64(b, 192)).to_string())
                .note("fraction of the threshold below which a bit string becomes a tree (f64)"),
            Node::new(
                "array descriptors",
                "header",
                DESCRIPTORS_OFFSET,
                ARRAYS as u64 * DESCRIPTOR_SIZE,
            )
            .note("chunk table extent and chunk count of every array")
            .part(Part::Descriptors),
            Node::new("crc32", "field", size - 4, 4)
                .value(format!("{:#010x}", get_u32(b, size as usize - 4)))
                .note("CRC-32 of the block before it"),
        ]
    }

    pub(super) fn journal_children(&self, node: &Node, bytes: &[u8]) -> Vec<Node> {
        let bs = self.bs();
        if bytes.len() < 8 || &bytes[..8] != DESCRIPTOR_MAGIC {
            return vec![Node::new("empty", "unused", node.offset, node.size)
                .note("no transaction has been written here")];
        }
        let seq = get_u64(bytes, 8);
        let count = get_u64(bytes, 16);
        let per = (bs - JOURNAL_HEADER) / 8;
        let descriptors = count.div_ceil(per).max(1);
        let blocks = node.size / bs;
        if descriptors + count + 1 > blocks {
            return vec![
                Node::new("torn transaction", "unused", node.offset, node.size).note(format!(
                    "seq {seq} claims {count} blocks, more than the region holds"
                )),
            ];
        }
        let state = if seq <= self.sb.seq {
            "already checkpointed"
        } else {
            "not checkpointed: replayed at the next mount if its CRC is valid"
        };
        let mut out = Vec::new();
        for index in 0..descriptors {
            out.push(
                Node::new(
                    format!("descriptor {index}"),
                    "journal",
                    node.offset + index * bs,
                    bs,
                )
                .note(format!("transaction {seq}, {state}"))
                .part(Part::JournalDescriptor { first: index * per }),
            );
        }
        let targets: Vec<u64> = (0..count)
            .map(|slot| {
                let block = (slot / per) as usize;
                let at = block * bs as usize + JOURNAL_HEADER as usize + (slot % per) as usize * 8;
                get_u64(bytes, at)
            })
            .collect();
        for (index, target) in targets.iter().enumerate() {
            out.push(
                Node::new(
                    format!("copy of block {target}"),
                    "data",
                    node.offset + (descriptors + index as u64) * bs,
                    bs,
                )
                .note(format!(
                    "new contents of block {target} in transaction {seq}"
                )),
            );
        }
        out.push(
            Node::new(
                "commit",
                "journal",
                node.offset + (descriptors + count) * bs,
                bs,
            )
            .note(format!("transaction {seq} is durable once this block is"))
            .part(Part::JournalCommit),
        );
        out
    }

    pub(super) fn table_children(&self, node: &Node, bytes: &[u8], array: usize) -> Vec<Node> {
        let chunks = self.sb.arrays[array].chunks.min(node.size / 8);
        (0..chunks)
            .map(|index| {
                let block = get_u64(bytes, index as usize * 8);
                Node::new(
                    format!("chunk {index}"),
                    "field",
                    node.offset + index * 8,
                    8,
                )
                .value(block.to_string())
                .note(format!("block of {} chunk {index}", array_name(array)))
            })
            .collect()
    }
}

pub(super) fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| {
            if byte.is_ascii_graphic() {
                char::from(byte).to_string()
            } else {
                format!("\\x{byte:02x}")
            }
        })
        .collect()
}

pub(super) fn cells(
    at: u64,
    bytes: &[u8],
    fields: &[(&str, &str)],
    value: impl Fn(u64) -> String,
) -> Vec<Node> {
    fields
        .iter()
        .enumerate()
        .map(|(index, (name, note))| {
            Node::new(*name, "field", at + index as u64 * 8, 8)
                .value(value(get_u64(bytes, index * 8)))
                .note(*note)
        })
        .collect()
}

pub(super) fn journal_descriptor(node: &Node, bytes: &[u8], first: u64) -> Vec<Node> {
    let count = get_u64(bytes, 16);
    let per = (node.size - JOURNAL_HEADER) / 8;
    let mut out = vec![
        Node::new("magic", "field", node.offset, 8).value(ascii(&bytes[..8])),
        Node::new("seq", "field", node.offset + 8, 8).value(get_u64(bytes, 8).to_string()),
        Node::new("count", "field", node.offset + 16, 8)
            .value(count.to_string())
            .note("blocks in the transaction"),
    ];
    for slot in 0..per.min(count.saturating_sub(first)) {
        out.push(
            Node::new(
                format!("target {}", first + slot),
                "field",
                node.offset + JOURNAL_HEADER + slot * 8,
                8,
            )
            .value(get_u64(bytes, (JOURNAL_HEADER + slot * 8) as usize).to_string())
            .note("block the data block is written to"),
        );
    }
    out
}
