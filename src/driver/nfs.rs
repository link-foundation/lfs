//! The NFS version 3 driver (`nfs` feature).
//!
//! It serves a volume over NFS version 3 on a TCP port, so a system mounts
//! it with its own NFS client and no kernel extension. macOS mounts it with
//! the built-in `mount_nfs` (as any user), Linux with `mount -t nfs` (as
//! root); see [`mount_command`].
//!
//! The server binds to `127.0.0.1` unless told otherwise, and answers
//! whoever reaches the port: NFS version 3 trusts the uid a client sends, so serve
//! other addresses only on a network you trust. Files are owned by the uid
//! and gid of the call that creates them.
//!
//! NFS version 3 has no open or close, so the driver trims growth slack and applies
//! control-file writes on `COMMIT` and on stable writes, which clients send
//! when a file is closed or synced. The volume commits on `COMMIT`, on
//! stable writes, on `commit-every` and when the server stops.

use super::{MountOptions, Shared};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::fs::inode::FileKind;
use crate::fs::{now, Attr, Creator, Fs, SetAttr, ROOT_INO};
use nfs3_server::nfs3_types::nfs3::{
    createverf3, fattr3, filename3, ftype3, nfspath3, nfsstat3, nfstime3, sattr3, set_atime,
    set_mtime, specdata3, stable_how, Nfs3Option,
};
use nfs3_server::nfs3_types::rpc::auth_unix;
use nfs3_server::nfs3_types::xdr_codec::{Pack, Unpack};
use nfs3_server::tcp::{NFSTcp, NFSTcpListener};
use nfs3_server::vfs::{
    DirEntryPlus, FileHandleU64, NextResult, NfsFileSystem, NfsReadFileSystem, ReadDirPlusIterator,
    VFSCapabilities,
};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// The file system id reported in every attribute.
const FSID: u64 = 0x4c46_5300;

/// Mode of directories made over NFS: `MKDIR` reaches the driver without
/// the client's mode, so clients that care follow with `SETATTR`.
const DIR_MODE: u16 = 0o755;

/// Mode of files made by an exclusive `CREATE`, which carries no mode.
const FILE_MODE: u16 = 0o644;

/// The outcome of an NFS operation.
type Reply<T> = std::result::Result<T, nfsstat3>;

struct Driver<D: BlockDevice> {
    volume: Arc<Shared<D>>,
    writable: bool,
    /// The credentials of the call being served. The server checks every
    /// call with [`NfsReadFileSystem::authorize`] right before running it
    /// on the same single-threaded runtime, and no operation here awaits,
    /// so this is the caller of the operation that reads it.
    caller: Mutex<Option<Creator>>,
}

const fn status(error: &Error) -> nfsstat3 {
    match error {
        Error::NotFound => nfsstat3::NFS3ERR_NOENT,
        Error::Exists => nfsstat3::NFS3ERR_EXIST,
        Error::NotDir => nfsstat3::NFS3ERR_NOTDIR,
        Error::IsDir => nfsstat3::NFS3ERR_ISDIR,
        Error::NotEmpty => nfsstat3::NFS3ERR_NOTEMPTY,
        Error::NoSpace => nfsstat3::NFS3ERR_NOSPC,
        Error::InvalidArgument(_) | Error::Busy(_) => nfsstat3::NFS3ERR_INVAL,
        Error::Unsupported(_) => nfsstat3::NFS3ERR_NOTSUPP,
        Error::NameTooLong => nfsstat3::NFS3ERR_NAMETOOLONG,
        Error::ReadOnly => nfsstat3::NFS3ERR_ROFS,
        Error::Io(_) | Error::Corrupt(_) => nfsstat3::NFS3ERR_IO,
    }
}

fn text(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| Error::invalid("names must be valid UTF-8"))
}

/// Converts nanoseconds since the epoch into NFS time (which starts at the
/// epoch and ends in 2106).
fn nfs_time(nanos: i64) -> nfstime3 {
    let nanos = u64::try_from(nanos).unwrap_or(0);
    nfstime3 {
        seconds: u32::try_from(nanos / 1_000_000_000).unwrap_or(u32::MAX),
        nseconds: (nanos % 1_000_000_000) as u32,
    }
}

fn nanos(time: nfstime3) -> i64 {
    i64::from(time.seconds) * 1_000_000_000 + i64::from(time.nseconds)
}

