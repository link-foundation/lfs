//! The embedded universal links database.
//!
//! A doublets store (every link is `index: source -> target`) shared by the
//! file system and by users through the CLI and the `/.lfs` directory. The
//! file system's own structures — inodes, names, directory entries — are
//! ordinary links of this store, so one journal covers both.
//!
//! Values that are not link indices are stored as *external references*
//! using the `Platform.Data.Hybrid` encoding of binary links notation: the
//! external value `v` is the raw cell `2^64 - v` (and external zero is
//! `2^63`), so internal indices are `1..2^63`.

pub mod packet;
pub mod records;
mod slots;
pub mod usage;

use crate::config::{IndexMode, Layout};
use crate::error::{Error, Result};
use crate::sbt::{self, NodesRead};
use crate::space::Space;
use links_notation::binary::packet::{decode_external, encode_external};
use records::{header, Records, Role, Tree, TreeId, TreeMut};
use std::collections::BTreeSet;
use usage::UsageIndex;

/// The raw cell of external zero.
pub const EXTERNAL_ZERO: u64 = 1 << 63;

/// Raw cell of the external value `value` (`value < 2^63`).
#[must_use]
pub fn external(value: u64) -> u64 {
    debug_assert!(value < EXTERNAL_ZERO);
    encode_external(value, 8)
}

/// External value of a raw cell, if it is an external reference.
#[must_use]
pub fn as_external(raw: u64) -> Option<u64> {
    decode_external(raw, 8)
}

/// Whether a raw cell is an external reference.
#[must_use]
pub const fn is_external(raw: u64) -> bool {
    raw >= EXTERNAL_ZERO
}

/// Whether a raw cell is a (non-null) link index.
#[must_use]
pub const fn is_internal(raw: u64) -> bool {
    raw != 0 && raw < EXTERNAL_ZERO
}

/// Formats a raw cell: `12` for links, `$7` for externals.
#[must_use]
pub fn format_ref(raw: u64) -> String {
    as_external(raw).map_or_else(|| raw.to_string(), |value| format!("${value}"))
}

/// Parses `12` or `$7` into a raw cell.
pub fn parse_ref(text: &str) -> Result<u64> {
    let text = text.trim();
    let (value, ext) = text
        .strip_prefix('$')
        .map_or((text, false), |rest| (rest, true));
    let value: u64 = value
        .parse()
        .map_err(|_| Error::invalid(format!("bad reference {text:?}")))?;
    if value >= EXTERNAL_ZERO {
        return Err(Error::invalid(format!("reference {text} is out of range")));
    }
    Ok(if ext { external(value) } else { value })
}

/// The links header (doublets `LinksHeader` without `reserved`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    /// Highest allocated link index.
    pub allocated: u64,
    /// Number of free slots below `allocated`.
    pub free: u64,
    /// First slot of the free list (united layout).
    pub first_free: u64,
    /// Root of the global sources tree.
    pub root_as_source: u64,
    /// Root of the global targets tree.
    pub root_as_target: u64,
    /// Last slot of the free list (united layout).
    pub last_free: u64,
}

impl Header {
    /// Serialized size.
    pub const SIZE: usize = 48;

    /// Writes the header.
    pub fn write(&self, bytes: &mut [u8]) {
        let cells = [
            self.allocated,
            self.free,
            self.first_free,
            self.root_as_source,
            self.root_as_target,
            self.last_free,
        ];
        for (index, cell) in cells.iter().enumerate() {
            crate::codec::put_u64(bytes, index * 8, *cell);
        }
    }

    /// Reads a header.
    #[must_use]
    pub fn read(bytes: &[u8]) -> Self {
        let cell = |index: usize| crate::codec::get_u64(bytes, index * 8);
        Self {
            allocated: cell(0),
            free: cell(1),
            first_free: cell(2),
            root_as_source: cell(3),
            root_as_target: cell(4),
            last_free: cell(5),
        }
    }
}

/// One link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    /// Index.
    pub index: u64,
    /// Source cell.
    pub source: u64,
    /// Target cell.
    pub target: u64,
}

/// The links store.
#[derive(Clone, Debug)]
pub struct Links {
    /// Records with the embedded index.
    pub records: Records,
    /// Optional variable-size usage index.
    pub usage: UsageIndex,
    header: Header,
    free_set: BTreeSet<u64>,
    ram: [BTreeSet<(u64, u64, u64)>; 2],
}

