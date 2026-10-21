//! The Dokan driver (Windows, needs the Dokan 2 driver installed).
//!
//! Windows semantics (statuses, dispositions, attributes) live in
//! [`super::nt`], where they are tested on every platform; this file only
//! adapts them to the `dokan` crate.

use super::nt::{self, Action, Status};
use super::{nanos, system_time, MountOptions, Shared};
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::fs::names::NAME_MAX;
use crate::fs::{is_control, Attr, Creator, FileKind, Fs, SetAttr, ROOT_INO};
use dokan::{
    CreateFileInfo, DiskSpaceInfo, FileInfo, FileSystemHandler, FileSystemMounter,
    FileTimeOperation, FillDataError, FillDataResult, FindData, MountFlags, OperationInfo,
    OperationResult, VolumeInfo, IO_SECURITY_CONTEXT,
};
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use widestring::{U16CStr, U16CString};

/// An open file or directory.
#[derive(Debug)]
pub struct Handle {
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

fn volume_path(file_name: &U16CStr) -> std::result::Result<String, Status> {
    let name = file_name
        .to_string()
        .map_err(|_| nt::STATUS_OBJECT_NAME_INVALID)?;
    nt::path(&name)
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

fn file_info(attr: &Attr, name: &str) -> FileInfo {
    FileInfo {
        attributes: nt::attributes(attr, name),
        creation_time: system_time(attr.ctime),
        last_access_time: system_time(attr.atime),
        last_write_time: system_time(attr.mtime),
        file_size: attr.size,
        number_of_links: attr.nlink,
        file_index: attr.ino,
    }
}

fn find_data(attr: &Attr, name: &str) -> std::result::Result<FindData, Status> {
    Ok(FindData {
        attributes: nt::attributes(attr, name),
        creation_time: system_time(attr.ctime),
        last_access_time: system_time(attr.atime),
        last_write_time: system_time(attr.mtime),
        file_size: attr.size,
        file_name: U16CString::from_str(name).map_err(|_| nt::STATUS_OBJECT_NAME_INVALID)?,
    })
}

const fn time(operation: FileTimeOperation) -> Option<std::time::SystemTime> {
    match operation {
        FileTimeOperation::SetTime(time) => Some(time),
        _ => None,
    }
}

/// A short serial number derived from the volume name (FNV-1a).
fn serial(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5, |hash: u32, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
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
    ) -> std::result::Result<CreateFileInfo<Handle>, Status> {
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
        Ok(CreateFileInfo {
            context: Handle {
                ino: attr.ino,
                kind: attr.kind,
                opened,
            },
            is_dir: attr.kind == FileKind::Dir,
            new_file_created: created,
        })
    }
}

impl<'c, 'h: 'c, D: BlockDevice + Send + 'h> FileSystemHandler<'c, 'h> for Handler<D> {
    type Context = Handle;