const fn file_type(kind: FileKind) -> ftype3 {
    match kind {
        FileKind::File => ftype3::NF3REG,
        FileKind::Dir => ftype3::NF3DIR,
        FileKind::Symlink => ftype3::NF3LNK,
    }
}

fn fattr(attr: &Attr) -> fattr3 {
    fattr3 {
        type_: file_type(attr.kind),
        mode: u32::from(attr.perm),
        nlink: attr.nlink,
        uid: attr.uid,
        gid: attr.gid,
        size: attr.size,
        used: attr.allocated,
        rdev: specdata3::default(),
        fsid: FSID,
        fileid: attr.ino,
        atime: nfs_time(attr.atime),
        mtime: nfs_time(attr.mtime),
        ctime: nfs_time(attr.ctime),
    }
}

const fn option<T: Copy + Pack + Unpack>(value: &Nfs3Option<T>) -> Option<T> {
    match value {
        Nfs3Option::Some(value) => Some(*value),
        Nfs3Option::None => None,
    }
}

/// The changes `attributes` ask for. Mode bits outside `0o7777` (the file
/// type) are ignored.
fn changes(attributes: &sattr3) -> SetAttr {
    SetAttr {
        perm: option(&attributes.mode).map(|mode| (mode & 0o7777) as u16),
        uid: option(&attributes.uid),
        gid: option(&attributes.gid),
        size: option(&attributes.size),
        atime: match attributes.atime {
            set_atime::DONT_CHANGE => None,
            set_atime::SET_TO_SERVER_TIME => Some(now()),
            set_atime::SET_TO_CLIENT_TIME(time) => Some(nanos(time)),
        },
        mtime: match attributes.mtime {
            set_mtime::DONT_CHANGE => None,
            set_mtime::SET_TO_SERVER_TIME => Some(now()),
            set_mtime::SET_TO_CLIENT_TIME(time) => Some(nanos(time)),
        },
    }
}

const fn changes_anything(changes: &SetAttr) -> bool {
    changes.perm.is_some()
        || changes.uid.is_some()
        || changes.gid.is_some()
        || changes.size.is_some()
        || changes.atime.is_some()
        || changes.mtime.is_some()
}

impl<D: BlockDevice> Driver<D> {
    fn run<T>(&self, operation: impl FnOnce(&mut Fs<D>) -> Result<T>) -> Reply<T> {
        self.volume.with(operation).map_err(|error| status(&error))
    }

    /// Who creates a file: the caller, else the owner of the root.
    fn creator(&self, fs: &Fs<D>) -> Result<Creator> {
        let caller = *self.caller.lock().unwrap_or_else(PoisonError::into_inner);
        caller.map_or_else(
            || {
                fs.getattr(ROOT_INO).map(|root| Creator {
                    uid: root.uid,
                    gid: root.gid,
                })
            },
            Ok,
        )
    }

    /// Makes a file, or opens the existing one (an unchecked `CREATE`),
    /// then applies the rest of `attributes`.
    fn create_file(&self, dir: u64, name: &[u8], attributes: &sattr3) -> Reply<Attr> {
        let mut changes = changes(attributes);
        self.run(|fs| {
            let name = text(name)?;
            let attr = match fs.lookup(dir, name) {
                Ok(attr) if attr.kind == FileKind::File => attr,
                Ok(_) => return Err(Error::Exists),
                Err(Error::NotFound) => {
                    let mut owner = self.creator(fs)?;
                    owner.uid = changes.uid.take().unwrap_or(owner.uid);
                    owner.gid = changes.gid.take().unwrap_or(owner.gid);
                    let perm = changes.perm.take().unwrap_or(FILE_MODE);
                    fs.create(dir, name, perm, owner)?
                }
                Err(error) => return Err(error),
            };
            if changes_anything(&changes) {
                fs.setattr(attr.ino, changes)
            } else {
                Ok(attr)
            }
        })
    }

    /// Ends a write burst on `ino`: trims slack or applies a control file,
    /// then commits.
    fn settle(&self, ino: u64) -> Reply<Attr> {
        self.run(|fs| {
            if fs.getattr(ino)?.kind == FileKind::File {
                fs.release(ino)?;
            }
            fs.commit()?;
            fs.getattr(ino)
        })
    }
}

/// A directory listing, paged by the server.
struct Listing(std::vec::IntoIter<DirEntryPlus<FileHandleU64>>);

