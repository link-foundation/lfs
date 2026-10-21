//! The file system: files and directories stored as links.
//!
//! # Schema
//!
//! Links `1..=5` are the binary links notation marker points (`ONE`,
//! `NUMBER`, `STRING`, `LIST`, `IDENTIFIED`); `6..=9` are the type points
//! [`FILE`], [`DIR`], [`SYMLINK`] and [`CONFIG`]. Then:
//!
//! ```text
//! inode     I = (TYPE, $slot)        slot of the 64-byte inode record
//! name      N = (STRING, chain)      binary links notation string
//! entry     E = (D, N)               name N in directory D
//! binding   B = (E, I)               the entry points at inode I
//! symlink   S = (I, N)               symbolic link I points at text N
//! config    C = (CONFIG, N)          persisted runtime configuration
//! ```
//!
//! Because a link is unique by its source and target, a name exists at most
//! once per directory and every name text is stored once per volume. Hard
//! links are several bindings of one inode; the parent of a directory is
//! found from its only binding.
//!
//! # Data
//!
//! Every file is one contiguous extent. Growing tries, in order: growth in
//! place (with `growth-percent` extra room), exact growth in place, moving
//! the link chunks that block growth in place, relocating the file, and
//! compacting the volume. Link, inode and index chunks fill the gaps between
//! files, so the volume never fragments.

mod check;
mod commit;
mod control;
mod dir;
mod file;
pub mod inode;
pub mod interchange;
pub mod map;
pub mod names;
#[cfg(test)]
mod tests;

pub use control::{CONTROL_DIR, CONTROL_DIR_INO, CONTROL_FILES};
pub use dir::{Creator, DirEntry};
pub use file::CompactReport;
pub use inode::{FileKind, Inode};
pub use map::{array_name, Extent, Owner, SpaceMap};

use crate::chunked::{ArrayDescriptor, ChunkedArray};
use crate::config::{FormatOptions, JournalMode, Layout, NameEncoding, RuntimeConfig};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::journal::{self, Journal};
use crate::links::records::Records;
use crate::links::usage::UsageIndex;
use crate::links::{external, Links};
use crate::space::Space;
use crate::superblock::{
    Superblock, ARRAYS, ARRAY_INDEX, ARRAY_INODES, ARRAY_LINKS, ARRAY_USAGE, ARRAY_VAR,
    MAX_BLOCK_SIZE, MIN_BLOCK_SIZE,
};
use crate::varstore::VarStore;
use inode::Inodes;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

/// Type point of regular files.
pub const FILE: u64 = 6;
/// Type point of directories.
pub const DIR: u64 = 7;
/// Type point of symbolic links.
pub const SYMLINK: u64 = 8;
/// Marker point of the persisted configuration.
pub const CONFIG: u64 = 9;
/// Number of points created by `format`.
pub const RESERVED_POINTS: u64 = 9;
/// Inode number of the root directory.
pub const ROOT_INO: u64 = 1;
/// Smallest volume in blocks.
pub const MIN_BLOCKS: u64 = 16;

/// Current time in nanoseconds since the Unix epoch.
#[must_use]
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as i64)
}

/// Attributes of a file, as reported to the operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    /// Inode number.
    pub ino: u64,
    /// Kind.
    pub kind: FileKind,
    /// Permission bits.
    pub perm: u16,
    /// Hard links.
    pub nlink: u32,
    /// Owner.
    pub uid: u32,
    /// Group.
    pub gid: u32,
    /// Size in bytes.
    pub size: u64,
    /// Bytes reserved on the device.
    pub allocated: u64,
    /// Access time (ns).
    pub atime: i64,
    /// Modification time (ns).
    pub mtime: i64,
    /// Change time (ns).
    pub ctime: i64,
}

/// Attribute changes (`None` keeps a value).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetAttr {
    /// New permission bits.
    pub perm: Option<u16>,
    /// New owner.
    pub uid: Option<u32>,
    /// New group.
    pub gid: Option<u32>,
    /// New size (truncate or extend with zeros).
    pub size: Option<u64>,
    /// New access time.
    pub atime: Option<i64>,
    /// New modification time.
    pub mtime: Option<i64>,
}

/// Volume statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatFs {
    /// Block size.
    pub block_size: u32,
    /// Total blocks.
    pub blocks: u64,
    /// Free blocks.
    pub free: u64,
    /// Largest contiguous free extent in blocks.
    pub largest_free: u64,
    /// Files (used inode slots).
    pub files: u64,
    /// Live links.
    pub links: u64,
}

