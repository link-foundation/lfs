//! Windows (NT) semantics for the Dokan driver: status codes, paths, file
//! attributes and `CreateFile` dispositions. Kept free of Windows types so
//! the rules are tested on every platform.

use crate::error::Error;
use crate::fs::{Attr, FileKind};

/// An `NTSTATUS` value.
pub type Status = i32;

/// `STATUS_SUCCESS`.
pub const STATUS_SUCCESS: Status = 0;
// The `as` casts reinterpret the documented unsigned codes as `NTSTATUS`.
/// `STATUS_BUFFER_OVERFLOW` (a warning: the buffer took what fit).
pub const STATUS_BUFFER_OVERFLOW: Status = 0x8000_0005_u32 as i32;
/// `STATUS_NOT_IMPLEMENTED`.
pub const STATUS_NOT_IMPLEMENTED: Status = 0xC000_0002_u32 as i32;
/// `STATUS_INVALID_PARAMETER`.
pub const STATUS_INVALID_PARAMETER: Status = 0xC000_000D_u32 as i32;
/// `STATUS_ACCESS_DENIED`.
pub const STATUS_ACCESS_DENIED: Status = 0xC000_0022_u32 as i32;
/// `STATUS_OBJECT_NAME_INVALID`.
pub const STATUS_OBJECT_NAME_INVALID: Status = 0xC000_0033_u32 as i32;
/// `STATUS_OBJECT_NAME_NOT_FOUND`.
pub const STATUS_OBJECT_NAME_NOT_FOUND: Status = 0xC000_0034_u32 as i32;
/// `STATUS_OBJECT_NAME_COLLISION`.
pub const STATUS_OBJECT_NAME_COLLISION: Status = 0xC000_0035_u32 as i32;
/// `STATUS_OBJECT_PATH_NOT_FOUND`.
pub const STATUS_OBJECT_PATH_NOT_FOUND: Status = 0xC000_003A_u32 as i32;
/// `STATUS_DISK_FULL`.
pub const STATUS_DISK_FULL: Status = 0xC000_007F_u32 as i32;
/// `STATUS_MEDIA_WRITE_PROTECTED`.
pub const STATUS_MEDIA_WRITE_PROTECTED: Status = 0xC000_00A2_u32 as i32;
/// `STATUS_FILE_IS_A_DIRECTORY`.
pub const STATUS_FILE_IS_A_DIRECTORY: Status = 0xC000_00BA_u32 as i32;
/// `STATUS_NOT_SUPPORTED`.
pub const STATUS_NOT_SUPPORTED: Status = 0xC000_00BB_u32 as i32;
/// `STATUS_DIRECTORY_NOT_EMPTY`.
pub const STATUS_DIRECTORY_NOT_EMPTY: Status = 0xC000_0101_u32 as i32;
/// `STATUS_FILE_CORRUPT_ERROR`.
pub const STATUS_FILE_CORRUPT_ERROR: Status = 0xC000_0102_u32 as i32;
/// `STATUS_NOT_A_DIRECTORY`.
pub const STATUS_NOT_A_DIRECTORY: Status = 0xC000_0103_u32 as i32;
/// `STATUS_NAME_TOO_LONG`.
pub const STATUS_NAME_TOO_LONG: Status = 0xC000_0106_u32 as i32;
/// `STATUS_SHARING_VIOLATION`.
pub const STATUS_SHARING_VIOLATION: Status = 0xC000_0043_u32 as i32;
/// `STATUS_INTERNAL_ERROR`.
pub const STATUS_INTERNAL_ERROR: Status = 0xC000_00E5_u32 as i32;
/// `STATUS_IO_DEVICE_ERROR`.
pub const STATUS_IO_DEVICE_ERROR: Status = 0xC000_0185_u32 as i32;

/// `FILE_ATTRIBUTE_READONLY`.
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
/// `FILE_ATTRIBUTE_HIDDEN`.
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
/// `FILE_ATTRIBUTE_ARCHIVE`.
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
/// `FILE_ATTRIBUTE_NORMAL`.
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