#[allow(
    clippy::unused_async_trait_impl,
    reason = "handlers never await; see `Driver::caller`"
)]
impl ReadDirPlusIterator<FileHandleU64> for Listing {
    async fn next(&mut self) -> NextResult<DirEntryPlus<FileHandleU64>> {
        self.0.next().map_or(NextResult::Eof, NextResult::Ok)
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "handlers never await; see `Driver::caller`"
)]
impl<D: BlockDevice + Send + 'static> NfsReadFileSystem for Driver<D> {
    type Handle = FileHandleU64;

    fn root_dir(&self) -> FileHandleU64 {
        FileHandleU64::new(ROOT_INO)
    }

    fn authorize(&self, auth: Option<&auth_unix>) -> bool {
        *self.caller.lock().unwrap_or_else(PoisonError::into_inner) = auth.map(|auth| Creator {
            uid: auth.uid,
            gid: auth.gid,
        });
        true
    }

    async fn lookup(&self, dir: &FileHandleU64, name: &filename3<'_>) -> Reply<FileHandleU64> {
        let dir = dir.as_u64();
        self.run(|fs| match name.as_ref() {
            b"." => fs.getattr(dir).map(|attr| attr.ino),
            b".." => fs.parent(dir),
            name => fs.lookup(dir, text(name)?).map(|attr| attr.ino),
        })
        .map(FileHandleU64::new)
    }

    async fn getattr(&self, id: &FileHandleU64) -> Reply<fattr3> {
        self.run(|fs| fs.getattr(id.as_u64()))
            .map(|attr| fattr(&attr))
    }

    async fn read(&self, id: &FileHandleU64, offset: u64, count: u32) -> Reply<(Vec<u8>, bool)> {
        let ino = id.as_u64();
        self.run(|fs| {
            let data = fs.read(ino, offset, count as usize)?;
            let size = fs.getattr(ino)?.size;
            let eof = offset + data.len() as u64 >= size;
            Ok((data, eof))
        })
    }

    async fn readdirplus(
        &self,
        dir: &FileHandleU64,
        cookie: u64,
    ) -> Reply<impl ReadDirPlusIterator<FileHandleU64>> {
        let dir = dir.as_u64();
        let entries = self.run(|fs| {
            let mut entries = vec![(dir, ".".to_string()), (fs.parent(dir)?, "..".to_string())];
            entries.extend(
                fs.readdir(dir)?
                    .into_iter()
                    .map(|entry| (entry.ino, entry.name)),
            );
            // The cookie of an entry is its position plus one, so a cookie
            // names the entry the next page starts after.
            entries
                .into_iter()
                .enumerate()
                .skip(usize::try_from(cookie).unwrap_or(usize::MAX))
                .map(|(index, (ino, name))| {
                    let attr = fs.getattr(ino)?;
                    Ok(DirEntryPlus {
                        fileid: ino,
                        name: name.into_bytes().into(),
                        cookie: index as u64 + 1,
                        name_attributes: Some(fattr(&attr)),
                        name_handle: Some(FileHandleU64::new(ino)),
                    })
                })
                .collect::<Result<Vec<_>>>()
        })?;
        Ok(Listing(entries.into_iter()))
    }

    async fn readlink(&self, id: &FileHandleU64) -> Reply<nfspath3<'_>> {
        self.run(|fs| fs.readlink(id.as_u64()))
            .map(|target| target.into_bytes().into())
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "handlers never await; see `Driver::caller`"
)]
impl<D: BlockDevice + Send + 'static> NfsFileSystem for Driver<D> {
    fn capabilities(&self) -> VFSCapabilities {
        if self.writable {
            VFSCapabilities::ReadWrite
        } else {
            VFSCapabilities::ReadOnly
        }
    }

    async fn setattr(&self, id: &FileHandleU64, attributes: sattr3) -> Reply<fattr3> {
        let changes = changes(&attributes);
        self.run(|fs| fs.setattr(id.as_u64(), changes))
            .map(|attr| fattr(&attr))
    }

    async fn write(
        &self,
        id: &FileHandleU64,
        offset: u64,
        data: &[u8],
        stable: stable_how,
    ) -> Reply<(fattr3, stable_how)> {
        let ino = id.as_u64();
        let attr = self.run(|fs| {
            let written = fs.write(ino, offset, data)?;
            if written < data.len() {
                return Err(Error::NoSpace);
            }
            fs.getattr(ino)
        })?;
        if stable == stable_how::UNSTABLE {
            Ok((fattr(&attr), stable_how::UNSTABLE))
        } else {
            Ok((fattr(&self.settle(ino)?), stable_how::FILE_SYNC))
        }
    }

    async fn create(
        &self,
        dir: &FileHandleU64,
        name: &filename3<'_>,
        attributes: sattr3,
    ) -> Reply<(FileHandleU64, fattr3)> {
        let attr = self.create_file(dir.as_u64(), name.as_ref(), &attributes)?;
        Ok((FileHandleU64::new(attr.ino), fattr(&attr)))
    }

    async fn create_exclusive(
        &self,
        dir: &FileHandleU64,
        name: &filename3<'_>,
        _verifier: createverf3,
    ) -> Reply<FileHandleU64> {
        // The server answers a retransmitted call from its own cache, so an
        // existing name is a real conflict.
        let dir = dir.as_u64();
        self.run(|fs| {
            let owner = self.creator(fs)?;
            fs.create(dir, text(name.as_ref())?, FILE_MODE, owner)
        })
        .map(|attr| FileHandleU64::new(attr.ino))
    }

    async fn mkdir(
        &self,
        dir: &FileHandleU64,
        name: &filename3<'_>,
    ) -> Reply<(FileHandleU64, fattr3)> {
        let dir = dir.as_u64();
        let attr = self.run(|fs| {
            let owner = self.creator(fs)?;
            fs.mkdir(dir, text(name.as_ref())?, DIR_MODE, owner)
        })?;
        Ok((FileHandleU64::new(attr.ino), fattr(&attr)))
    }

    async fn remove(&self, dir: &FileHandleU64, name: &filename3<'_>) -> Reply<()> {
        // NFS removes files and directories with the same call.
        let dir = dir.as_u64();
        self.run(|fs| {
            let name = text(name.as_ref())?;
            if fs.lookup(dir, name)?.kind == FileKind::Dir {
                fs.rmdir(dir, name)
            } else {
                fs.unlink(dir, name)
            }
        })
    }

    async fn rename<'a>(
        &self,
        from_dir: &FileHandleU64,
        from_name: &filename3<'a>,
        to_dir: &FileHandleU64,
        to_name: &filename3<'a>,
    ) -> Reply<()> {
        self.run(|fs| {
            fs.rename(
                from_dir.as_u64(),
                text(from_name.as_ref())?,
                to_dir.as_u64(),
                text(to_name.as_ref())?,
                false,
            )
        })
    }

    async fn symlink<'a>(
        &self,
        dir: &FileHandleU64,
        name: &filename3<'a>,
        target: &nfspath3<'a>,
        _attributes: &sattr3,
    ) -> Reply<(FileHandleU64, fattr3)> {
        let dir = dir.as_u64();
        let attr = self.run(|fs| {
            let owner = self.creator(fs)?;
            fs.symlink(dir, text(name.as_ref())?, text(target.as_ref())?, owner)
        })?;
        Ok((FileHandleU64::new(attr.ino), fattr(&attr)))
    }

    async fn commit(&self, id: &FileHandleU64, _offset: u64, _count: u32) -> Reply<()> {
        self.settle(id.as_u64()).map(drop)
    }
}

