//! The Dokan driver (Windows, needs the Dokan 2 driver installed).
//!
//! Windows semantics (statuses, dispositions, attributes, times) live in
//! [`super::nt`], where they are tested on every platform. This module
//! runs them on the volume and [`ffi`] hands them to the Dokan library
//! through the raw `dokan-sys` bindings.
//!
//! The high-level `dokan` crate is not used: its `OperationInfo::context()`
//! makes a reference from `DOKAN_FILE_INFO.Context` without a null check,
//! and Dokan passes a null context for opens that never reached
//! `ZwCreateFile` (debug builds then abort with "null reference
//! produced"). [`ffi`] looks such files up by path instead.

mod ffi;

use super::nt::{self, Action, Status};
use super::{MountOptions, Shared};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::fs::{is_control, Attr, Creator, FileKind, Fs, SetAttr, ROOT_INO};
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use widestring::U16CString;

/// An open file or directory.
#[derive(Clone, Copy, Debug)]
struct Handle {
    ino: u64,
    kind: FileKind,
    /// Whether `Fs::open_file` was called and `Fs::release` is owed.
    opened: bool,
}

struct Handler<D: BlockDevice> {
    volume: Shared<D>,
    name: String,
    read_only: bool,
}

trait Nt<T> {
    fn nt(self) -> std::result::Result<T, Status>;
}

impl<T> Nt<T> for Result<T> {
    fn nt(self) -> std::result::Result<T, Status> {
        self.map_err(|error| nt::status(&error))
    }
}

fn last_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Splits `path` into its parent directory and name; a missing parent is
/// a missing path, as Windows reports it.
fn parent_of<'p, D: BlockDevice>(
    fs: &Fs<D>,
    path: &'p str,
) -> std::result::Result<(u64, &'p str), Status> {
    fs.resolve_parent(path).map_err(|error| match error {
        Error::NotFound | Error::NotDir => nt::STATUS_OBJECT_PATH_NOT_FOUND,
        error => nt::status(&error),
    })
}

/// Total and free bytes.
struct Space {
    total: u64,
    free: u64,
}

impl<D: BlockDevice> Handler<D> {
    fn call<T>(
        &self,
        operation: impl FnOnce(&mut Fs<D>) -> std::result::Result<T, Status>,
    ) -> std::result::Result<T, Status> {
        self.volume.with(|fs| Ok(operation(fs))).nt()?
    }

    const fn writable(fs: &Fs<D>) -> std::result::Result<(), Status> {
        if fs.is_writable() {
            Ok(())
        } else {
            Err(nt::STATUS_MEDIA_WRITE_PROTECTED)
        }
    }

    fn open(
        fs: &mut Fs<D>,
        path: &str,
        disposition: u32,
        options: u32,
    ) -> std::result::Result<(Handle, bool), Status> {
        let (attr, created) = if path == "/" {
            nt::action(disposition, options, Some(true))?;
            (fs.getattr(ROOT_INO).nt()?, false)
        } else {
            let (parent, name) = parent_of(fs, path)?;
            let existing = match fs.lookup(parent, name) {
                Ok(attr) => Some(attr),
                Err(Error::NotFound) => None,
                Err(error) => return Err(nt::status(&error)),
            };
            let action = nt::action(
                disposition,
                options,
                existing.as_ref().map(|attr| attr.kind == FileKind::Dir),
            )?;
            match (action, existing) {
                (Action::Open, Some(attr)) => (attr, false),
                (Action::Truncate, Some(attr)) => {
                    let changes = SetAttr {
                        size: Some(0),
                        ..SetAttr::default()
                    };
                    (fs.setattr(attr.ino, changes).nt()?, false)
                }
                (Action::CreateFile, None) => (
                    fs.create(parent, name, 0o644, Creator::default()).nt()?,
                    true,
                ),
                (Action::CreateDir, None) => (
                    fs.mkdir(parent, name, 0o755, Creator::default()).nt()?,
                    true,
                ),
                _ => return Err(nt::STATUS_INVALID_PARAMETER),
            }
        };
        if options & nt::FILE_DELETE_ON_CLOSE != 0
            && attr.kind == FileKind::Dir
            && !fs.readdir(attr.ino).nt()?.is_empty()
        {
            return Err(nt::STATUS_DIRECTORY_NOT_EMPTY);
        }
        let opened = attr.kind == FileKind::File;
        if opened {
            fs.open_file(attr.ino).nt()?;
        }
        let handle = Handle {
            ino: attr.ino,
            kind: attr.kind,
            opened,
        };
        Ok((handle, created))
    }

    /// `ZwCreateFile`: the handle and whether the name was created.
    fn create(
        &self,
        path: &str,
        disposition: u32,
        options: u32,
    ) -> std::result::Result<(Handle, bool), Status> {
        self.call(|fs| Self::open(fs, path, disposition, options))
    }