    fn create_file(
        &'h self,
        file_name: &U16CStr,
        _security_context: &IO_SECURITY_CONTEXT,
        _desired_access: u32,
        _file_attributes: u32,
        _share_access: u32,
        create_disposition: u32,
        create_options: u32,
        _info: &mut OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<CreateFileInfo<Handle>> {
        let path = volume_path(file_name)?;
        self.call(|fs| Self::open(fs, &path, create_disposition, create_options))
    }

    fn cleanup(
        &'h self,
        file_name: &U16CStr,
        info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) {
        if !info.delete_on_close() {
            return;
        }
        let Ok(path) = volume_path(file_name) else {
            return;
        };
        let removed = self.call(|fs| {
            let (parent, name) = parent_of(fs, &path)?;
            if context.kind == FileKind::Dir {
                fs.rmdir(parent, name).nt()
            } else {
                fs.unlink(parent, name).nt()
            }
        });
        if let Err(status) = removed {
            eprintln!("lfs: deleting {path} failed (NTSTATUS {status:#x})");
        }
    }

    fn close_file(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) {
        if context.opened {
            if let Err(error) = self.volume.with(|fs| fs.release(context.ino)) {
                eprintln!("lfs: closing inode {} failed: {error}", context.ino);
            }
        }
    }

    fn read_file(
        &'h self,
        _file_name: &U16CStr,
        offset: i64,
        buffer: &mut [u8],
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<u32> {
        let offset = u64::try_from(offset).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        self.call(|fs| {
            let data = if context.kind == FileKind::Symlink {
                let target = fs.readlink(context.ino).nt()?.into_bytes();
                let start = usize::try_from(offset)
                    .unwrap_or(usize::MAX)
                    .min(target.len());
                target[start..].to_vec()
            } else {
                fs.read(context.ino, offset, buffer.len()).nt()?
            };
            let len = data.len().min(buffer.len());
            buffer[..len].copy_from_slice(&data[..len]);
            u32::try_from(len).map_err(|_| nt::STATUS_INVALID_PARAMETER)
        })
    }

    fn write_file(
        &'h self,
        _file_name: &U16CStr,
        offset: i64,
        buffer: &[u8],
        info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<u32> {
        self.call(|fs| {
            let size = fs.getattr(context.ino).nt()?.size;
            let offset = if info.write_to_eof() {
                size
            } else {
                u64::try_from(offset).map_err(|_| nt::STATUS_INVALID_PARAMETER)?
            };
            let mut data = buffer;
            if info.paging_io() {
                // Paging I/O never extends a file.
                let room = usize::try_from(size.saturating_sub(offset)).unwrap_or(usize::MAX);
                data = &data[..data.len().min(room)];
            }
            let written = fs.write(context.ino, offset, data).nt()?;
            u32::try_from(written).map_err(|_| nt::STATUS_INVALID_PARAMETER)
        })
    }

    fn flush_file_buffers(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Handle,
    ) -> OperationResult<()> {
        self.call(|fs| {
            if fs.is_writable() {
                fs.commit().nt()
            } else {
                Ok(())
            }
        })
    }

    fn get_file_information(
        &'h self,
        file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<FileInfo> {
        let path = volume_path(file_name)?;
        self.call(|fs| Ok(file_info(&fs.getattr(context.ino).nt()?, last_name(&path))))
    }

    fn find_files(
        &'h self,
        _file_name: &U16CStr,
        mut fill_find_data: impl FnMut(&FindData) -> FillDataResult,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<()> {
        let entries = self.call(|fs| {
            let mut entries = Vec::new();
            if context.ino != ROOT_INO {
                entries.push((".".to_string(), fs.getattr(context.ino).nt()?));
                let parent = fs.parent(context.ino).nt()?;
                entries.push(("..".to_string(), fs.getattr(parent).nt()?));
            }
            for entry in fs.readdir(context.ino).nt()? {
                entries.push((entry.name, fs.getattr(entry.ino).nt()?));
            }
            Ok(entries)
        })?;
        for (name, attr) in entries {
            match fill_find_data(&find_data(&attr, &name)?) {
                // Longer than MAX_PATH: Windows cannot list it.
                Ok(()) | Err(FillDataError::NameTooLong) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn set_file_attributes(
        &'h self,
        _file_name: &U16CStr,
        file_attributes: u32,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<()> {
        if file_attributes == 0 {
            return Ok(());
        }
        self.call(|fs| {
            let perm = fs.getattr(context.ino).nt()?.perm;
            let wanted = nt::permissions(perm, file_attributes);
            if wanted != perm {
                let changes = SetAttr {
                    perm: Some(wanted),
                    ..SetAttr::default()
                };
                fs.setattr(context.ino, changes).nt()?;
            }
            Ok(())
        })
    }

    fn set_file_time(
        &'h self,
        _file_name: &U16CStr,
        _creation_time: FileTimeOperation,
        last_access_time: FileTimeOperation,
        last_write_time: FileTimeOperation,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<()> {
        let changes = SetAttr {
            atime: time(last_access_time).map(nanos),
            mtime: time(last_write_time).map(nanos),
            ..SetAttr::default()
        };
        if changes.atime.is_none() && changes.mtime.is_none() {
            return Ok(());
        }
        self.call(|fs| fs.setattr(context.ino, changes).nt().map(drop))
    }

    fn delete_file(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<()> {
        self.call(|fs| {
            Self::writable(fs)?;
            if context.kind == FileKind::Dir || is_control(context.ino) {
                Err(nt::STATUS_ACCESS_DENIED)
            } else {
                Ok(())
            }
        })
    }

    fn delete_directory(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<()> {
        self.call(|fs| {
            Self::writable(fs)?;
            if context.ino == ROOT_INO || is_control(context.ino) {
                Err(nt::STATUS_ACCESS_DENIED)
            } else if fs.readdir(context.ino).nt()?.is_empty() {
                Ok(())
            } else {
                Err(nt::STATUS_DIRECTORY_NOT_EMPTY)
            }
        })
    }

    fn move_file(
        &'h self,
        file_name: &U16CStr,
        new_file_name: &U16CStr,
        replace_if_existing: bool,
        _info: &OperationInfo<'c, 'h, Self>,
        _context: &'c Handle,
    ) -> OperationResult<()> {
        let from = volume_path(file_name)?;
        let to = volume_path(new_file_name)?;
        self.call(|fs| {
            let (parent, name) = parent_of(fs, &from)?;
            let (new_parent, new_name) = parent_of(fs, &to)?;
            fs.rename(parent, name, new_parent, new_name, !replace_if_existing)
                .nt()
        })
    }

    fn set_end_of_file(
        &'h self,
        _file_name: &U16CStr,
        offset: i64,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<()> {
        let size = u64::try_from(offset).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        self.call(|fs| {
            let changes = SetAttr {
                size: Some(size),
                ..SetAttr::default()
            };
            fs.setattr(context.ino, changes).nt().map(drop)
        })
    }

    fn set_allocation_size(
        &'h self,
        _file_name: &U16CStr,
        alloc_size: i64,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Handle,
    ) -> OperationResult<()> {
        let alloc = u64::try_from(alloc_size).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        self.call(|fs| {
            // Shrinking the allocation below the size truncates; growing it
            // is a hint the contiguous allocator does not need.
            if alloc < fs.getattr(context.ino).nt()?.size {
                let changes = SetAttr {
                    size: Some(alloc),
                    ..SetAttr::default()
                };
                fs.setattr(context.ino, changes).nt()?;
            }
            Ok(())
        })
    }

    fn get_disk_free_space(
        &'h self,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<DiskSpaceInfo> {
        self.call(|fs| {
            let stats = fs.statfs();
            let block = u64::from(stats.block_size);
            Ok(DiskSpaceInfo {
                byte_count: stats.blocks * block,
                free_byte_count: stats.free * block,
                available_byte_count: stats.free * block,
            })
        })
    }

    fn get_volume_information(
        &'h self,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<VolumeInfo> {
        let mut fs_flags = nt::FILE_CASE_SENSITIVE_SEARCH
            | nt::FILE_CASE_PRESERVED_NAMES
            | nt::FILE_UNICODE_ON_DISK;
        if self.read_only {
            fs_flags |= nt::FILE_READ_ONLY_VOLUME;
        }
        Ok(VolumeInfo {
            name: U16CString::from_str(&self.name).map_err(|_| nt::STATUS_OBJECT_NAME_INVALID)?,
            serial_number: serial(&self.name),
            max_component_length: NAME_MAX as u32,
            fs_flags,
            fs_name: U16CString::from_str("LFS").map_err(|_| nt::STATUS_OBJECT_NAME_INVALID)?,
        })
    }

    fn mounted(
        &'h self,
        _mount_point: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<()> {
        Ok(())
    }

    fn unmounted(&'h self, _info: &OperationInfo<'c, 'h, Self>) -> OperationResult<()> {
        Ok(())
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
        // `false` only means it was already unmounted.
        let _ = dokan::unmount(&self.mount_point);
        self.wait()
    }

    /// Waits until someone else unmounts the volume.
    pub fn wait(self) -> Result<()> {
        self.thread
            .join()
            .map_err(|_| Error::corrupt("the Dokan thread panicked"))?
    }
}

fn serve<D: BlockDevice + Send>(
    handler: &Handler<D>,
    mount_point: &U16CStr,
    options: &dokan::MountOptions<'_>,
    ready: &mpsc::Sender<()>,
) -> Result<()> {
    dokan::init();
    let result = FileSystemMounter::new(handler, mount_point, options)
        .mount()
        .map(|file_system| {
            // Ignored when `spawn` stopped waiting.
            let _ = ready.send(());
            // Dropping the file system waits until it is unmounted.
            drop(file_system);
        })
        .map_err(|error| Error::Io(std::io::Error::other(format!("Dokan: {error}"))));
    dokan::shutdown();
    let closed = handler.volume.close();
    result.and(closed)
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
    let mut flags = MountFlags::CASE_SENSITIVE;
    if read_only {
        flags |= MountFlags::WRITE_PROTECT;
    }
    if !options.allow_other {
        flags |= MountFlags::CURRENT_SESSION;
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
    let thread = thread::spawn(move || {
        let options = dokan::MountOptions {
            flags,
            allocation_unit_size,
            ..dokan::MountOptions::default()
        };
        serve(&handler, &point, &options, &ready)
    });
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