/// A volume served over NFS from a background thread.
pub struct Served {
    address: SocketAddr,
    mounted: Arc<AtomicBool>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<()>>>,
}

impl Served {
    /// Where the server listens.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// The TCP port of both the NFS and the mount protocol.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.address.port()
    }

    /// Whether the server still runs.
    #[must_use]
    pub fn is_serving(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }

    /// Whether a client mounted the export and has not unmounted it yet.
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        self.mounted.load(Ordering::SeqCst)
    }

    /// Stops the server and waits until the volume is committed and closed.
    pub fn stop(mut self) -> Result<()> {
        self.finish()
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            // Fails only when the server already ended.
            let _ = stop.send(());
        }
        self.thread.take().map_or(Ok(()), |thread| {
            thread
                .join()
                .unwrap_or_else(|_| Err(Error::corrupt("the NFS server failed unexpectedly")))
        })
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("lfs: stopping the NFS server failed: {error}");
        }
    }
}

/// Serves `fs` over NFS version 3 at `address` (port 0 picks a free port) from a
/// background thread, until [`Served::stop`].
pub fn serve<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    address: SocketAddr,
    options: &MountOptions,
) -> Result<Served> {
    let volume = Arc::new(Shared::new(fs));
    let driver = Driver {
        writable: volume.with(|fs| Ok(fs.is_writable()))? && !options.read_only,
        volume: Arc::clone(&volume),
        caller: Mutex::new(None),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(Error::Io)?;
    let mounted = Arc::new(AtomicBool::new(false));
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let (ready, listening) = mpsc::channel();
    let watched = Arc::clone(&mounted);
    let thread = std::thread::Builder::new()
        .name("lfs-nfs".into())
        .spawn(move || {
            let served = runtime.block_on(async {
                let bound = NFSTcpListener::bind(&address.to_string(), driver).await;
                let mut listener = match bound {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return Ok(());
                    }
                };
                let (signal, mut signals) = tokio::sync::mpsc::channel(4);
                listener.set_mount_listener(signal);
                tokio::spawn(async move {
                    while let Some(mounted) = signals.recv().await {
                        watched.store(mounted, Ordering::SeqCst);
                    }
                });
                let port = listener.get_listen_port();
                let _ = ready.send(Ok(SocketAddr::new(address.ip(), port)));
                tokio::select! {
                    served = listener.handle_forever() => served.map_err(Error::Io),
                    _ = stopped => Ok(()),
                }
            });
            // Ends every connection, so the driver is ours alone again.
            drop(runtime);
            let closed = volume.close();
            served.and(closed)
        })
        .map_err(Error::Io)?;
    match listening.recv() {
        Ok(Ok(address)) => Ok(Served {
            address,
            mounted,
            stop: Some(stop),
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(Error::Io(error))
        }
        Err(_) => Err(thread.join().map_or_else(
            |_| Error::corrupt("the NFS server failed to start"),
            |result| {
                result
                    .err()
                    .unwrap_or_else(|| Error::corrupt("the NFS server did not start"))
            },
        )),
    }
}

/// The command that mounts the server on `port` of this host at `mountpoint`.
///
/// It mounts the export `/` with `mount_nfs` on macOS (no root needed for a
/// directory you own), and with `mount -t nfs` elsewhere (as root, with the
/// NFS client tools such as Debian's `nfs-common`).
#[must_use]
pub fn mount_command(port: u16, mountpoint: &Path) -> Command {
    let source = "127.0.0.1:/";
    if cfg!(target_os = "macos") {
        let mut command = Command::new("/sbin/mount_nfs");
        command.args([
            "-o",
            &format!("nolocks,vers=3,tcp,port={port},mountport={port}"),
            source,
        ]);
        command.arg(mountpoint);
        command
    } else {
        let mut command = Command::new("mount");
        command.args([
            "-t",
            "nfs",
            "-o",
            &format!("nolock,noacl,vers=3,tcp,port={port},mountport={port},mountproto=tcp"),
            source,
        ]);
        command.arg(mountpoint);
        command
    }
}

/// The command that unmounts `mountpoint`.
#[must_use]
pub fn unmount_command(mountpoint: &Path) -> Command {
    let mut command = Command::new(if cfg!(target_os = "macos") {
        "/sbin/umount"
    } else {
        "umount"
    });
    command.arg(mountpoint);
    command
}

fn run(mut command: Command) -> Result<()> {
    let output = command.output().map_err(Error::Io)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Io(std::io::Error::other(format!(
            "{:?} failed ({}): {}",
            command,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))))
    }
}

