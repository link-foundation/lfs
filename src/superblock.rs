//! The superblock (block 0): the single root of every on-disk structure.
//!
//! There is no allocation bitmap: free space is rebuilt at mount from the
//! owners listed here (journal, chunk tables, chunks) and from the file
//! extents in the inode records.

use crate::chunked::ArrayDescriptor;
use crate::codec::{crc32, get_i64, get_u32, get_u64, put_i64, put_u32, put_u64};
use crate::config::{IndexMode, Layout, NameEncoding};
use crate::error::{Error, Result};
use crate::links::Header;
use crate::varstore::CLASSES;

/// Superblock magic.
pub const MAGIC: &[u8; 8] = b"LINKSFS\0";
/// On-disk format version.
pub const VERSION: u32 = 1;
/// Smallest supported block size.
pub const MIN_BLOCK_SIZE: u32 = 1024;
/// Largest supported block size.
pub const MAX_BLOCK_SIZE: u32 = 65536;

/// Array slot: link data records.
pub const ARRAY_LINKS: usize = 0;
/// Array slot: split index records.
pub const ARRAY_INDEX: usize = 1;
/// Array slot: inode records.
pub const ARRAY_INODES: usize = 2;
/// Array slot: usage index handles.
pub const ARRAY_USAGE: usize = 3;
/// First array slot of the variable-size classes.
pub const ARRAY_VAR: usize = 4;
/// Number of array descriptors.
pub const ARRAYS: usize = ARRAY_VAR + CLASSES;

const ARRAYS_OFFSET: usize = 256;

/// Decoded superblock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Superblock {
    /// Block size in bytes.
    pub block_size: u32,
    /// Device size in blocks.
    pub total_blocks: u64,
    /// First journal block.
    pub journal_start: u64,
    /// Journal length in blocks.
    pub journal_blocks: u64,
    /// Record layout.
    pub layout: Layout,
    /// File name encoding.
    pub name_encoding: NameEncoding,
    /// Sequence number of the last commit.
    pub seq: u64,
    /// Links header.
    pub links: Header,
    /// Link of the root directory inode.
    pub root: u64,
    /// Handle of the stored runtime configuration.
    pub config: u64,
    /// Random volume identifier.
    pub volume_id: [u8; 16],
    /// Volume label (UTF-8, at most 32 bytes).
    pub label: String,
    /// Creation time (ns since the Unix epoch).
    pub created: i64,
    /// Mode of the optional usage index stored on disk.
    pub index_mode: IndexMode,
    /// Bit string threshold of that index (`f64` bits).
    pub index_threshold: u64,
    /// Hysteresis of that index (`f64` bits).
    pub index_hysteresis: u64,
    /// Array descriptors.
    pub arrays: [ArrayDescriptor; ARRAYS],
}

const fn index_code(mode: IndexMode) -> u8 {
    match mode {
        IndexMode::Off => 0,
        IndexMode::Tree => 1,
        IndexMode::Bitstring => 2,
        IndexMode::Adaptive => 3,
    }
}