const fn role_index(role: Role) -> usize {
    match role {
        Role::Source => 0,
        Role::Target => 1,
    }
}

const ROLES: [Role; 2] = [Role::Source, Role::Target];
const MAX_KEY: (u64, u64, u64) = (u64::MAX, u64::MAX, u64::MAX);

impl Links {
    /// An empty store.
    #[must_use]
    pub fn new(layout: Layout, block_size: usize) -> Self {
        let mut links = Self {
            records: Records::new(layout, block_size),
            usage: UsageIndex::new(block_size),
            header: Header::default(),
            free_set: BTreeSet::new(),
            ram: [BTreeSet::new(), BTreeSet::new()],
        };
        links.sync_header();
        links
    }

    /// Rebuilds the in-memory state of loaded records.
    pub fn from_parts(records: Records, usage: UsageIndex, header: Header) -> Result<Self> {
        let mut links = Self {
            records,
            usage,
            header,
            free_set: BTreeSet::new(),
            ram: [BTreeSet::new(), BTreeSet::new()],
        };
        if header.allocated >= links.records.capacity().max(1) && header.allocated != 0 {
            return Err(Error::corrupt("links header exceeds the records"));
        }
        if links.records.layout != Layout::United {
            for index in 1..=header.allocated {
                if links.records.source(index) == 0 && links.records.target(index) == 0 {
                    links.free_set.insert(index);
                }
            }
        }
        links.rebuild_ram();
        Ok(links)
    }

    fn rebuild_ram(&mut self) {
        self.ram = [BTreeSet::new(), BTreeSet::new()];
        if self.records.layout != Layout::DataOnly {
            return;
        }
        for index in 1..=self.header.allocated {
            if self.exists(index) {
                self.ram_update(index, true);
            }
        }
    }

    fn ram_wants(&self, end: u64) -> bool {
        self.records.layout == Layout::DataOnly && (!self.usage.enabled() || !is_internal(end))
    }

    fn ram_update(&mut self, index: u64, insert: bool) {
        for role in ROLES {
            let end = self.records.end(index, role);
            if self.ram_wants(end) {
                let key = (end, self.records.other_end(index, role), index);
                let set = &mut self.ram[role_index(role)];
                if insert {
                    set.insert(key);
                } else {
                    set.remove(&key);
                }
            }
        }
    }