    /// A handle for a file Dokan opened without asking (no context).
    fn find(&self, path: &str) -> std::result::Result<Handle, Status> {
        self.call(|fs| {
            let attr = fs.getattr(fs.resolve(path).nt()?).nt()?;
            Ok(Handle {
                ino: attr.ino,
                kind: attr.kind,
                opened: false,
            })
        })
    }

    /// `Cleanup` of a handle opened for deletion.
    fn remove(&self, path: &str, handle: Handle) {
        let removed = self.call(|fs| {
            let (parent, name) = parent_of(fs, path)?;
            if handle.kind == FileKind::Dir {
                fs.rmdir(parent, name).nt()
            } else {
                fs.unlink(parent, name).nt()
            }
        });
        if let Err(status) = removed {
            eprintln!("lfs: deleting {path} failed (NTSTATUS {status:#x})");
        }
    }

    fn close(&self, handle: Handle) {
        if handle.opened {
            if let Err(error) = self.volume.with(|fs| fs.release(handle.ino)) {
                eprintln!("lfs: closing inode {} failed: {error}", handle.ino);
            }
        }
    }

    fn read(
        &self,
        handle: Handle,
        offset: i64,
        buffer: &mut [u8],
    ) -> std::result::Result<usize, Status> {
        let offset = u64::try_from(offset).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        self.call(|fs| {
            let data = if handle.kind == FileKind::Symlink {
                let target = fs.readlink(handle.ino).nt()?.into_bytes();
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(target.len());
                target[start..].to_vec()
            } else {
                fs.read(handle.ino, offset, buffer.len()).nt()?
            };
            let len = data.len().min(buffer.len());
            buffer[..len].copy_from_slice(&data[..len]);
            Ok(len)
        })
    }

    fn write(
        &self,
        handle: Handle,
        offset: i64,
        buffer: &[u8],
        to_end: bool,
        paging_io: bool,
    ) -> std::result::Result<usize, Status> {
        self.call(|fs| {
            let size = fs.getattr(handle.ino).nt()?.size;
            let offset = if to_end {
                size
            } else {
                u64::try_from(offset).map_err(|_| nt::STATUS_INVALID_PARAMETER)?
            };
            let mut data = buffer;
            if paging_io {
                // Paging I/O never extends a file.
                let room = usize::try_from(size.saturating_sub(offset)).unwrap_or(usize::MAX);
                data = &data[..data.len().min(room)];
            }
            fs.write(handle.ino, offset, data).nt()
        })
    }

    fn flush(&self) -> std::result::Result<(), Status> {
        self.call(|fs| {
            if fs.is_writable() {
                fs.commit().nt()
            } else {
                Ok(())
            }
        })
    }

    fn attr(&self, handle: Handle) -> std::result::Result<Attr, Status> {
        self.call(|fs| fs.getattr(handle.ino).nt())
    }

    /// `FindFiles`: the entries of a directory, with `.` and `..` outside
    /// the root.
    fn entries(&self, handle: Handle) -> std::result::Result<Vec<(String, Attr)>, Status> {
        self.call(|fs| {
            let mut entries = Vec::new();
            if handle.ino != ROOT_INO {
                entries.push((".".to_string(), fs.getattr(handle.ino).nt()?));
                let parent = fs.parent(handle.ino).nt()?;
                entries.push(("..".to_string(), fs.getattr(parent).nt()?));
            }
            for entry in fs.readdir(handle.ino).nt()? {
                entries.push((entry.name, fs.getattr(entry.ino).nt()?));
            }
            Ok(entries)
        })
    }

    fn set_attributes(&self, handle: Handle, attributes: u32) -> std::result::Result<(), Status> {
        if attributes == 0 {
            return Ok(());
        }
        self.call(|fs| {
            let perm = fs.getattr(handle.ino).nt()?.perm;
            let wanted = nt::permissions(perm, attributes);
            if wanted != perm {
                let changes = SetAttr {
                    perm: Some(wanted),
                    ..SetAttr::default()
                };
                fs.setattr(handle.ino, changes).nt()?;
            }
            Ok(())
        })
    }

    fn set_times(
        &self,
        handle: Handle,
        atime: Option<i64>,
        mtime: Option<i64>,
    ) -> std::result::Result<(), Status> {
        if atime.is_none() && mtime.is_none() {
            return Ok(());
        }
        let changes = SetAttr {
            atime,
            mtime,
            ..SetAttr::default()
        };
        self.call(|fs| fs.setattr(handle.ino, changes).nt().map(drop))
    }

    /// `DeleteFile`: whether the file may be deleted on close.
    fn can_delete_file(&self, handle: Handle) -> std::result::Result<(), Status> {
        self.call(|fs| {
            Self::writable(fs)?;
            if handle.kind == FileKind::Dir || is_control(handle.ino) {
                Err(nt::STATUS_ACCESS_DENIED)
            } else {
                Ok(())
            }
        })
    }

