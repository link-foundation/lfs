//! The virtual `.lfs` control directory in the root of every mounted volume.
//!
//! * `.lfs/config` — the runtime configuration as links notation; writing
//!   `(key: value)` lines and closing the file applies them immediately,
//!   without remounting.
//! * `.lfs/map.json` — the disk space map read by the web visualizer.
//! * `.lfs/links` — every link of the embedded database as links notation.
//! * `.lfs/stats` — volume statistics.
//!
//! The entries are not stored on the device; their inode numbers sit at the
//! top of the inode number space where no real inode can reach.

use super::dir::DirEntry;
use super::inode::FileKind;
use super::{now, Attr, Fs, ROOT_INO};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use std::fmt::Write;

/// Inode number of the `.lfs` directory.
pub const CONTROL_DIR_INO: u64 = 0xFFFF_FFFF_FFFF_FF00;
/// Name of the control directory.
pub const CONTROL_DIR: &str = ".lfs";
/// Names of the control files (inode `CONTROL_DIR_INO + 1 + i`).
pub const CONTROL_FILES: [&str; 4] = ["config", "map.json", "links", "stats"];

const CONFIG_INO: u64 = CONTROL_DIR_INO + 1;

/// Whether `ino` is the control directory or one of its files.
#[must_use]
pub const fn is_control(ino: u64) -> bool {
    ino >= CONTROL_DIR_INO && ino <= CONTROL_DIR_INO + CONTROL_FILES.len() as u64
}

/// Looks a control entry up.
#[must_use]
pub fn lookup(parent: u64, name: &str) -> Option<u64> {
    if parent == ROOT_INO && name == CONTROL_DIR {
        return Some(CONTROL_DIR_INO);
    }
    if parent == CONTROL_DIR_INO {
        return CONTROL_FILES
            .iter()
            .position(|file| *file == name)
            .map(|index| CONTROL_DIR_INO + 1 + index as u64);
    }
    None
}

/// The entries of the control directory (`None` for other inodes).
#[must_use]
pub fn readdir(ino: u64) -> Option<Vec<DirEntry>> {
    (ino == CONTROL_DIR_INO).then(|| {
        CONTROL_FILES
            .iter()
            .enumerate()
            .map(|(index, name)| DirEntry {
                name: (*name).to_string(),
                ino: CONTROL_DIR_INO + 1 + index as u64,
                kind: FileKind::File,
            })
            .collect()
    })
}

/// The root directory entry of `.lfs`.
#[must_use]
pub fn root_entry() -> DirEntry {
    DirEntry {
        name: CONTROL_DIR.to_string(),
        ino: CONTROL_DIR_INO,
        kind: FileKind::Dir,
    }
}

/// Parent of a control inode.
#[must_use]
pub const fn parent(ino: u64) -> u64 {
    if ino == CONTROL_DIR_INO {
        ROOT_INO
    } else {
        CONTROL_DIR_INO
    }
}

impl<D: BlockDevice> Fs<D> {
    /// The current content of a control file.
    fn control_content(&self, ino: u64) -> Result<Vec<u8>> {
        if let Some(pending) = self.control.get(&ino) {
            return Ok(pending.clone());
        }
        let text = match ino - CONTROL_DIR_INO {
            1 => self.config.to_lino(),
            2 => self.map_json(),
            3 => self.links_text(),
            4 => self.stats_text(),
            _ => return Err(Error::IsDir),
        };
        Ok(text.into_bytes())
    }

    /// Every link as `(index: source target)`, one per line.
    #[must_use]
    pub fn links_text(&self) -> String {
        crate::links::packet::write_lino(&self.links.all())
    }

    /// Volume statistics as links notation.
    #[must_use]
    pub fn stats_text(&self) -> String {
        let stats = self.statfs();
        let (var_used, var_capacity) = self.links.usage.var.usage();
        let mut out = String::new();
        let _ = writeln!(out, "(label: \"{}\")", self.sb.label.replace('"', "'"));
        let _ = writeln!(out, "(block-size: {})", stats.block_size);
        let _ = writeln!(out, "(blocks: {})", stats.blocks);
        let _ = writeln!(out, "(free-blocks: {})", stats.free);
        let _ = writeln!(out, "(largest-free: {})", stats.largest_free);
        let _ = writeln!(out, "(free-extents: {})", self.space.free_extents().count());
        let _ = writeln!(out, "(pending-blocks: {})", self.space.pending_blocks());
        let _ = writeln!(out, "(files: {})", stats.files);
        let _ = writeln!(out, "(links: {})", stats.links);
        let _ = writeln!(out, "(layout: {})", self.sb.layout.name());
        let _ = writeln!(out, "(name-encoding: {})", self.sb.name_encoding.name());
        let _ = writeln!(out, "(journal-blocks: {})", self.sb.journal_blocks);
        let _ = writeln!(out, "(seq: {})", self.sb.seq);
        let _ = writeln!(out, "(index-bytes: {var_used})");
        let _ = writeln!(out, "(index-capacity: {var_capacity})");
        out
    }

    pub(super) fn control_attr(&self, ino: u64) -> Result<Attr> {
        let time = now();
        let (kind, perm, size) = if ino == CONTROL_DIR_INO {
            (FileKind::Dir, 0o555, CONTROL_FILES.len() as u64)
        } else {
            let perm = if ino == CONFIG_INO && self.is_writable() {
                0o644
            } else {
                0o444
            };
            (
                FileKind::File,
                perm,
                self.control_content(ino)?.len() as u64,
            )
        };
        // Whoever owns the volume's root may configure it.
        let owner = self.inode(ROOT_INO)?;
        Ok(Attr {
            ino,
            kind,
            perm,
            nlink: if kind == FileKind::Dir { 2 } else { 1 },
            uid: owner.uid,
            gid: owner.gid,
            size,
            allocated: 0,
            atime: time,
            mtime: time,
            ctime: time,
        })
    }

    pub(super) fn control_read(&self, ino: u64, offset: u64, size: usize) -> Result<Vec<u8>> {
        let content = self.control_content(ino)?;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(content.len());
        let end = start.saturating_add(size).min(content.len());
        Ok(content[start..end].to_vec())
    }

    pub(super) fn control_write(&mut self, ino: u64, offset: u64, data: &[u8]) -> Result<usize> {
        if ino != CONFIG_INO {
            return Err(if ino == CONTROL_DIR_INO {
                Error::IsDir
            } else {
                Error::ReadOnly
            });
        }
        self.check_writable()?;
        let offset = usize::try_from(offset).map_err(|_| Error::invalid("offset too large"))?;
        if offset.saturating_add(data.len()) > 1 << 20 {
            return Err(Error::invalid("configuration text too large"));
        }
        let buffer = self.control.entry(ino).or_default();
        if buffer.len() < offset + data.len() {
            buffer.resize(offset + data.len(), 0);
        }
        buffer[offset..offset + data.len()].copy_from_slice(data);
        Ok(data.len())
    }

    pub(super) fn control_flush(&mut self, ino: u64) -> Result<()> {
        let Some(buffer) = self.control.remove(&ino) else {
            return Ok(());
        };
        let text =
            String::from_utf8(buffer).map_err(|_| Error::invalid("configuration is not UTF-8"))?;
        self.apply_config_text(&text)?;
        self.commit()
    }
}
