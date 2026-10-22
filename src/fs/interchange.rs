//! The embedded links database as seen by its users.
//!
//! Links are created, changed, deleted and queried next to the file
//! system, and the whole store is imported and exported (binary links
//! notation archives, doublets files, links notation text).
//!
//! User links share the store, the commits and the journal of the file
//! system. While `protect-structure` is on, the links the file system is
//! made of cannot be changed or deleted, user links cannot start at or
//! point to them (so the file system never mistakes a user link for an
//! entry or a binding), and a link other links refer to cannot be changed
//! (links are shared by content, so changing one would change every user).
//! The marker points `1..=5` stay usable as ends.

use super::{Fs, CONFIG, FILE, RESERVED_POINTS};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::links::packet::{self, Format};
use crate::links::{is_internal, Link};
use std::collections::{HashMap, HashSet};

/// Names of the points `1..=9` in exported archives.
pub const POINT_NAMES: [&str; RESERVED_POINTS as usize] = [
    "one",
    "number",
    "string",
    "list",
    "identified",
    "file",
    "dir",
    "symlink",
    "config",
];

/// Number of marker points (`ONE` … `IDENTIFIED`) shared with imports.
const MARKERS: u64 = 5;

impl<D: BlockDevice> Fs<D> {
    /// Whether `link` belongs to the file system: a reserved point, an
    /// inode or configuration link, an entry, a symbolic link target or a
    /// binding.
    #[must_use]
    pub fn is_structure(&self, link: u64) -> bool {
        if (1..=RESERVED_POINTS).contains(&link) {
            return true;
        }
        if !is_internal(link) || !self.links.exists(link) {
            return false;
        }
        let source = self.links.source(link);
        (FILE..=CONFIG).contains(&source)
            || self.ino_of_link(source).is_some()
            || (is_internal(source)
                && self.links.exists(source)
                && self.ino_of_link(self.links.source(source)).is_some())
    }

    fn check_end(&self, end: u64) -> Result<()> {
        if self.config.protect_structure && end > MARKERS && self.is_structure(end) {
            return Err(Error::invalid(format!(
                "link {end} belongs to the file system (protect-structure is on)"
            )));
        }
        Ok(())
    }

    fn check_mutable(&self, link: u64) -> Result<()> {
        self.links.get(link)?;
        if !self.config.protect_structure {
            return Ok(());
        }
        if self.is_structure(link) {
            return Err(Error::invalid(format!(
                "link {link} belongs to the file system (protect-structure is on)"
            )));
        }
        Ok(())
    }

    /// Creates the user link `source -> target` and returns its index.
    pub fn create_link(&mut self, source: u64, target: u64) -> Result<u64> {
        self.check_writable()?;
        self.check_end(source)?;
        self.check_end(target)?;
        let index = self.links.create(source, target)?;
        self.modified()?;
        Ok(index)
    }

    /// Creates a user point (a link whose ends are itself).
    pub fn create_point(&mut self) -> Result<u64> {
        self.check_writable()?;
        let index = self.links.create_point()?;
        self.modified()?;
        Ok(index)
    }

    /// Changes both ends of the user link `link`.
    pub fn update_link(&mut self, link: u64, source: u64, target: u64) -> Result<()> {
        self.check_writable()?;
        self.check_mutable(link)?;
        self.check_end(source)?;
        self.check_end(target)?;
        if self.config.protect_structure && self.links.references(link) > 0 {
            return Err(Error::Busy(format!(
                "link {link} is referenced by {} links",
                self.links.references(link)
            )));
        }
        self.links.update(link, source, target)?;
        self.modified()
    }

    /// Deletes the user link `link` (no other link may refer to it).
    pub fn delete_link(&mut self, link: u64) -> Result<()> {
        self.check_writable()?;
        self.check_mutable(link)?;
        self.links.delete(link)?;
        self.modified()
    }

    /// Every link matching the pattern (`None` matches anything), in
    /// address order.
    #[must_use]
    pub fn query_links(&self, source: Option<u64>, target: Option<u64>) -> Vec<Link> {
        let link = |index| self.links.get(index).ok();
        let mut found: Vec<Link> = match (source, target) {
            (Some(source), Some(target)) => self
                .links
                .search(source, target)
                .and_then(link)
                .into_iter()
                .collect(),
            (Some(source), None) => self
                .links
                .users(crate::links::records::Role::Source, source)
                .into_iter()
                .filter_map(link)
                .collect(),
            (None, Some(target)) => self
                .links
                .users(crate::links::records::Role::Target, target)
                .into_iter()
                .filter_map(link)
                .collect(),
            (None, None) => self.links.all(),
        };
        found.sort_unstable_by_key(|link| link.index);
        found
    }

    /// The whole links store in `format`; archives name the points
    /// `1..=9` (see [`POINT_NAMES`]).
    pub fn export_links(&self, format: Format) -> Result<Vec<u8>> {
        let names: Vec<(u64, String)> = (1..=RESERVED_POINTS)
            .zip(POINT_NAMES)
            .map(|(point, name)| (point, name.to_string()))
            .collect();
        packet::write(format, &self.links.all(), &names)
    }

    /// Imports foreign links (any format) under new addresses, reusing
    /// links that already exist, and commits. Points named `one` …
    /// `identified` in an archive become the volume's marker points; every
    /// other point, `file` and `dir` included, is created anew, so imported
    /// links never become part of the file system. Every link is checked
    /// before any is added, so a malformed input changes nothing. Returns the
    /// number of links added.
    pub fn import_links(&mut self, bytes: &[u8], format: Format) -> Result<u64> {
        self.check_writable()?;
        let (links, names) = match format {
            Format::Archive => {
                let archive = packet::read_archive(bytes)?;
                (archive.links, archive.names)
            }
            other => (packet::read(other, bytes)?, Vec::new()),
        };
        let points: HashSet<u64> = links
            .iter()
            .filter(|link| link.source == link.index && link.target == link.index)
            .map(|link| link.index)
            .collect();
        let mut taken = HashSet::new();
        let fixed: HashMap<u64, u64> = names
            .iter()
            .filter(|(address, _)| points.contains(address))
            .filter_map(|(address, name)| {
                let marker = POINT_NAMES[..MARKERS as usize]
                    .iter()
                    .position(|marker| marker == name)?;
                taken
                    .insert(marker)
                    .then_some((*address, marker as u64 + 1))
            })
            .collect();
        let before = self.links.count();
        packet::import_remapped(&mut self.links, &links, &fixed)?;
        let added = self.links.count() - before;
        if added > 0 {
            self.dirty = true;
            self.commit()?;
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests;