/// `FILE_SUPERSEDE`: replace an existing file or create one.
pub const FILE_SUPERSEDE: u32 = 0;
/// `FILE_OPEN`: open an existing file.
pub const FILE_OPEN: u32 = 1;
/// `FILE_CREATE`: create a file that must not exist.
pub const FILE_CREATE: u32 = 2;
/// `FILE_OPEN_IF`: open or create.
pub const FILE_OPEN_IF: u32 = 3;
/// `FILE_OVERWRITE`: truncate an existing file.
pub const FILE_OVERWRITE: u32 = 4;
/// `FILE_OVERWRITE_IF`: truncate or create.
pub const FILE_OVERWRITE_IF: u32 = 5;

/// `FILE_DIRECTORY_FILE` create option.
pub const FILE_DIRECTORY_FILE: u32 = 0x1;
/// `FILE_NON_DIRECTORY_FILE` create option.
pub const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
/// `FILE_DELETE_ON_CLOSE` create option.
pub const FILE_DELETE_ON_CLOSE: u32 = 0x1000;

/// `FILE_CASE_SENSITIVE_SEARCH` volume flag.
pub const FILE_CASE_SENSITIVE_SEARCH: u32 = 0x1;
/// `FILE_CASE_PRESERVED_NAMES` volume flag.
pub const FILE_CASE_PRESERVED_NAMES: u32 = 0x2;
/// `FILE_UNICODE_ON_DISK` volume flag.
pub const FILE_UNICODE_ON_DISK: u32 = 0x4;
/// `FILE_READ_ONLY_VOLUME` volume flag.
pub const FILE_READ_ONLY_VOLUME: u32 = 0x8_0000;

/// The `NTSTATUS` that reports `error`.
#[must_use]
pub const fn status(error: &Error) -> Status {
    match error {
        Error::Io(_) => STATUS_IO_DEVICE_ERROR,
        Error::Corrupt(_) => STATUS_FILE_CORRUPT_ERROR,
        Error::NotFound => STATUS_OBJECT_NAME_NOT_FOUND,
        Error::Exists => STATUS_OBJECT_NAME_COLLISION,
        Error::NotDir => STATUS_NOT_A_DIRECTORY,
        Error::IsDir => STATUS_FILE_IS_A_DIRECTORY,
        Error::NotEmpty => STATUS_DIRECTORY_NOT_EMPTY,
        Error::NoSpace => STATUS_DISK_FULL,
        Error::InvalidArgument(_) => STATUS_INVALID_PARAMETER,
        Error::Unsupported(_) => STATUS_NOT_SUPPORTED,
        Error::NameTooLong => STATUS_NAME_TOO_LONG,
        Error::ReadOnly => STATUS_MEDIA_WRITE_PROTECTED,
        Error::Busy(_) => STATUS_SHARING_VIOLATION,
    }
}

/// Turns a Dokan path (`\dir\file`) into a volume path (`/dir/file`).
/// Names holding `/` cannot exist on the volume, and `:` would address an
/// NTFS stream, so both are refused.
pub fn path(windows: &str) -> Result<String, Status> {
    if windows.contains(['/', ':']) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    let path = windows.replace('\\', "/");
    Ok(if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    })
}

/// The Windows attributes of a file: directories are directories, a file
/// without any write permission is read-only and names starting with a
/// dot are hidden, as on Unix.
#[must_use]
pub fn attributes(attr: &Attr, name: &str) -> u32 {
    let mut attributes = match attr.kind {
        FileKind::Dir => FILE_ATTRIBUTE_DIRECTORY,
        FileKind::File => FILE_ATTRIBUTE_ARCHIVE,
        // Dokan cannot present Unix symbolic links; they read as small
        // read-only files holding the target path.
        FileKind::Symlink => FILE_ATTRIBUTE_READONLY,
    };
    if attr.perm & 0o222 == 0 {
        attributes |= FILE_ATTRIBUTE_READONLY;
    }
    if name.starts_with('.') && name != "." && name != ".." {
        attributes |= FILE_ATTRIBUTE_HIDDEN;
    }
    if attributes == 0 {
        FILE_ATTRIBUTE_NORMAL
    } else {
        attributes
    }
}

