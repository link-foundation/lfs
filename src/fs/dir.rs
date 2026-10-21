//! Directories, names, hard and symbolic links.

use super::inode::{FileKind, Inode};
use super::{control, names, now, Attr, Fs, ROOT_INO};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::links::external;

/// One directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// Name.
    pub name: String,
    /// Inode number.
    pub ino: u64,
    /// Kind.
    pub kind: FileKind,
}

/// Who creates a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Creator {
    /// User id.
    pub uid: u32,
    /// Group id.
    pub gid: u32,
}

impl<D: BlockDevice> Fs<D> {
    fn dir_link(&self, ino: u64) -> Result<u64> {
        let inode = self.inode(ino)?;
        if inode.kind() != Some(FileKind::Dir) {
            return Err(Error::NotDir);
        }
        self.inode_link(ino)
    }

    /// The inode link an entry is bound to.
    pub(super) fn bound(&self, entry: u64) -> Option<u64> {
        let mut found = None;
        self.links.each_source(entry, &mut |binding| {
            found = Some(self.links.target(binding));
            false
        });
        found
    }

    /// Finds the entry link of `name` in directory link `dir`.
    fn entry(&self, dir: u64, name: &str) -> Option<u64> {
        let text = names::find(&self.links, name, self.sb.name_encoding)?;
        self.links.search(dir, text)
    }

    /// Looks a name up in a directory.
    pub fn lookup(&self, parent: u64, name: &str) -> Result<Attr> {
        if let Some(ino) = control::lookup(parent, name) {
            return self.getattr(ino);
        }
        let dir = self.dir_link(parent)?;
        let entry = self.entry(dir, name).ok_or(Error::NotFound)?;
        let child = self.bound(entry).ok_or(Error::NotFound)?;
        let ino = self.ino_of_link(child).ok_or(Error::NotFound)?;
        self.getattr(ino)
    }

