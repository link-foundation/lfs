//! The FUSE driver (Linux, macOS with macFUSE, FreeBSD).

use super::{nanos, permissions, system_time as time, MountOptions, Shared};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::fs::inode::{FileKind, S_IFMT, S_IFREG};
use crate::fs::names::NAME_MAX;
use crate::fs::{is_control, now, Attr, Creator, Fs, SetAttr, CONTROL_DIR_INO};
use fuser::{
    BackgroundSession, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, KernelConfig, LockOwner, MountOption, OpenFlags, RenameFlags, ReplyAttr,
    ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs,
    ReplyWrite, Request, SessionACL, TimeOrNow, WriteFlags,
};
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// How long the kernel may cache attributes and entries. Only this driver
/// changes the volume while it is mounted, so the cache stays right; the
/// control files change by themselves and are never cached.
const TTL: Duration = Duration::from_secs(1);

struct Driver<D: BlockDevice> {
    volume: Shared<D>,
}

fn errno(error: &Error) -> Errno {
    Errno::from_i32(error.errno())
}

fn name(name: &OsStr) -> Result<&str> {
    name.to_str()
        .ok_or_else(|| Error::invalid("names must be valid UTF-8"))
}

fn time_or_now(time: TimeOrNow) -> i64 {
    match time {
        TimeOrNow::Now => now(),
        TimeOrNow::SpecificTime(time) => nanos(time),
    }
}

const fn file_type(kind: FileKind) -> FileType {
    match kind {
        FileKind::File => FileType::RegularFile,
        FileKind::Dir => FileType::Directory,
        FileKind::Symlink => FileType::Symlink,
    }
}

fn file_attr(attr: &Attr, block_size: u32) -> FileAttr {
    FileAttr {
        ino: INodeNo(attr.ino),
        size: attr.size,
        blocks: attr.allocated.div_ceil(512),
        atime: time(attr.atime),
        mtime: time(attr.mtime),
        ctime: time(attr.ctime),
        crtime: time(attr.ctime),
        kind: file_type(attr.kind),
        perm: attr.perm,
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        rdev: 0,
        blksize: block_size,
        flags: 0,
    }
}

const fn ttl(ino: u64) -> Duration {
    if is_control(ino) {
        Duration::ZERO
    } else {
        TTL
    }
}

fn creator(req: &Request) -> Creator {
    Creator {
        uid: req.uid(),
        gid: req.gid(),
    }
}