impl Superblock {
    /// Encodes into one block.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let size = self.block_size as usize;
        let mut b = vec![0u8; size];
        b[..8].copy_from_slice(MAGIC);
        put_u32(&mut b, 8, VERSION);
        put_u32(&mut b, 12, self.block_size);
        put_u64(&mut b, 16, self.total_blocks);
        put_u64(&mut b, 24, self.journal_start);
        put_u64(&mut b, 32, self.journal_blocks);
        b[40] = self.layout.code() as u8;
        b[41] = match self.name_encoding {
            NameEncoding::Chars => 0,
            NameEncoding::Packed => 1,
        };
        put_u64(&mut b, 48, self.seq);
        self.links.write(&mut b[56..56 + Header::SIZE]);
        put_u64(&mut b, 104, self.root);
        put_u64(&mut b, 112, self.config);
        b[120..136].copy_from_slice(&self.volume_id);
        let label = self.label.as_bytes();
        let len = label.len().min(32);
        b[136..136 + len].copy_from_slice(&label[..len]);
        put_i64(&mut b, 168, self.created);
        b[176] = index_code(self.index_mode);
        put_u64(&mut b, 184, self.index_threshold);
        put_u64(&mut b, 192, self.index_hysteresis);
        for (index, array) in self.arrays.iter().enumerate() {
            let offset = ARRAYS_OFFSET + index * ArrayDescriptor::SIZE;
            array.write(&mut b[offset..offset + ArrayDescriptor::SIZE]);
        }
        let crc = crc32(&b[..size - 4]);
        put_u32(&mut b, size - 4, crc);
        b
    }

    /// Reads the block size from a raw superblock without validating it.
    #[must_use]
    pub fn peek_block_size(bytes: &[u8]) -> Option<u32> {
        (bytes.len() >= 16 && &bytes[..8] == MAGIC).then(|| get_u32(bytes, 12))
    }

    /// Decodes and validates a superblock block.
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < MIN_BLOCK_SIZE as usize || &b[..8] != MAGIC {
            return Err(Error::corrupt("not an LFS image (bad magic)"));
        }
        let version = get_u32(b, 8);
        if version != VERSION {
            return Err(Error::Unsupported(format!("format version {version}")));
        }
        let block_size = get_u32(b, 12);
        if block_size as usize != b.len() {
            return Err(Error::corrupt("block size mismatch"));
        }
        let size = b.len();
        if get_u32(b, size - 4) != crc32(&b[..size - 4]) {
            return Err(Error::corrupt("superblock checksum mismatch"));
        }
        let layout =
            Layout::from_code(u32::from(b[40])).ok_or_else(|| Error::corrupt("unknown layout"))?;
        let name_encoding = match b[41] {
            0 => NameEncoding::Chars,
            1 => NameEncoding::Packed,
            _ => return Err(Error::corrupt("unknown name encoding")),
        };
        let index_mode = match b[176] {
            0 => IndexMode::Off,
            1 => IndexMode::Tree,
            2 => IndexMode::Bitstring,
            3 => IndexMode::Adaptive,
            _ => return Err(Error::corrupt("unknown index mode")),
        };
        let mut arrays = [ArrayDescriptor::default(); ARRAYS];
        for (index, array) in arrays.iter_mut().enumerate() {
            let offset = ARRAYS_OFFSET + index * ArrayDescriptor::SIZE;
            *array = ArrayDescriptor::read(&b[offset..offset + ArrayDescriptor::SIZE]);
        }
        let label_bytes = &b[136..168];
        let end = label_bytes.iter().position(|&c| c == 0).unwrap_or(32);
        let mut volume_id = [0u8; 16];
        volume_id.copy_from_slice(&b[120..136]);
        Ok(Self {
            block_size,
            total_blocks: get_u64(b, 16),
            journal_start: get_u64(b, 24),
            journal_blocks: get_u64(b, 32),
            layout,
            name_encoding,
            seq: get_u64(b, 48),
            links: Header::read(&b[56..56 + Header::SIZE]),
            root: get_u64(b, 104),
            config: get_u64(b, 112),
            volume_id,
            label: String::from_utf8_lossy(&label_bytes[..end]).into_owned(),
            created: get_i64(b, 168),
            index_mode,
            index_threshold: get_u64(b, 184),
            index_hysteresis: get_u64(b, 192),
            arrays,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superblock_round_trips_and_detects_damage() {
        let mut arrays = [ArrayDescriptor::default(); ARRAYS];
        arrays[ARRAY_INODES] = ArrayDescriptor {
            table_start: 9,
            table_blocks: 1,
            chunks: 3,
        };
        let sb = Superblock {
            block_size: 1024,
            total_blocks: 4096,
            journal_start: 1,
            journal_blocks: 64,
            layout: Layout::Split,
            name_encoding: NameEncoding::Packed,
            seq: 77,
            links: Header {
                allocated: 12,
                free: 2,
                ..Header::default()
            },
            root: 10,
            config: 5,
            volume_id: [7; 16],
            label: "backup".into(),
            created: 1_700_000_000_000_000_000,
            index_mode: IndexMode::Adaptive,
            index_threshold: 0.25f64.to_bits(),
            index_hysteresis: 0.5f64.to_bits(),
            arrays,
        };
        let mut bytes = sb.encode();
        assert_eq!(Superblock::decode(&bytes).unwrap(), sb);
        assert_eq!(Superblock::peek_block_size(&bytes), Some(1024));
        bytes[300] ^= 1;
        assert!(matches!(Superblock::decode(&bytes), Err(Error::Corrupt(_))));
    }
}
