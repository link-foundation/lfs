//! The Dokan callbacks and the mount loop, over the raw `dokan-sys`
//! bindings.
//!
//! Every callback runs inside [`run`], which turns a Rust error into its
//! `NTSTATUS` and a panic into `STATUS_INTERNAL_ERROR`, so nothing unwinds
//! into the Dokan library. Set `LFS_DOKAN_TRACE=1` to log every callback
//! with its path and status on stderr.

// Calling into Dokan and reading what it passes needs `unsafe`; every
// pointer is checked for null before use.
#![allow(unsafe_code)]

use super::{last_name, Handle, Handler};
use crate::device::BlockDevice;
use crate::driver::nt::{self, Status};
use crate::error::{Error, Result};
use crate::fs::names::NAME_MAX;
use crate::fs::{Attr, FileKind};
use dokan_sys::{
    DokanCloseHandle, DokanCreateFileSystem, DokanInit, DokanRemoveMountPoint, DokanShutdown,
    DokanWaitForFileSystemClosed, DOKAN_FILE_INFO, DOKAN_HANDLE, DOKAN_OPERATIONS, DOKAN_OPTIONS,
    PDOKAN_FILE_INFO, PDOKAN_IO_SECURITY_CONTEXT, VOLUME_SECURITY_DESCRIPTOR_MAX_SIZE,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::{mpsc, OnceLock};
use widestring::U16CStr;
use winapi::shared::minwindef::{BOOL, DWORD, FILETIME, LPCVOID, LPDWORD, LPVOID, MAX_PATH};
use winapi::shared::ntdef::{LPCWSTR, LPWSTR, NTSTATUS, PULONGLONG};
use winapi::um::fileapi::{BY_HANDLE_FILE_INFORMATION, LPBY_HANDLE_FILE_INFORMATION};
use winapi::um::minwinbase::WIN32_FIND_DATAW;
use winapi::um::winnt::ACCESS_MASK;

type NtResult<T> = std::result::Result<T, Status>;

/// `PFillFindData`: adds an entry to a directory listing.
type FillFindData =
    unsafe extern "system" fn(*mut WIN32_FIND_DATAW, PDOKAN_FILE_INFO) -> std::ffi::c_int;

/// `INFINITE` for `DokanWaitForFileSystemClosed`.
const INFINITE: DWORD = 0xFFFF_FFFF;

fn tracing() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("LFS_DOKAN_TRACE").is_some_and(|value| value != "0"))
}

/// Runs a callback body: `Ok` is `STATUS_SUCCESS`, `Err` its status and a
/// panic `STATUS_INTERNAL_ERROR`.
fn run(operation: &str, name: LPCWSTR, body: impl FnOnce() -> NtResult<()>) -> NTSTATUS {
    let status = match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(())) => nt::STATUS_SUCCESS,
        Ok(Err(status)) => status,
        Err(_) => nt::STATUS_INTERNAL_ERROR,
    };
    if tracing() {
        // SAFETY: Dokan passes NUL-terminated names (or none).
        let name = unsafe { wide(name) }.map(U16CStr::to_string_lossy);
        eprintln!(
            "lfs dokan: {operation} {} -> {status:#010x}",
            name.unwrap_or_default()
        );
    }
    status
}

/// # Safety
/// `name` is null or a NUL-terminated UTF-16 string.
unsafe fn wide<'a>(name: LPCWSTR) -> Option<&'a U16CStr> {
    (!name.is_null()).then(|| U16CStr::from_ptr_str(name))
}

/// The volume path for a Dokan file name.
///
/// # Safety
/// As for [`wide`].
unsafe fn path(name: LPCWSTR) -> NtResult<String> {
    let name = wide(name).ok_or(nt::STATUS_INVALID_PARAMETER)?;
    nt::path(
        &name
            .to_string()
            .map_err(|_| nt::STATUS_OBJECT_NAME_INVALID)?,
    )
}

/// The handler a file system was mounted with.
///
/// # Safety
/// `info` comes from Dokan for a file system [`serve`] mounted with a
/// `Handler<D>`.
unsafe fn handler<'a, D: BlockDevice>(info: PDOKAN_FILE_INFO) -> &'a Handler<D> {
    &*((*(*info).DokanOptions).GlobalContext as usize as *const Handler<D>)
}