impl<D: BlockDevice> Driver<D> {
    fn entry(&self, reply: ReplyEntry, made: impl FnOnce(&mut Fs<D>) -> Result<Attr>) {
        match self
            .volume
            .with(|fs| made(fs).map(|attr| (attr, fs.superblock().block_size)))
        {
            Ok((attr, bs)) => reply.entry(&ttl(attr.ino), &file_attr(&attr, bs), Generation(0)),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn attr(&self, reply: ReplyAttr, read: impl FnOnce(&mut Fs<D>) -> Result<Attr>) {
        match self
            .volume
            .with(|fs| read(fs).map(|attr| (attr, fs.superblock().block_size)))
        {
            Ok((attr, bs)) => reply.attr(&ttl(attr.ino), &file_attr(&attr, bs)),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn empty(&self, reply: ReplyEmpty, operation: impl FnOnce(&mut Fs<D>) -> Result<()>) {
        match self.volume.with(operation) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(errno(&error)),
        }
    }
}

impl<D: BlockDevice + Send + 'static> Filesystem for Driver<D> {
    fn init(&mut self, _req: &Request, _config: &mut KernelConfig) -> std::io::Result<()> {
        Ok(())
    }

    fn destroy(&mut self) {
        if let Err(error) = self.volume.close() {
            eprintln!("lfs: committing on unmount failed: {error}");
        }
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, entry: &OsStr, reply: ReplyEntry) {
        self.entry(reply, |fs| fs.lookup(parent.0, name(entry)?));
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        self.attr(reply, |fs| fs.getattr(ino.0));
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let changes = SetAttr {
            perm: mode.map(|mode| permissions(mode, 0)),
            uid,
            gid,
            size,
            atime: atime.map(time_or_now),
            mtime: mtime.map(time_or_now),
        };
        self.attr(reply, |fs| fs.setattr(ino.0, changes));
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        match self.volume.with(|fs| fs.readlink(ino.0)) {
            Ok(target) => reply.data(target.as_bytes()),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn mknod(
        &self,
        req: &Request,
        parent: INodeNo,
        entry: &OsStr,
        mode: u32,
        umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        if mode & S_IFMT != S_IFREG {
            reply.error(Errno::from_i32(Error::Unsupported(String::new()).errno()));
            return;
        }
        self.entry(reply, |fs| {
            fs.create(
                parent.0,
                name(entry)?,
                permissions(mode, umask),
                creator(req),
            )
        });
    }

    fn mkdir(
        &self,
        req: &Request,
        parent: INodeNo,
        entry: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        self.entry(reply, |fs| {
            fs.mkdir(
                parent.0,
                name(entry)?,
                permissions(mode, umask),
                creator(req),
            )
        });
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, entry: &OsStr, reply: ReplyEmpty) {
        self.empty(reply, |fs| fs.unlink(parent.0, name(entry)?));
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, entry: &OsStr, reply: ReplyEmpty) {
        self.empty(reply, |fs| fs.rmdir(parent.0, name(entry)?));
    }

    fn symlink(
        &self,
        req: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        self.entry(reply, |fs| {
            let target = name(target.as_os_str())?;
            fs.symlink(parent.0, name(link_name)?, target, creator(req))
        });
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        entry: &OsStr,
        new_parent: INodeNo,
        new_name: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let no_replace = flags.contains(RenameFlags::RENAME_NOREPLACE);
        if !(flags - RenameFlags::RENAME_NOREPLACE).is_empty() {
            reply.error(Errno::EINVAL);
            return;
        }
        self.empty(reply, |fs| {
            fs.rename(
                parent.0,
                name(entry)?,
                new_parent.0,
                name(new_name)?,
                no_replace,
            )
        });
    }

    fn link(
        &self,
        _req: &Request,
        ino: INodeNo,
        new_parent: INodeNo,
        new_name: &OsStr,
        reply: ReplyEntry,
    ) {
        self.entry(reply, |fs| fs.link(ino.0, new_parent.0, name(new_name)?));
    }

    fn open(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        match self.volume.with(|fs| fs.open_file(ino.0)) {
            Ok(()) => reply.opened(FileHandle(0), open_flags(ino.0)),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        match self.volume.with(|fs| fs.read(ino.0, offset, size as usize)) {
            Ok(data) => reply.data(&data),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        match self.volume.with(|fs| fs.write(ino.0, offset, data)) {
            Ok(written) => reply.written(written as u32),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn release(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.empty(reply, |fs| fs.release(ino.0));
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        self.empty(reply, Fs::commit);
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        match self.volume.with(|fs| fs.getattr(ino.0)) {
            Ok(attr) if attr.kind == FileKind::Dir => {
                reply.opened(FileHandle(0), FopenFlags::empty());
            }
            Ok(_) => reply.error(Errno::ENOTDIR),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let listing = self.volume.with(|fs| {
            let mut entries = vec![
                (ino.0, FileKind::Dir, ".".to_string()),
                (fs.parent(ino.0)?, FileKind::Dir, "..".to_string()),
            ];
            entries.extend(
                fs.readdir(ino.0)?
                    .into_iter()
                    .map(|entry| (entry.ino, entry.kind, entry.name)),
            );
            Ok(entries)
        });
        match listing {
            Ok(entries) => {
                for (index, (child, kind, entry)) in
                    entries.into_iter().enumerate().skip(offset as usize)
                {
                    if reply.add(INodeNo(child), index as u64 + 1, file_type(kind), entry) {
                        break;
                    }
                }
                reply.ok();
            }
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn fsyncdir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        self.empty(reply, Fs::commit);
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        match self.volume.with(|fs| Ok(fs.statfs())) {
            Ok(stats) => reply.statfs(
                stats.blocks,
                stats.free,
                stats.free,
                stats.files + stats.free,
                stats.free,
                stats.block_size,
                NAME_MAX as u32,
                stats.block_size,
            ),
            Err(error) => reply.error(errno(&error)),
        }
    }

    fn create(
        &self,
        req: &Request,
        parent: INodeNo,
        entry: &OsStr,
        mode: u32,
        umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let created = self.volume.with(|fs| {
            let attr = fs.create(
                parent.0,
                name(entry)?,
                permissions(mode, umask),
                creator(req),
            )?;
            fs.open_file(attr.ino)?;
            Ok((attr, fs.superblock().block_size))
        });
        match created {
            Ok((attr, bs)) => reply.created(
                &TTL,
                &file_attr(&attr, bs),
                Generation(0),
                FileHandle(0),
                FopenFlags::empty(),
            ),
            Err(error) => reply.error(errno(&error)),
        }
    }
}

/// Control files are generated on every read, so the page cache must not
/// keep them.
const fn open_flags(ino: u64) -> FopenFlags {
    if is_control(ino) && ino != CONTROL_DIR_INO {
        FopenFlags::FOPEN_DIRECT_IO
    } else {
        FopenFlags::empty()
    }
}

fn config(fs_writable: bool, options: &MountOptions) -> Config {
    let mut config = Config::default();
    let name = if options.name.is_empty() {
        "lfs".to_string()
    } else {
        options.name.clone()
    };
    config.mount_options = vec![
        MountOption::FSName(name),
        MountOption::Subtype("lfs".into()),
        MountOption::DefaultPermissions,
        MountOption::NoDev,
        MountOption::NoSuid,
        if options.read_only || !fs_writable {
            MountOption::RO
        } else {
            MountOption::RW
        },
    ];
    if options.allow_other {
        config.acl = SessionACL::All;
    }
    config
}

/// A volume mounted in the background.
pub struct Mounted {
    session: BackgroundSession,
}

impl Mounted {
    /// Whether the volume is still mounted (it ends when someone unmounts
    /// it with `fusermount3 -u` or `umount`).
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        !self.session.guard.is_finished()
    }

    /// Unmounts the volume and waits until it is committed and closed.
    pub fn unmount(self) -> Result<()> {
        self.session.umount_and_join().map_err(Error::Io)
    }

    /// Waits until someone else unmounts the volume.
    pub fn wait(self) -> Result<()> {
        self.session.join().map_err(Error::Io)
    }
}

/// Mounts `fs` at `mountpoint` and serves it from a background thread.
pub fn spawn<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    mountpoint: &Path,
    options: &MountOptions,
) -> Result<Mounted> {
    let config = config(fs.is_writable(), options);
    let driver = Driver {
        volume: Shared::new(fs),
    };
    let session = fuser::spawn_mount(driver, mountpoint, &config).map_err(Error::Io)?;
    Ok(Mounted { session })
}

/// Mounts `fs` at `mountpoint` and serves it until it is unmounted.
pub fn mount<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    mountpoint: &Path,
    options: &MountOptions,
) -> Result<()> {
    spawn(fs, mountpoint, options)?.wait()
}