/// A volume served over NFS and mounted with the system's NFS client.
pub struct NfsMounted {
    served: Served,
    mountpoint: PathBuf,
}

impl NfsMounted {
    /// The server behind the mount.
    #[must_use]
    pub const fn served(&self) -> &Served {
        &self.served
    }

    /// Whether the volume is still mounted (it ends when someone unmounts
    /// it with `umount`).
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        self.served.is_serving() && self.served.is_mounted()
    }

    /// Unmounts the volume, then stops the server, which commits and closes
    /// the volume.
    pub fn unmount(self) -> Result<()> {
        let unmounted = if self.served.is_mounted() {
            run(unmount_command(&self.mountpoint))
        } else {
            Ok(())
        };
        let stopped = self.served.stop();
        unmounted.and(stopped)
    }
}

/// Serves `fs` on a free port of `127.0.0.1` and mounts it at `mountpoint`
/// with [`mount_command`].
pub fn spawn<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    mountpoint: &Path,
    options: &MountOptions,
) -> Result<NfsMounted> {
    let served = serve(fs, SocketAddr::from(([127, 0, 0, 1], 0)), options)?;
    run(mount_command(served.port(), mountpoint))?;
    Ok(NfsMounted {
        served,
        mountpoint: mountpoint.to_path_buf(),
    })
}