/// The handle `ZwCreateFile` stored, or, when Dokan opened the file
/// without asking (a null context), one found by path.
///
/// # Safety
/// As for [`handler`] and [`wide`].
unsafe fn handle<D: BlockDevice>(info: PDOKAN_FILE_INFO, name: LPCWSTR) -> NtResult<Handle> {
    let context = (*info).Context as usize as *const Handle;
    if context.is_null() {
        handler::<D>(info).find(&path(name)?)
    } else {
        Ok(*context)
    }
}

/// Takes the handle `ZwCreateFile` stored, leaving a null context.
///
/// # Safety
/// `info` is a live `DOKAN_FILE_INFO` whose context is null or a boxed
/// [`Handle`].
unsafe fn take_context(info: &mut DOKAN_FILE_INFO) -> Option<Handle> {
    let context = std::mem::replace(&mut info.Context, 0) as usize as *mut Handle;
    (!context.is_null()).then(|| *Box::from_raw(context))
}

/// Splits a 64-bit value into its low and high halves.
const fn halves(value: u64) -> (DWORD, DWORD) {
    (value as DWORD, (value >> 32) as DWORD)
}

const fn filetime(nanos: i64) -> FILETIME {
    let (low, high) = halves(nt::filetime(nanos));
    FILETIME {
        dwLowDateTime: low,
        dwHighDateTime: high,
    }
}

/// # Safety
/// `time` is null or points to a `FILETIME`.
unsafe fn set_time(time: *const FILETIME) -> Option<i64> {
    let time = time.as_ref()?;
    nt::set_time(u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime))
}

/// # Safety
/// `out` is null or points to writable memory for a `T`.
unsafe fn put<T>(out: *mut T, value: T) {
    if !out.is_null() {
        out.write(value);
    }
}

/// Copies `text` into a buffer of `size` UTF-16 units, NUL-terminated and
/// cut to fit.
///
/// # Safety
/// `buffer` is null or holds `size` units.
unsafe fn copy_wide(text: &str, buffer: LPWSTR, size: DWORD) {
    let size = usize::try_from(size).unwrap_or(0);
    if buffer.is_null() || size == 0 {
        return;
    }
    let text: Vec<u16> = text.encode_utf16().collect();
    let len = text.len().min(size - 1);
    ptr::copy_nonoverlapping(text.as_ptr(), buffer, len);
    buffer.add(len).write(0);
}

/// The `WIN32_FIND_DATAW` for a directory entry; `None` when the name does
/// not fit in `MAX_PATH` units, so Windows cannot list it.
fn find_data(name: &str, attr: &Attr) -> Option<WIN32_FIND_DATAW> {
    let wide: Vec<u16> = name.encode_utf16().collect();
    if wide.len() >= MAX_PATH {
        return None;
    }
    // SAFETY: plain data, for which all zeroes is valid.
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    data.cFileName[..wide.len()].copy_from_slice(&wide);
    data.dwFileAttributes = nt::attributes(attr, name);
    data.ftCreationTime = filetime(attr.ctime);
    data.ftLastAccessTime = filetime(attr.atime);
    data.ftLastWriteTime = filetime(attr.mtime);
    (data.nFileSizeLow, data.nFileSizeHigh) = halves(attr.size);
    Some(data)
}

#[allow(clippy::too_many_arguments)]
extern "system" fn create_file<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    _security_context: PDOKAN_IO_SECURITY_CONTEXT,
    _desired_access: ACCESS_MASK,
    _file_attributes: u32,
    _share_access: u32,
    disposition: u32,
    options: u32,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("create", file_name, || unsafe {
        let outcome =
            path(file_name).and_then(|path| handler::<D>(info).create(&path, disposition, options));
        let created = outcome.map(|(handle, created)| {
            let info = &mut *info;
            // A context from an earlier open would leak otherwise.
            if let Some(old) = take_context(info) {
                handler::<D>(info).close(old);
            }
            info.Context = Box::into_raw(Box::new(handle)) as usize as u64;
            info.IsDirectory = u8::from(handle.kind == FileKind::Dir);
            created
        });
        match nt::create_status(disposition, created) {
            nt::STATUS_SUCCESS => Ok(()),
            status => Err(status),
        }
    })
}