    /// `DeleteDirectory`: whether the directory may be deleted on close.
    fn can_delete_directory(&self, handle: Handle) -> std::result::Result<(), Status> {
        self.call(|fs| {
            Self::writable(fs)?;
            if handle.ino == ROOT_INO || is_control(handle.ino) {
                Err(nt::STATUS_ACCESS_DENIED)
            } else if fs.readdir(handle.ino).nt()?.is_empty() {
                Ok(())
            } else {
                Err(nt::STATUS_DIRECTORY_NOT_EMPTY)
            }
        })
    }

    fn rename(&self, from: &str, to: &str, replace: bool) -> std::result::Result<(), Status> {
        self.call(|fs| {
            let (parent, name) = parent_of(fs, from)?;
            let (new_parent, new_name) = parent_of(fs, to)?;
            fs.rename(parent, name, new_parent, new_name, !replace).nt()
        })
    }

    fn set_end_of_file(&self, handle: Handle, offset: i64) -> std::result::Result<(), Status> {
        let size = u64::try_from(offset).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        self.call(|fs| {
            let changes = SetAttr {
                size: Some(size),
                ..SetAttr::default()
            };
            fs.setattr(handle.ino, changes).nt().map(drop)
        })
    }

    fn set_allocation_size(&self, handle: Handle, size: i64) -> std::result::Result<(), Status> {
        let alloc = u64::try_from(size).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        self.call(|fs| {
            // Shrinking the allocation below the size truncates; growing it
            // is a hint the contiguous allocator does not need.
            if alloc < fs.getattr(handle.ino).nt()?.size {
                let changes = SetAttr {
                    size: Some(alloc),
                    ..SetAttr::default()
                };
                fs.setattr(handle.ino, changes).nt()?;
            }
            Ok(())
        })
    }

    fn space(&self) -> std::result::Result<Space, Status> {
        self.call(|fs| {
            let stats = fs.statfs();
            let block = u64::from(stats.block_size);
            Ok(Space {
                total: stats.blocks * block,
                free: stats.free * block,
            })
        })
    }

    /// `GetVolumeInformation` flags.
    const fn fs_flags(&self) -> u32 {
        let flags = nt::FILE_CASE_SENSITIVE_SEARCH
            | nt::FILE_CASE_PRESERVED_NAMES
            | nt::FILE_UNICODE_ON_DISK;
        if self.read_only {
            flags | nt::FILE_READ_ONLY_VOLUME
        } else {
            flags
        }
    }
}

/// A volume mounted in the background.
pub struct Mounted {
    mount_point: U16CString,
    thread: JoinHandle<Result<()>>,
}

impl Mounted {
    /// Whether the volume is still mounted (it ends when someone unmounts
    /// it, for example with `dokanctl /u M:`).
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        !self.thread.is_finished()
    }

    /// Unmounts the volume and waits until it is committed and closed.
    pub fn unmount(self) -> Result<()> {
        ffi::unmount(&self.mount_point);
        self.wait()
    }

    /// Waits until someone else unmounts the volume.
    pub fn wait(self) -> Result<()> {
        self.thread
            .join()
            .map_err(|_| Error::corrupt("the Dokan thread panicked"))?
    }
}

/// Mounts `fs` at `at` (a drive letter like `M:` or an empty NTFS
/// directory) and serves it from a background thread.
pub fn spawn<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    at: &Path,
    options: &MountOptions,
) -> Result<Mounted> {
    let mount_point = U16CString::from_os_str(at.as_os_str())
        .map_err(|_| Error::invalid("the mount point holds a NUL character"))?;
    let read_only = options.read_only || !fs.is_writable();
    let mut flags = dokan_sys::DOKAN_OPTION_CASE_SENSITIVE;
    if read_only {
        flags |= dokan_sys::DOKAN_OPTION_WRITE_PROTECT;
    }
    if !options.allow_other {
        flags |= dokan_sys::DOKAN_OPTION_CURRENT_SESSION;
    }
    let allocation_unit_size = fs.superblock().block_size;
    let handler = Handler {
        volume: Shared::new(fs),
        name: if options.name.is_empty() {
            "lfs".to_string()
        } else {
            options.name.clone()
        },
        read_only,
    };
    let (ready, mounted) = mpsc::channel();
    let point = mount_point.clone();
    let thread =
        thread::spawn(move || ffi::serve(&handler, &point, flags, allocation_unit_size, &ready));
    if mounted.recv().is_ok() {
        Ok(Mounted {
            mount_point,
            thread,
        })
    } else {
        // The thread ended without mounting: report why.
        thread
            .join()
            .map_err(|_| Error::corrupt("the Dokan thread panicked"))??;
        Err(Error::corrupt("Dokan stopped before mounting"))
    }
}

/// Mounts `fs` at `mountpoint` and serves it until it is unmounted.
pub fn mount<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    mountpoint: &Path,
    options: &MountOptions,
) -> Result<()> {
    spawn(fs, mountpoint, options)?.wait()
}
