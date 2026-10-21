//! `fsck`: verifies every invariant of a volume.

use super::inode::FileKind;
use super::map::{array_name, Owner};
use super::{names, Fs, FILE, RESERVED_POINTS, ROOT_INO, SYMLINK};
use crate::device::BlockDevice;
use crate::links::records::Role;
use crate::links::{as_external, is_internal};

fn describe(owner: Owner) -> String {
    match owner {
        Owner::Superblock => "the superblock".into(),
        Owner::Journal => "the journal".into(),
        Owner::Table { array } => format!("the {} chunk table", array_name(array)),
        Owner::Chunk { array, index } => format!("{} chunk {index}", array_name(array)),
        Owner::File { ino } => format!("inode {ino}"),
        Owner::Free => "free space".into(),
        Owner::Pending => "pending space".into(),
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Checks the volume; returns the list of problems (empty if clean).
    #[must_use]
    pub fn check(&self) -> Vec<String> {
        let mut problems: Vec<String> = self
            .links
            .check()
            .into_iter()
            .map(|problem| format!("links: {problem}"))
            .collect();
        self.check_space(&mut problems);
        self.check_tree(&mut problems);
        problems
    }

    /// Every block has one owner or is free (or pending since the last
    /// commit); files are one extent each.
    fn check_space(&self, problems: &mut Vec<String>) {
        let total = self.sb.total_blocks;
        let mut last: Option<(u64, u64, Owner)> = None;
        for (start, len, owner) in self.owners() {
            if len == 0 || start.checked_add(len).is_none_or(|end| end > total) {
                problems.push(format!(
                    "{} lies outside the volume ({start}+{len})",
                    describe(owner)
                ));
                continue;
            }
            if let Some((last_start, last_len, last_owner)) = last {
                if start < last_start + last_len {
                    problems.push(format!(
                        "{} and {} overlap at block {start}",
                        describe(last_owner),
                        describe(owner)
                    ));
                }
            }
            if (start..start + len).any(|block| !self.space.is_used(block)) {
                problems.push(format!("{} uses blocks marked free", describe(owner)));
            }
            if last.is_none_or(|(last_start, last_len, _)| start + len > last_start + last_len) {
                last = Some((start, len, owner));
            }
        }
        let owned: u64 = self.owners().iter().map(|(_, len, _)| len).sum();
        let used = total - self.space.free_blocks() - self.space.pending_blocks();
        if owned != used {
            problems.push(format!(
                "{used} blocks are in use but {owned} have an owner"
            ));
        }
    }

    /// Inodes, their links, names and link counts agree.
    fn check_tree(&self, problems: &mut Vec<String>) {
        if !self
            .inode(ROOT_INO)
            .is_ok_and(|inode| inode.kind() == Some(FileKind::Dir))
        {
            problems.push("the root inode is not a directory".into());
            return;
        }
        let block_size = self.block_size() as u64;
        let encoding = self.sb.name_encoding;
        for (slot, inode) in self.inodes.each() {
            let ino = slot + 1;
            let Some(kind) = inode.kind() else {
                problems.push(format!("inode {ino} has an unknown type"));
                continue;
            };
            // Symbolic link targets live in links, not in blocks.
            if kind != FileKind::Symlink && inode.size > inode.blocks * block_size {
                problems.push(format!(
                    "inode {ino} is {} bytes but has {} blocks",
                    inode.size, inode.blocks
                ));
            }
            let Ok(link) = self.inode_link(ino) else {
                problems.push(format!("inode {ino} has no link"));
                continue;
            };
            let bindings = self.links.users(Role::Target, link);
            if ino != ROOT_INO && bindings.is_empty() && inode.nlink > 0 {
                problems.push(format!("inode {ino} has no name"));
            }
            for &binding in &bindings {
                let entry = self.links.source(binding);
                let dir = self.links.source(entry);
                let text = self.links.target(entry);
                if !self.ino_of_link(dir).is_some_and(|parent| {
                    self.inode(parent)
                        .is_ok_and(|parent| parent.kind() == Some(FileKind::Dir))
                }) {
                    problems.push(format!("inode {ino} is named in a non-directory"));
                }
                if let Err(error) = names::read(&self.links, text, encoding) {
                    problems.push(format!("inode {ino} has a bad name: {error}"));
                }
            }
            match kind {
                FileKind::Dir => {
                    if bindings.len() > 1 {
                        problems.push(format!("directory {ino} has {} names", bindings.len()));
                    }
                    let mut subdirs = 0;
                    self.links.each_source(link, &mut |entry| {
                        let child = self.bound(entry).and_then(|bound| self.ino_of_link(bound));
                        if child.is_some_and(|child| {
                            self.inode(child)
                                .is_ok_and(|child| child.kind() == Some(FileKind::Dir))
                        }) {
                            subdirs += 1;
                        }
                        true
                    });
                    if inode.nlink != 2 + subdirs {
                        problems.push(format!(
                            "directory {ino} has nlink {} but {subdirs} subdirectories",
                            inode.nlink
                        ));
                    }
                }
                FileKind::File | FileKind::Symlink => {
                    let open = self.open.contains_key(&ino);
                    if u64::from(inode.nlink) != bindings.len() as u64 && !open {
                        problems.push(format!(
                            "inode {ino} has nlink {} but {} names",
                            inode.nlink,
                            bindings.len()
                        ));
                    }
                    if kind == FileKind::Symlink && self.readlink(ino).is_err() {
                        problems.push(format!("symbolic link {ino} has no target"));
                    }
                }
            }
        }
        // Every inode link names a used inode of its type.
        for point in FILE..=SYMLINK {
            self.links.each_source(point, &mut |link| {
                let target = self.links.target(link);
                match as_external(target) {
                    Some(slot) if self.inodes.get(slot).is_used() => {}
                    _ if is_internal(target) => {}
                    _ => problems.push(format!("link {link} names a free inode")),
                }
                true
            });
        }
        for point in 1..=RESERVED_POINTS {
            if !self.links.exists(point) || self.links.get(point).is_ok_and(|l| l.source != point) {
                problems.push(format!("reserved point {point} is missing"));
            }
        }
    }
}