/// The permission bits after `SetFileAttributes`: the read-only attribute
/// clears every write bit, clearing it gives the owner write access back.
#[must_use]
pub const fn permissions(perm: u16, attributes: u32) -> u16 {
    if attributes & FILE_ATTRIBUTE_READONLY != 0 {
        perm & !0o222
    } else if perm & 0o222 == 0 {
        perm | 0o200
    } else {
        perm
    }
}

/// What `CreateFile` does with an existing or missing name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the existing file or directory.
    Open,
    /// Open the existing file and truncate it.
    Truncate,
    /// Create a file.
    CreateFile,
    /// Create a directory.
    CreateDir,
}

/// Decides what `CreateFile` does. `existing` is `Some(is_dir)` when the
/// name exists.
pub const fn action(
    disposition: u32,
    options: u32,
    existing: Option<bool>,
) -> Result<Action, Status> {
    let wants_dir = options & FILE_DIRECTORY_FILE != 0;
    let wants_file = options & FILE_NON_DIRECTORY_FILE != 0;
    if disposition > FILE_OVERWRITE_IF || (wants_dir && wants_file) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    match existing {
        Some(true) if wants_file => Err(STATUS_FILE_IS_A_DIRECTORY),
        Some(false) if wants_dir => Err(STATUS_NOT_A_DIRECTORY),
        Some(_) if disposition == FILE_CREATE => Err(STATUS_OBJECT_NAME_COLLISION),
        Some(_) if disposition == FILE_OPEN || disposition == FILE_OPEN_IF => Ok(Action::Open),
        // Directories cannot be overwritten.
        Some(true) => Err(STATUS_INVALID_PARAMETER),
        Some(false) => Ok(Action::Truncate),
        None if disposition == FILE_OPEN || disposition == FILE_OVERWRITE => {
            Err(STATUS_OBJECT_NAME_NOT_FOUND)
        }
        None if wants_dir => {
            if disposition == FILE_CREATE || disposition == FILE_OPEN_IF {
                Ok(Action::CreateDir)
            } else {
                Err(STATUS_INVALID_PARAMETER)
            }
        }
        None => Ok(Action::CreateFile),
    }
}

/// Whether `CreateFile` reports an existing name as a collision.
///
/// With these dispositions `STATUS_OBJECT_NAME_COLLISION` means "opened
/// a name that already existed". Dokan counts that status
/// as a successful open for these dispositions (it is how `OPEN_ALWAYS`
/// and `CREATE_ALWAYS` set `ERROR_ALREADY_EXISTS`).
#[must_use]
pub const fn reports_existing(disposition: u32) -> bool {
    matches!(
        disposition,
        FILE_SUPERSEDE | FILE_OPEN_IF | FILE_OVERWRITE_IF
    )
}

/// The status `CreateFile` returns.
///
/// An open that succeeded is
/// `STATUS_SUCCESS`, or `STATUS_OBJECT_NAME_COLLISION` when
/// [`reports_existing`] says so and nothing was created. A failure that
/// happens to be a collision becomes a sharing violation for those
/// dispositions, since Dokan would take it for an open and hand the next
/// requests an empty context.
#[must_use]
pub const fn create_status(disposition: u32, outcome: Result<bool, Status>) -> Status {
    match outcome {
        Ok(false) if reports_existing(disposition) => STATUS_OBJECT_NAME_COLLISION,
        Ok(_) => STATUS_SUCCESS,
        Err(STATUS_OBJECT_NAME_COLLISION) if reports_existing(disposition) => {
            STATUS_SHARING_VIOLATION
        }
        Err(status) => status,
    }
}

/// 100 ns intervals from 1601-01-01 (the `FILETIME` epoch) to 1970-01-01.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// A `FILETIME` (100 ns intervals since 1601) for nanoseconds since the
/// Unix epoch.
#[must_use]
pub const fn filetime(nanos: i64) -> u64 {
    // `i64` nanoseconds reach back to 1677, so the sum is positive.
    (nanos.div_euclid(100) + UNIX_EPOCH_TICKS).unsigned_abs()
}