/// A mounted LFS volume.
pub struct Fs<D: BlockDevice> {
    device: D,
    sb: Superblock,
    journal: Journal,
    space: Space,
    links: Links,
    inodes: Inodes,
    config: RuntimeConfig,
    /// Overwrites of committed file blocks waiting to be journaled
    /// (full journal mode).
    cache: BTreeMap<u64, Vec<u8>>,
    /// Open handles per inode slot (unlinked open files stay until closed).
    open: HashMap<u64, u32>,
    /// Pending writes to virtual control files.
    control: HashMap<u64, Vec<u8>>,
    writable: bool,
    dirty: bool,
    journal_clean: bool,
    ops: u64,
}

impl<D: BlockDevice> std::fmt::Debug for Fs<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fs")
            .field("block_size", &self.sb.block_size)
            .field("total_blocks", &self.sb.total_blocks)
            .field("seq", &self.sb.seq)
            .finish_non_exhaustive()
    }
}

const fn record_size(layout: Layout) -> usize {
    if matches!(layout, Layout::United) {
        64
    } else {
        16
    }
}

fn read_superblock(device: &mut dyn BlockDevice) -> Result<Superblock> {
    let mut head = vec![0u8; MIN_BLOCK_SIZE as usize];
    device.read_at(0, &mut head)?;
    let block_size = Superblock::peek_block_size(&head)
        .filter(|size| (MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(size) && size.is_power_of_two())
        .ok_or_else(|| Error::corrupt("not an LFS image (bad magic)"))?;
    Superblock::decode(&journal::read_block(device, 0, block_size as usize)?)
}

/// Replays a committed but not checkpointed transaction. Returns whether
/// anything was replayed.
fn recover(device: &mut dyn BlockDevice, writable: bool) -> Result<bool> {
    let tx = if let Ok(sb) = read_superblock(device) {
        {
            let region = Journal {
                start: sb.journal_start,
                blocks: sb.journal_blocks,
                block_size: sb.block_size as usize,
            };
            region
                .read(device)
                .ok()
                .flatten()
                .filter(|tx| tx.seq == sb.seq + 1)
        }
    } else {
        {
            // A torn superblock: the transaction that wrote it is in the
            // journal, which always starts at block 1.
            let mut found = None;
            let mut size = MIN_BLOCK_SIZE;
            while size <= MAX_BLOCK_SIZE && found.is_none() {
                let blocks = (device.len_bytes() / u64::from(size)).saturating_sub(1);
                let region = Journal {
                    start: 1,
                    blocks,
                    block_size: size as usize,
                };
                found = region
                    .read(device)
                    .ok()
                    .flatten()
                    .filter(|tx| tx.touches(0));
                size *= 2;
            }
            found
        }
    };
    let Some(tx) = tx else {
        return Ok(false);
    };
    if !writable {
        return Err(Error::Unsupported(
            "the journal holds a committed transaction; open the volume read-write once to replay it"
                .into(),
        ));
    }
    journal::checkpoint(device, &tx.writes)?;
    Ok(true)
}

impl<D: BlockDevice> Fs<D> {
    /// Creates a new volume on `device`.
    pub fn format(mut device: D, options: &FormatOptions, label: &str) -> Result<Self> {
        let block_size = options.block_size;
        if !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size) || !block_size.is_power_of_two()
        {
            return Err(Error::invalid(format!(
                "block size must be a power of two within {MIN_BLOCK_SIZE}..={MAX_BLOCK_SIZE}"
            )));
        }
        let bs = block_size as usize;
        let total = device.len_bytes() / u64::from(block_size);
        if total < MIN_BLOCKS {
            return Err(Error::invalid(format!(
                "the device holds {total} blocks; at least {MIN_BLOCKS} are needed"
            )));
        }
        let journal_blocks = options
            .journal_blocks
            .unwrap_or_else(|| Journal::default_blocks(total));
        if journal_blocks != 0 && (journal_blocks < 3 || journal_blocks > total / 2) {
            return Err(Error::invalid(
                "the journal needs at least 3 blocks and at most half of the device",
            ));
        }
        let journal = Journal {
            start: 1,
            blocks: journal_blocks,
            block_size: bs,
        };
        journal.wipe(&mut device)?;
        let mut space = Space::new(total);
        space.mark_used(0, 1);
        space.mark_used(1, journal_blocks);
        let mut links = Links::new(options.layout, bs);
        for _ in 0..RESERVED_POINTS {
            links.create_point()?;
        }
        let mut inodes = Inodes::new(bs);
        let slot = inodes.allocate();
        let created = now();
        inodes.set(
            slot,
            &Inode {
                mode: inode::S_IFDIR | 0o755,
                nlink: 2,
                mtime: created,
                ctime: created,
                atime: created,
                ..Inode::default()
            },
        );
        let root = links.create(DIR, external(slot))?;
        let config = RuntimeConfig::default();
        let mut volume_id = [0u8; 16];
        let seed = created as u64 ^ (u64::from(std::process::id()) << 32) ^ total;
        for (index, byte) in volume_id.iter_mut().enumerate() {
            *byte = (seed.rotate_left(index as u32 * 7) >> (index % 8)) as u8 ^ index as u8;
        }
        let sb = Superblock {
            block_size,
            total_blocks: total,
            journal_start: 1,
            journal_blocks,
            layout: options.layout,
            name_encoding: options.name_encoding,
            seq: 0,
            links: links.header(),
            root,
            config: 0,
            volume_id,
            label: label.chars().take(32).collect(),
            created,
            index_mode: config.index,
            index_threshold: config.bitstring_threshold.fraction().to_bits(),
            index_hysteresis: (f64::from(config.hysteresis_percent) / 100.0).to_bits(),
            arrays: [ArrayDescriptor::default(); ARRAYS],
        };
        let mut fs = Self {
            device,
            sb,
            journal,
            space,
            links,
            inodes,
            config: config.clone(),
            cache: BTreeMap::new(),
            open: HashMap::new(),
            control: HashMap::new(),
            writable: true,
            dirty: true,
            journal_clean: false,
            ops: 0,
        };
        fs.store_config(&config)?;
        fs.commit()?;
        Ok(fs)
    }

    /// Opens a volume, replaying the journal first if needed.
    ///
    /// `overrides` are applied on top of the persisted configuration for
    /// this mount only (they are persisted only if changed again later).
    pub fn open(mut device: D, writable: bool, overrides: &[(String, String)]) -> Result<Self> {
        recover(&mut device, writable)?;
        let sb = read_superblock(&mut device)?;
        let bs = sb.block_size as usize;
        if device.len_bytes() < sb.total_blocks * u64::from(sb.block_size) {
            return Err(Error::corrupt("the device is smaller than the volume"));
        }
        let journal = Journal {
            start: sb.journal_start,
            blocks: sb.journal_blocks,
            block_size: bs,
        };
        let mut space = Space::new(sb.total_blocks);
        if !space.mark_used(0, 1) || !space.mark_used(journal.start, journal.blocks) {
            return Err(Error::corrupt("the journal lies outside the volume"));
        }
        let dev: &mut dyn BlockDevice = &mut device;
        let load = |dev: &mut dyn BlockDevice, slot: usize, size: usize, space: &mut Space| {
            ChunkedArray::load(dev, sb.arrays[slot], size, bs, space)
        };
        let records = Records {
            layout: sb.layout,
            data: load(dev, ARRAY_LINKS, record_size(sb.layout), &mut space)?,
            index: load(dev, ARRAY_INDEX, 64, &mut space)?,
        };
        let usage = UsageIndex {
            handles: load(dev, ARRAY_USAGE, 16, &mut space)?,
            var: VarStore::load(dev, &sb.arrays[ARRAY_VAR..], bs, &mut space)?,
            mode: sb.index_mode,
            threshold: f64::from_bits(sb.index_threshold),
            hysteresis: f64::from_bits(sb.index_hysteresis),
        };
        let inodes = Inodes::from_array(load(dev, ARRAY_INODES, inode::RECORD_SIZE, &mut space)?);
        for (slot, inode) in inodes.each() {
            if inode.blocks > 0 && !space.mark_used(inode.start, inode.blocks) {
                return Err(Error::corrupt(format!(
                    "the extent of inode {} overlaps another owner",
                    slot + 1
                )));
            }
        }
        let links = Links::from_parts(records, usage, sb.links)?;
        let mut fs = Self {
            device,
            sb,
            journal,
            space,
            links,
            inodes,
            config: RuntimeConfig::default(),
            cache: BTreeMap::new(),
            open: HashMap::new(),
            control: HashMap::new(),
            writable,
            dirty: false,
            journal_clean: false,
            ops: 0,
        };
        let mut config = fs.load_config()?;
        for (key, value) in overrides {
            config.set(key, value)?;
        }
        fs.config = config.clone();
        if writable {
            fs.apply_index(&config)?;
            fs.remove_orphans()?;
            if fs.dirty {
                fs.commit()?;
            }
        }
        Ok(fs)
    }

    /// Commits pending changes and returns the device.
    pub fn close(mut self) -> Result<D> {
        if self.writable {
            self.commit()?;
        }
        Ok(self.device)
    }

    /// Returns the device without committing (simulates a crash).
    pub fn into_device(self) -> D {
        self.device
    }

    /// The superblock as last committed (plus pending in-memory changes).
    #[must_use]
    pub const fn superblock(&self) -> &Superblock {
        &self.sb
    }

    /// The links database.
    #[must_use]
    pub const fn links(&self) -> &Links {
        &self.links
    }

    /// The free space map.
    #[must_use]
    pub const fn space(&self) -> &Space {
        &self.space
    }

    /// The runtime configuration.
    #[must_use]
    pub const fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Block size in bytes.
    #[must_use]
    pub const fn block_size(&self) -> usize {
        self.sb.block_size as usize
    }

    /// Name encoding of the volume.
    #[must_use]
    pub const fn name_encoding(&self) -> NameEncoding {
        self.sb.name_encoding
    }

    /// Whether the volume accepts changes.
    #[must_use]
    pub const fn is_writable(&self) -> bool {
        self.writable && !self.config.read_only
    }

    const fn check_writable(&self) -> Result<()> {
        if self.is_writable() {
            Ok(())
        } else {
            Err(Error::ReadOnly)
        }
    }

    /// Records a modification and commits when `commit-every` is reached.
    fn modified(&mut self) -> Result<()> {
        self.dirty = true;
        self.ops += 1;
        if self.config.commit_every > 0 && self.ops >= self.config.commit_every {
            self.commit()?;
        }
        Ok(())
    }

    /// Volume statistics.
    #[must_use]
    pub fn statfs(&self) -> StatFs {
        StatFs {
            block_size: self.sb.block_size,
            blocks: self.sb.total_blocks,
            free: self.space.free_blocks(),
            largest_free: self.space.largest_free(),
            files: self.inodes.used(),
            links: self.links.count(),
        }
    }

    // ----- configuration -----

    fn load_config(&self) -> Result<RuntimeConfig> {
        let mut config = RuntimeConfig::default();
        if self.sb.config != 0 {
            let text = names::read(
                &self.links,
                self.links.target(self.sb.config),
                self.sb.name_encoding,
            )?;
            config.apply_lino(&text)?;
        }
        Ok(config)
    }

    fn store_config(&mut self, config: &RuntimeConfig) -> Result<()> {
        let text = names::intern(&mut self.links, &config.to_lino(), self.sb.name_encoding)?;
        let link = self.links.get_or_create(CONFIG, text)?;
        let old = self.sb.config;
        self.sb.config = link;
        if old != 0 && old != link {
            let old_text = self.links.target(old);
            self.links.delete(old)?;
            names::release(&mut self.links, old_text)?;
        }
        Ok(())
    }

    fn apply_index(&mut self, config: &RuntimeConfig) -> Result<()> {
        let threshold = config.bitstring_threshold.fraction();
        let hysteresis = f64::from(config.hysteresis_percent) / 100.0;
        let usage = &self.links.usage;
        let unchanged = usage.mode == config.index
            && (usage.threshold - threshold).abs() < f64::EPSILON
            && (usage.hysteresis - hysteresis).abs() < f64::EPSILON;
        if !unchanged {
            self.links
                .set_index(config.index, threshold, hysteresis, &mut self.space)?;
            self.sb.index_mode = config.index;
            self.sb.index_threshold = threshold.to_bits();
            self.sb.index_hysteresis = hysteresis.to_bits();
            self.dirty = true;
        }
        Ok(())
    }

    /// Replaces the runtime configuration and persists it in the volume.
    pub fn set_config(&mut self, config: RuntimeConfig) -> Result<()> {
        if !self.writable {
            return Err(Error::ReadOnly);
        }
        let leaving_full = self.config.journal == JournalMode::Full
            && config.journal != JournalMode::Full
            && !self.cache.is_empty();
        self.apply_index(&config)?;
        self.store_config(&config)?;
        self.config = config;
        self.dirty = true;
        if leaving_full {
            self.commit()?;
        }
        Ok(())
    }

    /// Changes one configuration key.
    pub fn set_config_value(&mut self, key: &str, value: &str) -> Result<()> {
        let mut config = self.config.clone();
        config.set(key, value)?;
        self.set_config(config)
    }

    /// Applies links notation configuration text.
    pub fn apply_config_text(&mut self, text: &str) -> Result<()> {
        let mut config = self.config.clone();
        config.apply_lino(text)?;
        self.set_config(config)
    }

    // ----- inode helpers -----

    fn slot_of(ino: u64) -> Result<u64> {
        ino.checked_sub(1).ok_or(Error::NotFound)
    }

    fn inode(&self, ino: u64) -> Result<Inode> {
        let inode = self.inodes.get(Self::slot_of(ino)?);
        if inode.is_used() {
            Ok(inode)
        } else {
            Err(Error::NotFound)
        }
    }

    const fn type_point(kind: FileKind) -> u64 {
        match kind {
            FileKind::File => FILE,
            FileKind::Dir => DIR,
            FileKind::Symlink => SYMLINK,
        }
    }

    /// The link `(TYPE, $slot)` of inode `ino`.
    pub fn inode_link(&self, ino: u64) -> Result<u64> {
        let inode = self.inode(ino)?;
        let kind = inode
            .kind()
            .ok_or_else(|| Error::corrupt(format!("inode {ino} has an unknown type")))?;
        self.links
            .search(Self::type_point(kind), external(Self::slot_of(ino)?))
            .ok_or_else(|| Error::corrupt(format!("inode {ino} has no link")))
    }

    /// The inode number an inode link stands for.
    #[must_use]
    pub fn ino_of_link(&self, link: u64) -> Option<u64> {
        if !self.links.exists(link) {
            return None;
        }
        let source = self.links.source(link);
        if !(FILE..=SYMLINK).contains(&source) {
            return None;
        }
        crate::links::as_external(self.links.target(link)).map(|slot| slot + 1)
    }

    /// Attributes of `ino` (virtual control files included).
    pub fn getattr(&self, ino: u64) -> Result<Attr> {
        if control::is_control(ino) {
            return self.control_attr(ino);
        }
        let inode = self.inode(ino)?;
        let kind = inode
            .kind()
            .ok_or_else(|| Error::corrupt(format!("inode {ino} has an unknown type")))?;
        let size = if kind == FileKind::Dir {
            self.links
                .count_role(crate::links::records::Role::Source, self.inode_link(ino)?)
        } else {
            inode.size
        };
        Ok(Attr {
            ino,
            kind,
            perm: (inode.mode & 0o7777) as u16,
            nlink: inode.nlink,
            uid: inode.uid,
            gid: inode.gid,
            size,
            allocated: inode.blocks * self.block_size() as u64,
            atime: inode.atime,
            mtime: inode.mtime,
            ctime: inode.ctime,
        })
    }

    /// Changes attributes.
    pub fn setattr(&mut self, ino: u64, changes: SetAttr) -> Result<Attr> {
        if control::is_control(ino) {
            if changes.size == Some(0) {
                self.control.insert(ino, Vec::new());
            }
            return self.control_attr(ino);
        }
        self.check_writable()?;
        if let Some(size) = changes.size {
            self.truncate(ino, size)?;
        }
        let slot = Self::slot_of(ino)?;
        let mut inode = self.inode(ino)?;
        if let Some(perm) = changes.perm {
            inode.mode = (inode.mode & inode::S_IFMT) | u32::from(perm & 0o7777);
        }
        if let Some(uid) = changes.uid {
            inode.uid = uid;
        }
        if let Some(gid) = changes.gid {
            inode.gid = gid;
        }
        if let Some(atime) = changes.atime {
            inode.atime = atime;
        }
        if let Some(mtime) = changes.mtime {
            inode.mtime = mtime;
        }
        inode.ctime = now();
        self.inodes.set(slot, &inode);
        self.modified()?;
        self.getattr(ino)
    }

    /// Notes that `ino` was opened.
    pub fn open_file(&mut self, ino: u64) -> Result<()> {
        if !control::is_control(ino) {
            self.inode(ino)?;
            *self.open.entry(ino).or_insert(0) += 1;
        }
        Ok(())
    }

    /// Notes that a handle of `ino` was closed: trims growth slack, frees
    /// unlinked files and applies control file writes.
    pub fn release(&mut self, ino: u64) -> Result<()> {
        if control::is_control(ino) {
            return self.control_flush(ino);
        }
        let remaining = match self.open.get_mut(&ino) {
            Some(count) if *count > 1 => {
                *count -= 1;
                *count
            }
            _ => {
                self.open.remove(&ino);
                0
            }
        };
        if remaining == 0 && self.writable {
            let inode = self.inode(ino)?;
            if inode.nlink == 0 {
                self.destroy(ino)?;
                self.modified()?;
            } else if self.trim(ino)? {
                self.modified()?;
            }
        }
        Ok(())
    }

    fn remove_orphans(&mut self) -> Result<()> {
        let orphans: Vec<u64> = self
            .inodes
            .each()
            .filter(|(_, inode)| inode.nlink == 0)
            .map(|(slot, _)| slot + 1)
            .collect();
        for ino in orphans {
            self.destroy(ino)?;
            self.dirty = true;
        }
        Ok(())
    }
}