extern "system" fn cleanup<D: BlockDevice + Send>(file_name: LPCWSTR, info: PDOKAN_FILE_INFO) {
    run("cleanup", file_name, || unsafe {
        if (*info).DeleteOnClose != 0 {
            let handle = handle::<D>(info, file_name)?;
            handler::<D>(info).remove(&path(file_name)?, handle);
        }
        Ok(())
    });
}

extern "system" fn close_file<D: BlockDevice + Send>(file_name: LPCWSTR, info: PDOKAN_FILE_INFO) {
    run("close", file_name, || unsafe {
        if let Some(handle) = take_context(&mut *info) {
            handler::<D>(info).close(handle);
        }
        Ok(())
    });
}

extern "system" fn read_file<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    buffer: LPVOID,
    length: DWORD,
    read: LPDWORD,
    offset: i64,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("read", file_name, || unsafe {
        put(read, 0);
        let len = usize::try_from(length).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        if buffer.is_null() || len == 0 {
            return Ok(());
        }
        let buffer = std::slice::from_raw_parts_mut(buffer.cast::<u8>(), len);
        let handle = handle::<D>(info, file_name)?;
        let done = handler::<D>(info).read(handle, offset, buffer)?;
        put(read, DWORD::try_from(done).unwrap_or(length));
        Ok(())
    })
}

extern "system" fn write_file<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    buffer: LPCVOID,
    length: DWORD,
    written: LPDWORD,
    offset: i64,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("write", file_name, || unsafe {
        put(written, 0);
        let len = usize::try_from(length).map_err(|_| nt::STATUS_INVALID_PARAMETER)?;
        if buffer.is_null() || len == 0 {
            return Ok(());
        }
        let buffer = std::slice::from_raw_parts(buffer.cast::<u8>(), len);
        let handle = handle::<D>(info, file_name)?;
        let (to_end, paging_io) = ((*info).WriteToEndOfFile != 0, (*info).PagingIo != 0);
        let done = handler::<D>(info).write(handle, offset, buffer, to_end, paging_io)?;
        put(written, DWORD::try_from(done).unwrap_or(length));
        Ok(())
    })
}

extern "system" fn flush_file_buffers<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("flush", file_name, || unsafe { handler::<D>(info).flush() })
}

extern "system" fn get_file_information<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    buffer: LPBY_HANDLE_FILE_INFORMATION,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("info", file_name, || unsafe {
        let handler = handler::<D>(info);
        let attr = handler.attr(handle::<D>(info, file_name)?)?;
        let path = path(file_name)?;
        let (size_low, size_high) = halves(attr.size);
        let (index_low, index_high) = halves(attr.ino);
        put(
            buffer,
            BY_HANDLE_FILE_INFORMATION {
                dwFileAttributes: nt::attributes(&attr, last_name(&path)),
                ftCreationTime: filetime(attr.ctime),
                ftLastAccessTime: filetime(attr.atime),
                ftLastWriteTime: filetime(attr.mtime),
                dwVolumeSerialNumber: nt::serial(&handler.name),
                nFileSizeHigh: size_high,
                nFileSizeLow: size_low,
                nNumberOfLinks: attr.nlink,
                nFileIndexHigh: index_high,
                nFileIndexLow: index_low,
            },
        );
        Ok(())
    })
}

extern "system" fn find_files<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    fill: FillFindData,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("list", file_name, || unsafe {
        let entries = handler::<D>(info).entries(handle::<D>(info, file_name)?)?;
        for (name, attr) in entries {
            if let Some(mut data) = find_data(&name, &attr) {
                // Non-zero: the buffer is full.
                if fill(&raw mut data, info) != 0 {
                    return Err(nt::STATUS_BUFFER_OVERFLOW);
                }
            }
        }
        Ok(())
    })
}

extern "system" fn set_file_attributes<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    attributes: DWORD,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("attributes", file_name, || unsafe {
        handler::<D>(info).set_attributes(handle::<D>(info, file_name)?, attributes)
    })
}

extern "system" fn set_file_time<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    _creation_time: *const FILETIME,
    last_access_time: *const FILETIME,
    last_write_time: *const FILETIME,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("times", file_name, || unsafe {
        let (atime, mtime) = (set_time(last_access_time), set_time(last_write_time));
        handler::<D>(info).set_times(handle::<D>(info, file_name)?, atime, mtime)
    })
}