/// The time a `SetFileTime` `FILETIME` asks for, in nanoseconds since the
/// Unix epoch. 0 (keep), `-1` (stop updating) and `-2` (resume updating)
/// change nothing.
#[must_use]
pub const fn set_time(filetime: u64) -> Option<i64> {
    // The special values are negative when read as signed.
    let ticks = filetime.cast_signed();
    if ticks <= 0 {
        None
    } else {
        Some((ticks - UNIX_EPOCH_TICKS).saturating_mul(100))
    }
}

/// A volume serial number derived from the volume name (FNV-1a).
#[must_use]
pub fn serial(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5, |hash: u32, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(kind: FileKind, perm: u16) -> Attr {
        Attr {
            ino: 2,
            kind,
            perm,
            nlink: 1,
            uid: 0,
            gid: 0,
            size: 0,
            allocated: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
        }
    }

    #[test]
    fn paths_use_forward_slashes() {
        assert_eq!(path("\\").unwrap(), "/");
        assert_eq!(path("\\a\\b c.txt").unwrap(), "/a/b c.txt");
        assert_eq!(path("a").unwrap(), "/a");
        assert_eq!(path("\\a/b"), Err(STATUS_OBJECT_NAME_INVALID));
        assert_eq!(path("\\a:stream"), Err(STATUS_OBJECT_NAME_INVALID));
    }

    #[test]
    fn errors_map_to_statuses() {
        assert_eq!(status(&Error::NotFound), STATUS_OBJECT_NAME_NOT_FOUND);
        assert_eq!(status(&Error::NotEmpty), STATUS_DIRECTORY_NOT_EMPTY);
        assert_eq!(status(&Error::ReadOnly), STATUS_MEDIA_WRITE_PROTECTED);
        assert_eq!(status(&Error::NoSpace), STATUS_DISK_FULL);
        assert_eq!(STATUS_OBJECT_NAME_NOT_FOUND, -1_073_741_772);
    }

    #[test]
    fn attributes_follow_kind_permissions_and_dot_names() {
        assert_eq!(
            attributes(&attr(FileKind::Dir, 0o755), "d"),
            FILE_ATTRIBUTE_DIRECTORY
        );
        assert_eq!(
            attributes(&attr(FileKind::File, 0o644), "f"),
            FILE_ATTRIBUTE_ARCHIVE
        );
        assert_eq!(
            attributes(&attr(FileKind::File, 0o444), ".f"),
            FILE_ATTRIBUTE_ARCHIVE | FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN
        );
        assert_eq!(
            attributes(&attr(FileKind::Symlink, 0o777), "l"),
            FILE_ATTRIBUTE_READONLY
        );
        assert_eq!(
            attributes(&attr(FileKind::Dir, 0o755), ".."),
            FILE_ATTRIBUTE_DIRECTORY
        );
    }

    #[test]
    fn the_read_only_attribute_toggles_write_bits() {
        assert_eq!(permissions(0o664, FILE_ATTRIBUTE_READONLY), 0o444);
        assert_eq!(permissions(0o444, FILE_ATTRIBUTE_NORMAL), 0o644);
        assert_eq!(permissions(0o640, FILE_ATTRIBUTE_ARCHIVE), 0o640);
    }

    #[test]
    fn dispositions_follow_create_file() {
        let file = Some(false);
        let dir = Some(true);
        assert_eq!(action(FILE_OPEN, 0, file), Ok(Action::Open));
        assert_eq!(action(FILE_OPEN, 0, dir), Ok(Action::Open));
        assert_eq!(
            action(FILE_OPEN, 0, None),
            Err(STATUS_OBJECT_NAME_NOT_FOUND)
        );
        assert_eq!(
            action(FILE_CREATE, 0, file),
            Err(STATUS_OBJECT_NAME_COLLISION)
        );
        assert_eq!(action(FILE_CREATE, 0, None), Ok(Action::CreateFile));
        assert_eq!(
            action(FILE_CREATE, FILE_DIRECTORY_FILE, None),
            Ok(Action::CreateDir)
        );
        assert_eq!(action(FILE_OPEN_IF, 0, file), Ok(Action::Open));
        assert_eq!(action(FILE_OPEN_IF, 0, None), Ok(Action::CreateFile));
        assert_eq!(action(FILE_OVERWRITE, 0, file), Ok(Action::Truncate));
        assert_eq!(
            action(FILE_OVERWRITE, 0, None),
            Err(STATUS_OBJECT_NAME_NOT_FOUND)
        );
        assert_eq!(action(FILE_OVERWRITE_IF, 0, None), Ok(Action::CreateFile));
        assert_eq!(action(FILE_SUPERSEDE, 0, file), Ok(Action::Truncate));
        assert_eq!(
            action(FILE_SUPERSEDE, 0, dir),
            Err(STATUS_INVALID_PARAMETER)
        );
        assert_eq!(
            action(FILE_OPEN, FILE_NON_DIRECTORY_FILE, dir),
            Err(STATUS_FILE_IS_A_DIRECTORY)
        );
        assert_eq!(
            action(FILE_OPEN, FILE_DIRECTORY_FILE, file),
            Err(STATUS_NOT_A_DIRECTORY)
        );
        assert_eq!(
            action(FILE_OVERWRITE_IF, FILE_DIRECTORY_FILE, None),
            Err(STATUS_INVALID_PARAMETER)
        );
        assert_eq!(action(6, 0, None), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(
            action(
                FILE_OPEN,
                FILE_DIRECTORY_FILE | FILE_NON_DIRECTORY_FILE,
                dir
            ),
            Err(STATUS_INVALID_PARAMETER)
        );
    }

    #[test]
    fn create_reports_existing_names_the_way_dokan_expects() {
        let collision = STATUS_OBJECT_NAME_COLLISION;
        assert_eq!(create_status(FILE_OPEN_IF, Ok(false)), collision);
        assert_eq!(create_status(FILE_OVERWRITE_IF, Ok(false)), collision);
        assert_eq!(create_status(FILE_SUPERSEDE, Ok(false)), collision);
        assert_eq!(create_status(FILE_OPEN_IF, Ok(true)), STATUS_SUCCESS);
        assert_eq!(create_status(FILE_OPEN, Ok(false)), STATUS_SUCCESS);
        assert_eq!(create_status(FILE_CREATE, Ok(true)), STATUS_SUCCESS);
        // A real collision must not read as an open.
        assert_eq!(
            create_status(FILE_OPEN_IF, Err(collision)),
            STATUS_SHARING_VIOLATION
        );
        assert_eq!(create_status(FILE_CREATE, Err(collision)), collision);
        assert_eq!(
            create_status(FILE_OPEN, Err(STATUS_OBJECT_NAME_NOT_FOUND)),
            STATUS_OBJECT_NAME_NOT_FOUND
        );
    }

    #[test]
    fn filetimes_round_trip_at_100_ns() {
        assert_eq!(filetime(0), 116_444_736_000_000_000);
        // 2023-11-14T22:13:20Z, a known FILETIME.
        assert_eq!(filetime(1_700_000_000_000_000_000), 133_444_736_000_000_000);
        for nanos in [
            0,
            100,
            1_700_000_000_123_456_700,
            -86_400_000_000_000,
            i64::MIN / 100 * 100,
            i64::MAX / 100 * 100,
        ] {
            assert_eq!(set_time(filetime(nanos)), Some(nanos));
        }
        assert_eq!(
            set_time(filetime(1_700_000_000_000_000_099)),
            Some(1_700_000_000_000_000_000)
        );
        assert_eq!(set_time(0), None);
        assert_eq!(set_time(u64::MAX), None);
        assert_eq!(set_time(u64::MAX - 1), None);
    }

    #[test]
    fn serial_numbers_follow_the_name() {
        assert_eq!(serial(""), 0x811c_9dc5);
        assert_ne!(serial("lfs"), serial("lfs2"));
    }
}