    /// The layout.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.records.layout
    }

    /// The header.
    #[must_use]
    pub const fn header(&self) -> Header {
        self.header
    }

    /// Highest allocated index.
    #[must_use]
    pub const fn allocated(&self) -> u64 {
        self.header.allocated
    }

    /// Number of live links.
    #[must_use]
    pub const fn count(&self) -> u64 {
        self.header.allocated - self.header.free
    }

    /// Mirrors the header into the records (doublets-compatible header).
    pub fn sync_header(&mut self) {
        let h = self.header;
        if self.records.layout == Layout::DataOnly {
            return;
        }
        self.records.ensure(h.allocated);
        let reserved = self.records.capacity().saturating_sub(1);
        for (cell, value) in [
            (header::ALLOCATED, h.allocated),
            (header::RESERVED, reserved),
            (header::FREE, h.free),
            (header::FIRST_FREE, h.first_free),
            (header::ROOT_AS_SOURCE, h.root_as_source),
            (header::ROOT_AS_TARGET, h.root_as_target),
            (header::LAST_FREE, h.last_free),
        ] {
            self.records.set_header(cell, value);
        }
    }

    fn united_unused(&self, index: u64) -> bool {
        if index <= self.header.allocated && self.header.first_free != index {
            self.records.tree_cell(index, Role::Source, 2) == 0 && self.records.source(index) != 0
        } else {
            true
        }
    }

    /// Whether `index` is a live link.
    #[must_use]
    pub fn exists(&self, index: u64) -> bool {
        if index == 0 || index > self.header.allocated {
            return false;
        }
        match self.records.layout {
            Layout::United => !self.united_unused(index),
            _ => self.records.source(index) != 0 || self.records.target(index) != 0,
        }
    }

    /// Reads a live link.
    pub fn get(&self, index: u64) -> Result<Link> {
        if !self.exists(index) {
            return Err(Error::NotFound);
        }
        Ok(Link {
            index,
            source: self.records.source(index),
            target: self.records.target(index),
        })
    }

    /// Source of a live link (0 if missing).
    #[must_use]
    pub fn source(&self, index: u64) -> u64 {
        if self.exists(index) {
            self.records.source(index)
        } else {
            0
        }
    }

    /// Target of a live link (0 if missing).
    #[must_use]
    pub fn target(&self, index: u64) -> u64 {
        if self.exists(index) {
            self.records.target(index)
        } else {
            0
        }
    }

    fn check_ref(&self, raw: u64, own: u64) -> Result<()> {
        if is_internal(raw) && raw != own && !self.exists(raw) {
            return Err(Error::invalid(format!("link {raw} does not exist")));
        }
        Ok(())
    }

    /// Creates `source -> target`; fails with [`Error::Exists`] for duplicates.
    pub fn create(&mut self, source: u64, target: u64) -> Result<u64> {
        if source == 0 && target == 0 {
            return Err(Error::invalid("a link needs a non-null end"));
        }
        self.check_ref(source, 0)?;
        self.check_ref(target, 0)?;
        if self.search(source, target).is_some() {
            return Err(Error::Exists);
        }
        let index = self.allocate_slot();
        self.attach(index, source, target)?;
        self.sync_header();
        Ok(index)
    }

    /// Creates a point (a link whose ends are itself).
    pub fn create_point(&mut self) -> Result<u64> {
        let index = self.allocate_slot();
        self.attach(index, index, index)?;
        self.sync_header();
        Ok(index)
    }

    /// Finds `source -> target` or creates it.
    pub fn get_or_create(&mut self, source: u64, target: u64) -> Result<u64> {
        self.search(source, target)
            .map_or_else(|| self.create(source, target), Ok)
    }

    /// Changes both ends of a live link.
    pub fn update(&mut self, index: u64, source: u64, target: u64) -> Result<()> {
        let old = self.get(index)?;
        if (old.source, old.target) == (source, target) {
            return Ok(());
        }
        if source == 0 && target == 0 {
            return Err(Error::invalid("a link needs a non-null end"));
        }
        self.check_ref(source, index)?;
        self.check_ref(target, index)?;
        if self.search(source, target).is_some() {
            return Err(Error::Exists);
        }
        self.detach(index)?;
        self.attach(index, source, target)?;
        self.sync_header();
        Ok(())
    }

    /// Number of other links that refer to `index`.
    #[must_use]
    pub fn references(&self, index: u64) -> u64 {
        let mut count = self.count_role(Role::Source, index) + self.count_role(Role::Target, index);
        if self.exists(index) {
            count -= u64::from(self.records.source(index) == index)
                + u64::from(self.records.target(index) == index);
        }
        count
    }

    /// Deletes a link that no other link refers to.
    pub fn delete(&mut self, index: u64) -> Result<()> {
        self.get(index)?;
        let refs = self.references(index);
        if refs > 0 {
            return Err(Error::Busy(format!(
                "link {index} is referenced by {refs} links"
            )));
        }
        self.detach(index)?;
        self.free_slot(index);
        self.sync_header();
        Ok(())
    }

    fn tree_root(&self, tree: TreeId, role: Role) -> u64 {
        match (tree, role) {
            (TreeId::Global, Role::Source) => self.header.root_as_source,
            (TreeId::Global, Role::Target) => self.header.root_as_target,
            (TreeId::PerLink(link), role) => self.records.link_root(link, role),
        }
    }

    fn set_tree_root(&mut self, tree: TreeId, role: Role, root: u64) {
        match (tree, role) {
            (TreeId::Global, Role::Source) => self.header.root_as_source = root,
            (TreeId::Global, Role::Target) => self.header.root_as_target = root,
            (TreeId::PerLink(link), role) => self.records.set_link_root(link, role, root),
        }
    }

    fn attach(&mut self, index: u64, source: u64, target: u64) -> Result<()> {
        self.records.set_link(index, source, target);
        for role in ROLES {
            if let Some(tree) = self.records.tree_of(index, role) {
                let root = self.tree_root(tree, role);
                let per_link = matches!(tree, TreeId::PerLink(_));
                let mut view = TreeMut {
                    records: &mut self.records,
                    role,
                    per_link,
                };
                let root = sbt::insert(&mut view, root, index);
                self.set_tree_root(tree, role, root);
            }
            self.usage
                .add(&self.records, index, role, self.header.allocated)?;
        }
        self.ram_update(index, true);
        Ok(())
    }

    fn detach(&mut self, index: u64) -> Result<()> {
        self.ram_update(index, false);
        for role in ROLES {
            self.usage
                .remove(&self.records, index, role, self.header.allocated)?;
            if let Some(tree) = self.records.tree_of(index, role) {
                let root = self.tree_root(tree, role);
                let per_link = matches!(tree, TreeId::PerLink(_));
                let mut view = TreeMut {
                    records: &mut self.records,
                    role,
                    per_link,
                };
                let key = view.key(index);
                let root = sbt::remove(&mut view, root, key);
                self.set_tree_root(tree, role, root);
            }
        }
        self.records.set_link(index, 0, 0);
        Ok(())
    }

    fn view(&self, tree: TreeId, role: Role) -> (Tree<'_>, u64) {
        let per_link = matches!(tree, TreeId::PerLink(_));
        (
            Tree {
                records: &self.records,
                role,
                per_link,
            },
            self.tree_root(tree, role),
        )
    }

    /// The tree holding links whose `role` end is `end`.
    fn tree_for(&self, role: Role, end: u64) -> Option<TreeId> {
        match self.records.layout {
            Layout::Split if is_internal(end) => {
                if self.exists(end) {
                    Some(TreeId::PerLink(end))
                } else {
                    None
                }
            }
            Layout::United | Layout::Split if end != 0 => Some(TreeId::Global),
            _ => {
                let _ = role;
                None
            }
        }
    }

    fn uses_usage(&self, end: u64) -> bool {
        self.records.layout == Layout::DataOnly && self.usage.enabled() && is_internal(end)
    }

    /// Finds the link `source -> target`.
    #[must_use]
    pub fn search(&self, source: u64, target: u64) -> Option<u64> {
        if self.records.layout == Layout::DataOnly {
            if self.uses_usage(source) {
                self.usage.find(&self.records, source, Role::Source, target)
            } else if self.uses_usage(target) {
                self.usage.find(&self.records, target, Role::Target, source)
            } else {
                let low = (source, target, 0);
                self.ram[0]
                    .range(low..=(source, target, u64::MAX))
                    .next()
                    .map(|key| key.2)
            }
        } else {
            let (role, end, other) = if source != 0 && (is_internal(source) || !is_internal(target))
            {
                (Role::Source, source, target)
            } else {
                (Role::Target, target, source)
            };
            if is_internal(end) && !self.exists(end) {
                return None;
            }
            let tree = self.tree_for(role, end)?;
            let (view, root) = self.view(tree, role);
            match tree {
                TreeId::Global => sbt::find(&view, root, end, other),
                TreeId::PerLink(_) => {
                    let mut found = None;
                    sbt::each_in_range(
                        &view,
                        root,
                        (other, 0, 0),
                        (other, u64::MAX, u64::MAX),
                        &mut |node| {
                            found = Some(node);
                            false
                        },
                    );
                    found
                }
            }
        }
    }

    /// Visits the links whose `role` end is `end`; returns `false` if stopped.
    pub fn each_role(&self, role: Role, end: u64, visit: &mut dyn FnMut(u64) -> bool) -> bool {
        if self.uses_usage(end) {
            return self.usage.each(&self.records, end, role, visit);
        }
        if self.records.layout == Layout::DataOnly {
            let set = &self.ram[role_index(role)];
            return set
                .range((end, 0, 0)..=(end, u64::MAX, u64::MAX))
                .all(|key| visit(key.2));
        }
        match self.tree_for(role, end) {
            Some(tree) => {
                let (view, root) = self.view(tree, role);
                let (low, high) = match tree {
                    TreeId::Global => ((end, 0, 0), (end, u64::MAX, u64::MAX)),
                    TreeId::PerLink(_) => ((0, 0, 0), MAX_KEY),
                };
                sbt::each_in_range(&view, root, low, high, visit)
            }
            None if end == 0 => (1..=self.header.allocated)
                .filter(|&i| self.exists(i) && self.records.end(i, role) == 0)
                .all(visit),
            None => true,
        }
    }

    /// Visits links with source `source`.
    pub fn each_source(&self, source: u64, visit: &mut dyn FnMut(u64) -> bool) -> bool {
        self.each_role(Role::Source, source, visit)
    }

    /// Visits links with target `target`.
    pub fn each_target(&self, target: u64, visit: &mut dyn FnMut(u64) -> bool) -> bool {
        self.each_role(Role::Target, target, visit)
    }

    /// Collects links whose `role` end is `end`.
    #[must_use]
    pub fn users(&self, role: Role, end: u64) -> Vec<u64> {
        let mut out = Vec::new();
        self.each_role(role, end, &mut |link| {
            out.push(link);
            true
        });
        out
    }

    /// Number of links whose `role` end is `end`.
    #[must_use]
    pub fn count_role(&self, role: Role, end: u64) -> u64 {
        if self.uses_usage(end) {
            return self.usage.count(end, role);
        }
        match self.tree_for(role, end) {
            Some(tree @ TreeId::Global) => {
                let (view, root) = self.view(tree, role);
                sbt::count_primary(&view, root, end)
            }
            Some(tree) => {
                let (view, root) = self.view(tree, role);
                sbt::size(&view, root)
            }
            None => {
                let mut count = 0;
                self.each_role(role, end, &mut |_| {
                    count += 1;
                    true
                });
                count
            }
        }
    }

    /// Visits every live link in index order; returns `false` if stopped.
    pub fn each_link(&self, visit: &mut dyn FnMut(Link) -> bool) -> bool {
        for index in 1..=self.header.allocated {
            if self.exists(index)
                && !visit(Link {
                    index,
                    source: self.records.source(index),
                    target: self.records.target(index),
                })
            {
                return false;
            }
        }
        true
    }

    /// All live links.
    #[must_use]
    pub fn all(&self) -> Vec<Link> {
        let mut out = Vec::with_capacity(self.count() as usize);
        self.each_link(&mut |link| {
            out.push(link);
            true
        });
        out
    }

    /// Switches the optional usage index and rebuilds it.
    pub fn set_index(
        &mut self,
        mode: IndexMode,
        threshold: f64,
        hysteresis: f64,
        space: &mut Space,
    ) -> Result<()> {
        let unchanged = self.usage.mode == mode
            && (self.usage.threshold - threshold).abs() < f64::EPSILON
            && (self.usage.hysteresis - hysteresis).abs() < f64::EPSILON;
        if unchanged {
            return Ok(());
        }
        self.usage.var.clear(space);
        self.usage.handles.truncate_chunks(0, space);
        self.usage.mode = mode;
        self.usage.threshold = threshold;
        self.usage.hysteresis = hysteresis;
        if self.usage.enabled() {
            for index in 1..=self.header.allocated {
                if self.exists(index) {
                    for role in ROLES {
                        self.usage
                            .add(&self.records, index, role, self.header.allocated)?;
                    }
                }
            }
        }
        self.rebuild_ram();
        Ok(())
    }

    /// Releases record chunks above `allocated`.
    pub fn shrink(&mut self, space: &mut Space) {
        let keep = self.header.allocated + 1;
        self.records.data.truncate_chunks(keep, space);
        if self.records.layout == Layout::Split {
            self.records.index.truncate_chunks(keep, space);
        }
        self.usage.handles.truncate_chunks(keep, space);
        self.usage.var.shrink(space);
        self.sync_header();
    }

    /// Verifies every invariant; returns a list of problems.
    #[must_use]
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut live = 0;
        for link in self.all() {
            live += 1;
            for (raw, what) in [(link.source, "source"), (link.target, "target")] {
                if is_internal(raw) && !self.exists(raw) {
                    problems.push(format!("link {} has a dangling {what} {raw}", link.index));
                }
            }
            if self.search(link.source, link.target) != Some(link.index) {
                problems.push(format!("link {} is not found by its ends", link.index));
            }
        }
        if live != self.count() {
            problems.push(format!(
                "header counts {} links but {live} are live",
                self.count()
            ));
        }
        match self.records.layout {
            Layout::United => {
                for role in ROLES {
                    let (view, root) = self.view(TreeId::Global, role);
                    if let Err(error) = sbt::check(&view, root, None, None) {
                        problems.push(format!("{role:?} tree: {error}"));
                    }
                }
            }
            Layout::Split => {
                for role in ROLES {
                    let (view, root) = self.view(TreeId::Global, role);
                    if let Err(error) = sbt::check(&view, root, None, None) {
                        problems.push(format!("external {role:?} tree: {error}"));
                    }
                    for link in self.all() {
                        let (view, root) = self.view(TreeId::PerLink(link.index), role);
                        if let Err(error) = sbt::check(&view, root, None, None) {
                            problems.push(format!("{role:?} tree of {}: {error}", link.index));
                        }
                    }
                }
            }
            Layout::DataOnly => {}
        }
        if self.usage.enabled() {
            if let Err(error) = self.usage.check(&self.records, &|link| self.exists(link)) {
                problems.push(error);
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests;