extern "system" fn delete_file<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("delete-file", file_name, || unsafe {
        handler::<D>(info).can_delete_file(handle::<D>(info, file_name)?)
    })
}

extern "system" fn delete_directory<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("delete-dir", file_name, || unsafe {
        handler::<D>(info).can_delete_directory(handle::<D>(info, file_name)?)
    })
}

extern "system" fn move_file<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    new_file_name: LPCWSTR,
    replace_if_existing: BOOL,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("move", file_name, || unsafe {
        let (from, to) = (path(file_name)?, path(new_file_name)?);
        handler::<D>(info).rename(&from, &to, replace_if_existing != 0)
    })
}

extern "system" fn set_end_of_file<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    offset: i64,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("end-of-file", file_name, || unsafe {
        handler::<D>(info).set_end_of_file(handle::<D>(info, file_name)?, offset)
    })
}

extern "system" fn set_allocation_size<D: BlockDevice + Send>(
    file_name: LPCWSTR,
    size: i64,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("allocation", file_name, || unsafe {
        handler::<D>(info).set_allocation_size(handle::<D>(info, file_name)?, size)
    })
}

extern "system" fn get_disk_free_space<D: BlockDevice + Send>(
    available: PULONGLONG,
    total: PULONGLONG,
    free: PULONGLONG,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("space", ptr::null(), || unsafe {
        let space = handler::<D>(info).space()?;
        put(available, space.free);
        put(total, space.total);
        put(free, space.free);
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
extern "system" fn get_volume_information<D: BlockDevice + Send>(
    name: LPWSTR,
    name_size: DWORD,
    serial: LPDWORD,
    max_component_length: LPDWORD,
    fs_flags: LPDWORD,
    fs_name: LPWSTR,
    fs_name_size: DWORD,
    info: PDOKAN_FILE_INFO,
) -> NTSTATUS {
    run("volume", ptr::null(), || unsafe {
        let handler = handler::<D>(info);
        copy_wide(&handler.name, name, name_size);
        put(serial, nt::serial(&handler.name));
        put(max_component_length, NAME_MAX as DWORD);
        put(fs_flags, handler.fs_flags());
        copy_wide("LFS", fs_name, fs_name_size);
        Ok(())
    })
}

extern "system" fn mounted(mount_point: LPCWSTR, _info: PDOKAN_FILE_INFO) -> NTSTATUS {
    run("mounted", mount_point, || Ok(()))
}

extern "system" fn unmounted(_info: PDOKAN_FILE_INFO) -> NTSTATUS {
    run("unmounted", ptr::null(), || Ok(()))
}

/// A callback's address in [`Operations`] (null for Dokan's default).
type Callback = *const ();

/// `DOKAN_OPERATIONS`, field for field, filled with `extern "system"`
/// callbacks: `dokan-sys` types them `extern "stdcall"`, which rustc is
/// phasing out on x64. Both name the C convention there, and `system` is
/// `stdcall` on x86.
#[repr(C)]
struct Operations {
    zw_create_file: Callback,
    cleanup: Callback,
    close_file: Callback,
    read_file: Callback,
    write_file: Callback,
    flush_file_buffers: Callback,
    get_file_information: Callback,
    find_files: Callback,
    find_files_with_pattern: Callback,
    set_file_attributes: Callback,
    set_file_time: Callback,
    delete_file: Callback,
    delete_directory: Callback,
    move_file: Callback,
    set_end_of_file: Callback,
    set_allocation_size: Callback,
    lock_file: Callback,
    unlock_file: Callback,
    get_disk_free_space: Callback,
    get_volume_information: Callback,
    mounted: Callback,
    unmounted: Callback,
    get_file_security: Callback,
    set_file_security: Callback,
    find_streams: Callback,
}

const _: () = assert!(size_of::<Operations>() == size_of::<DOKAN_OPERATIONS>());

/// The callbacks; the rest (streams, security, locks, patterns) are left
/// to Dokan's defaults.
fn operations<D: BlockDevice + Send>() -> Operations {
    Operations {
        zw_create_file: create_file::<D> as Callback,
        cleanup: cleanup::<D> as Callback,
        close_file: close_file::<D> as Callback,
        read_file: read_file::<D> as Callback,
        write_file: write_file::<D> as Callback,
        flush_file_buffers: flush_file_buffers::<D> as Callback,
        get_file_information: get_file_information::<D> as Callback,
        find_files: find_files::<D> as Callback,
        find_files_with_pattern: ptr::null(),
        set_file_attributes: set_file_attributes::<D> as Callback,
        set_file_time: set_file_time::<D> as Callback,
        delete_file: delete_file::<D> as Callback,
        delete_directory: delete_directory::<D> as Callback,
        move_file: move_file::<D> as Callback,
        set_end_of_file: set_end_of_file::<D> as Callback,
        set_allocation_size: set_allocation_size::<D> as Callback,
        lock_file: ptr::null(),
        unlock_file: ptr::null(),
        get_disk_free_space: get_disk_free_space::<D> as Callback,
        get_volume_information: get_volume_information::<D> as Callback,
        mounted: mounted as Callback,
        unmounted: unmounted as Callback,
        get_file_security: ptr::null(),
        set_file_security: ptr::null(),
        find_streams: ptr::null(),
    }
}

/// What a `DokanCreateFileSystem` error code means.
const fn mount_error(code: i32) -> &'static str {
    match code {
        dokan_sys::DOKAN_DRIVE_LETTER_ERROR => "bad drive letter",
        dokan_sys::DOKAN_DRIVER_INSTALL_ERROR => "the Dokan driver is not installed",
        dokan_sys::DOKAN_START_ERROR => "the Dokan driver failed to start",
        dokan_sys::DOKAN_MOUNT_ERROR => "cannot assign the drive letter or mount point",
        dokan_sys::DOKAN_MOUNT_POINT_ERROR => "the mount point is invalid",
        dokan_sys::DOKAN_VERSION_ERROR => "the installed Dokan library is incompatible",
        _ => "general error",
    }
}

/// Mounts `handler` at `mount_point`, signals `ready` and serves requests
/// until the volume is unmounted, then commits and closes it.
pub(super) fn serve<D: BlockDevice + Send>(
    handler: &Handler<D>,
    mount_point: &U16CStr,
    flags: u32,
    allocation_unit_size: u32,
    ready: &mpsc::Sender<()>,
) -> Result<()> {
    // Dokan keeps pointers to both until the file system is closed.
    let mut options = Box::new(DOKAN_OPTIONS {
        Version: u16::try_from(dokan_sys::DOKAN_VERSION).unwrap_or(u16::MAX),
        SingleThread: 0,
        Options: flags,
        GlobalContext: ptr::from_ref(handler) as usize as u64,
        MountPoint: mount_point.as_ptr(),
        UNCName: ptr::null(),
        Timeout: 0,
        AllocationUnitSize: allocation_unit_size,
        SectorSize: 0,
        VolumeSecurityDescriptorLength: 0,
        VolumeSecurityDescriptor: [0; VOLUME_SECURITY_DESCRIPTOR_MAX_SIZE],
    });
    let mut operations = operations::<D>();
    let mut instance: DOKAN_HANDLE = ptr::null_mut();
    // SAFETY: `options`, `operations`, `handler` and `mount_point` outlive
    // the file system, which is closed before this function returns.
    let result = unsafe {
        DokanInit();
        let code = DokanCreateFileSystem(
            &raw mut *options,
            ptr::from_mut(&mut operations).cast::<DOKAN_OPERATIONS>(),
            &raw mut instance,
        );
        let result = if code == dokan_sys::DOKAN_SUCCESS {
            // Ignored when `spawn` stopped waiting.
            let _ = ready.send(());
            DokanWaitForFileSystemClosed(instance, INFINITE);
            DokanCloseHandle(instance);
            Ok(())
        } else {
            Err(Error::Io(std::io::Error::other(format!(
                "Dokan: {} ({code})",
                mount_error(code)
            ))))
        };
        DokanShutdown();
        result
    };
    let closed = handler.volume.close();
    result.and(closed)
}

/// Asks Dokan to unmount the volume at `mount_point` (nothing happens when
/// it is already unmounted).
pub(super) fn unmount(mount_point: &U16CStr) {
    // SAFETY: a NUL-terminated string.
    unsafe {
        DokanRemoveMountPoint(mount_point.as_ptr());
    }
}