    /// Lists a directory (without `.` and `..`).
    pub fn readdir(&self, ino: u64) -> Result<Vec<DirEntry>> {
        if let Some(entries) = control::readdir(ino) {
            return Ok(entries);
        }
        let dir = self.dir_link(ino)?;
        let mut entries = Vec::new();
        let mut failure = None;
        self.links.each_source(dir, &mut |entry| {
            let text = self.links.target(entry);
            if !names::is_string(&self.links, text) {
                return true;
            }
            let Some(child) = self.bound(entry).and_then(|link| self.ino_of_link(link)) else {
                return true;
            };
            match (
                names::read(&self.links, text, self.sb.name_encoding),
                self.inode(child).ok().and_then(|inode| inode.kind()),
            ) {
                (Ok(name), Some(kind)) => {
                    entries.push(DirEntry {
                        name,
                        ino: child,
                        kind,
                    });
                    true
                }
                (Err(error), _) => {
                    failure = Some(error);
                    false
                }
                (Ok(_), None) => {
                    failure = Some(Error::corrupt(format!("entry {entry} names a free inode")));
                    false
                }
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        if ino == ROOT_INO {
            entries.push(control::root_entry());
        }
        Ok(entries)
    }

    /// Parent directory of `ino` (the root is its own parent).
    pub fn parent(&self, ino: u64) -> Result<u64> {
        if control::is_control(ino) {
            return Ok(control::parent(ino));
        }
        if ino == ROOT_INO {
            return Ok(ROOT_INO);
        }
        let link = self.inode_link(ino)?;
        let mut parent = None;
        self.links.each_target(link, &mut |binding| {
            let entry = self.links.source(binding);
            parent = self.ino_of_link(self.links.source(entry));
            parent.is_none()
        });
        parent.ok_or(Error::NotFound)
    }

    /// One path of `ino` from the root (`/` for the root).
    pub fn path_of(&self, ino: u64) -> Result<String> {
        let mut parts = Vec::new();
        let mut current = ino;
        while current != ROOT_INO {
            if parts.len() > 4096 {
                return Err(Error::corrupt("directory cycle"));
            }
            let link = self.inode_link(current)?;
            let mut step = None;
            self.links.each_target(link, &mut |binding| {
                let entry = self.links.source(binding);
                step = Some((self.links.source(entry), self.links.target(entry)));
                false
            });
            let (dir, text) = step.ok_or(Error::NotFound)?;
            parts.push(names::read(&self.links, text, self.sb.name_encoding)?);
            current = self.ino_of_link(dir).ok_or(Error::NotFound)?;
        }
        parts.reverse();
        Ok(format!("/{}", parts.join("/")))
    }

    /// Resolves an absolute or root-relative path (symbolic links are not
    /// followed).
    pub fn resolve(&self, path: &str) -> Result<u64> {
        let mut ino = ROOT_INO;
        for part in path
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
        {
            ino = if part == ".." {
                self.parent(ino)?
            } else {
                self.lookup(ino, part)?.ino
            };
        }
        Ok(ino)
    }

    /// Splits a path into its parent directory inode and last name.
    pub fn resolve_parent<'p>(&self, path: &'p str) -> Result<(u64, &'p str)> {
        let trimmed = path.trim_end_matches('/');
        let (dir, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
        Ok((self.resolve(dir)?, name))
    }

    fn check_new_name(&self, parent: u64, name: &str) -> Result<u64> {
        self.check_writable()?;
        names::validate(name)?;
        if control::lookup(parent, name).is_some() || control::is_control(parent) {
            return Err(Error::Exists);
        }
        let dir = self.dir_link(parent)?;
        if self
            .entry(dir, name)
            .is_some_and(|entry| self.bound(entry).is_some())
        {
            return Err(Error::Exists);
        }
        Ok(dir)
    }

    /// Adds the entry `name -> child` to directory link `dir`.
    fn bind(&mut self, dir: u64, name: &str, child: u64) -> Result<()> {
        let text = names::intern(&mut self.links, name, self.sb.name_encoding)?;
        let entry = self.links.get_or_create(dir, text)?;
        self.links.create(entry, child)?;
        Ok(())
    }

    /// Removes the entry link `entry` (with its binding and unused name).
    fn unbind(&mut self, entry: u64) -> Result<()> {
        let users = self.links.users(crate::links::records::Role::Source, entry);
        for binding in users {
            self.links.delete(binding)?;
        }
        let text = self.links.target(entry);
        self.links.delete(entry)?;
        names::release(&mut self.links, text)
    }

    fn touch_dir(&mut self, ino: u64, nlink_delta: i32) -> Result<()> {
        let slot = Self::slot_of(ino)?;
        let mut inode = self.inode(ino)?;
        inode.nlink = inode.nlink.saturating_add_signed(nlink_delta);
        let time = now();
        inode.mtime = time;
        inode.ctime = time;
        self.inodes.set(slot, &inode);
        Ok(())
    }

    fn new_inode(&mut self, kind: FileKind, perm: u16, owner: Creator) -> Result<(u64, u64)> {
        let slot = self.inodes.allocate();
        let time = now();
        let inode = Inode {
            mode: kind.mode_bits() | u32::from(perm & 0o7777),
            nlink: if kind == FileKind::Dir { 2 } else { 1 },
            uid: owner.uid,
            gid: owner.gid,
            mtime: time,
            ctime: time,
            atime: time,
            ..Inode::default()
        };
        self.inodes.set(slot, &inode);
        match self.links.create(Self::type_point(kind), external(slot)) {
            Ok(link) => Ok((slot + 1, link)),
            Err(error) => {
                self.inodes.free(slot);
                Err(error)
            }
        }
    }

    fn make(
        &mut self,
        parent: u64,
        name: &str,
        kind: FileKind,
        perm: u16,
        owner: Creator,
        target: Option<&str>,
    ) -> Result<Attr> {
        let dir = self.check_new_name(parent, name)?;
        if let Some(target) = target {
            if target.is_empty() || target.len() > names::TARGET_MAX || target.contains('\0') {
                return Err(Error::invalid("bad symbolic link target"));
            }
        }
        let (ino, link) = self.new_inode(kind, perm, owner)?;
        if let Some(target) = target {
            let text = names::intern(&mut self.links, target, self.sb.name_encoding)?;
            self.links.create(link, text)?;
            let slot = Self::slot_of(ino)?;
            let mut inode = self.inodes.get(slot);
            inode.size = target.len() as u64;
            self.inodes.set(slot, &inode);
        }
        self.bind(dir, name, link)?;
        self.touch_dir(parent, i32::from(kind == FileKind::Dir))?;
        self.modified()?;
        self.getattr(ino)
    }

    /// Creates a regular file.
    pub fn create(&mut self, parent: u64, name: &str, perm: u16, owner: Creator) -> Result<Attr> {
        self.make(parent, name, FileKind::File, perm, owner, None)
    }

    /// Creates a directory.
    pub fn mkdir(&mut self, parent: u64, name: &str, perm: u16, owner: Creator) -> Result<Attr> {
        self.make(parent, name, FileKind::Dir, perm, owner, None)
    }

    /// Creates a symbolic link to `target`.
    pub fn symlink(
        &mut self,
        parent: u64,
        name: &str,
        target: &str,
        owner: Creator,
    ) -> Result<Attr> {
        self.make(parent, name, FileKind::Symlink, 0o777, owner, Some(target))
    }

    /// Reads a symbolic link.
    pub fn readlink(&self, ino: u64) -> Result<String> {
        if self.inode(ino)?.kind() != Some(FileKind::Symlink) {
            return Err(Error::invalid("not a symbolic link"));
        }
        let link = self.inode_link(ino)?;
        let mut text = None;
        self.links.each_source(link, &mut |pointer| {
            let target = self.links.target(pointer);
            if names::is_string(&self.links, target) {
                text = Some(target);
            }
            text.is_none()
        });
        let text = text.ok_or_else(|| Error::corrupt(format!("symlink {ino} has no target")))?;
        names::read(&self.links, text, self.sb.name_encoding)
    }

    /// Adds a hard link to a non-directory.
    pub fn link(&mut self, ino: u64, parent: u64, name: &str) -> Result<Attr> {
        let dir = self.check_new_name(parent, name)?;
        let mut inode = self.inode(ino)?;
        if inode.kind() == Some(FileKind::Dir) {
            return Err(Error::IsDir);
        }
        let link = self.inode_link(ino)?;
        self.bind(dir, name, link)?;
        inode.nlink += 1;
        inode.ctime = now();
        self.inodes.set(Self::slot_of(ino)?, &inode);
        self.touch_dir(parent, 0)?;
        self.modified()?;
        self.getattr(ino)
    }

    /// Finds the entry and child of `name` in `parent`.
    fn existing(&self, parent: u64, name: &str) -> Result<(u64, u64, u64)> {
        if control::lookup(parent, name).is_some() || control::is_control(parent) {
            return Err(Error::Unsupported(
                "the control directory is virtual".into(),
            ));
        }
        let dir = self.dir_link(parent)?;
        let entry = self.entry(dir, name).ok_or(Error::NotFound)?;
        let child = self.bound(entry).ok_or(Error::NotFound)?;
        let ino = self.ino_of_link(child).ok_or(Error::NotFound)?;
        Ok((dir, entry, ino))
    }

    fn is_empty_dir(&self, ino: u64) -> Result<bool> {
        let link = self.inode_link(ino)?;
        Ok(self
            .links
            .count_role(crate::links::records::Role::Source, link)
            == 0)
    }

    /// Drops one name of `ino`; frees the file when no name and no open
    /// handle is left.
    fn drop_name(&mut self, entry: u64, ino: u64) -> Result<()> {
        self.unbind(entry)?;
        let slot = Self::slot_of(ino)?;
        let mut inode = self.inode(ino)?;
        if inode.kind() == Some(FileKind::Dir) {
            inode.nlink = 0;
        } else {
            inode.nlink = inode.nlink.saturating_sub(1);
        }
        inode.ctime = now();
        self.inodes.set(slot, &inode);
        if inode.nlink == 0 && !self.open.contains_key(&ino) {
            self.destroy(ino)?;
        }
        Ok(())
    }

    /// Removes a non-directory name.
    pub fn unlink(&mut self, parent: u64, name: &str) -> Result<()> {
        self.check_writable()?;
        let (_, entry, ino) = self.existing(parent, name)?;
        if self.inode(ino)?.kind() == Some(FileKind::Dir) {
            return Err(Error::IsDir);
        }
        self.drop_name(entry, ino)?;
        self.touch_dir(parent, 0)?;
        self.modified()
    }

    /// Removes an empty directory.
    pub fn rmdir(&mut self, parent: u64, name: &str) -> Result<()> {
        self.check_writable()?;
        let (_, entry, ino) = self.existing(parent, name)?;
        if self.inode(ino)?.kind() != Some(FileKind::Dir) {
            return Err(Error::NotDir);
        }
        if !self.is_empty_dir(ino)? {
            return Err(Error::NotEmpty);
        }
        self.drop_name(entry, ino)?;
        self.touch_dir(parent, -1)?;
        self.modified()
    }

    /// Frees an inode with no names: its symlink target, its link, its
    /// extent and its record.
    pub(super) fn destroy(&mut self, ino: u64) -> Result<()> {
        let slot = Self::slot_of(ino)?;
        let inode = self.inodes.get(slot);
        if let Ok(link) = self.inode_link(ino) {
            for pointer in self.links.users(crate::links::records::Role::Source, link) {
                let text = self.links.target(pointer);
                self.links.delete(pointer)?;
                names::release(&mut self.links, text)?;
            }
            self.links.delete(link)?;
        }
        if inode.blocks > 0 {
            self.free_extent(inode.start, inode.blocks);
        }
        self.inodes.free(slot);
        self.open.remove(&ino);
        Ok(())
    }

    /// Renames `parent/name` to `new_parent/new_name`, replacing an existing
    /// target unless `no_replace`.
    pub fn rename(
        &mut self,
        parent: u64,
        name: &str,
        new_parent: u64,
        new_name: &str,
        no_replace: bool,
    ) -> Result<()> {
        self.check_writable()?;
        names::validate(new_name)?;
        let (_, entry, ino) = self.existing(parent, name)?;
        if control::lookup(new_parent, new_name).is_some() || control::is_control(new_parent) {
            return Err(Error::Exists);
        }
        let new_dir = self.dir_link(new_parent)?;
        if parent == new_parent && name == new_name {
            return Ok(());
        }
        let is_dir = self.inode(ino)?.kind() == Some(FileKind::Dir);
        if is_dir {
            let mut up = new_parent;
            loop {
                if up == ino {
                    return Err(Error::invalid("cannot move a directory into itself"));
                }
                if up == ROOT_INO {
                    break;
                }
                up = self.parent(up)?;
            }
        }
        if let Some(old_entry) = self
            .entry(new_dir, new_name)
            .filter(|&entry| self.bound(entry).is_some())
        {
            if no_replace {
                return Err(Error::Exists);
            }
            let old = self
                .bound(old_entry)
                .and_then(|link| self.ino_of_link(link))
                .ok_or(Error::NotFound)?;
            if old == ino {
                // Two names of one file: POSIX says do nothing.
                return Ok(());
            }
            let old_is_dir = self.inode(old)?.kind() == Some(FileKind::Dir);
            match (is_dir, old_is_dir) {
                (true, false) => return Err(Error::NotDir),
                (false, true) => return Err(Error::IsDir),
                (true, true) if !self.is_empty_dir(old)? => return Err(Error::NotEmpty),
                _ => {}
            }
            self.drop_name(old_entry, old)?;
            if old_is_dir {
                self.touch_dir(new_parent, -1)?;
            }
        }
        let link = self.inode_link(ino)?;
        self.bind(new_dir, new_name, link)?;
        self.unbind(entry)?;
        let moved_dir = i32::from(is_dir && parent != new_parent);
        self.touch_dir(parent, -moved_dir)?;
        self.touch_dir(new_parent, moved_dir)?;
        let slot = Self::slot_of(ino)?;
        let mut inode = self.inodes.get(slot);
        inode.ctime = now();
        self.inodes.set(slot, &inode);
        self.modified()
    }
}
