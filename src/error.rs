//! Error type shared by every layer of the file system.

use std::fmt;

/// Errors returned by LFS operations.
#[derive(Debug)]
pub enum Error {
    /// The underlying device failed.
    Io(std::io::Error),
    /// On-disk structures are inconsistent.
    Corrupt(String),
    /// No such file, directory or link.
    NotFound,
    /// The name or link already exists.
    Exists,
    /// A directory was expected.
    NotDir,
    /// A non-directory was expected.
    IsDir,
    /// The directory is not empty.
    NotEmpty,
    /// No contiguous space is left, even after compaction.
    NoSpace,
    /// The argument is invalid.
    InvalidArgument(String),
    /// The operation is not supported by this format or configuration.
    Unsupported(String),
    /// The name is longer than the format allows.
    NameTooLong,
    /// The file system is mounted read-only.
    ReadOnly,
    /// The link is in use and cannot be deleted or changed.
    Busy(String),
}

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Builds a [`Error::Corrupt`] error.
    pub fn corrupt(message: impl Into<String>) -> Self {
        Self::Corrupt(message.into())
    }

    /// Builds a [`Error::InvalidArgument`] error.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidArgument(message.into())
    }

    /// POSIX errno for this error (Linux numbering; macOS differs only for
    /// `ENOTEMPTY`, which is handled here too).
    #[must_use]
    pub const fn errno(&self) -> i32 {
        match self {
            Self::Io(_) | Self::Corrupt(_) => 5,
            Self::NotFound => 2,
            Self::Exists => 17,
            Self::NotDir => 20,
            Self::IsDir => 21,
            Self::NotEmpty => {
                if cfg!(any(target_os = "macos", target_os = "freebsd")) {
                    66
                } else {
                    39
                }
            }
            Self::NoSpace => 28,
            Self::InvalidArgument(_) => 22,
            Self::Unsupported(_) => {
                if cfg!(any(target_os = "macos", target_os = "freebsd")) {
                    78
                } else {
                    38
                }
            }
            Self::NameTooLong => {
                if cfg!(any(target_os = "macos", target_os = "freebsd")) {
                    63
                } else {
                    36
                }
            }
            Self::ReadOnly => 30,
            Self::Busy(_) => 16,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Corrupt(message) => write!(f, "corrupt file system: {message}"),
            Self::NotFound => f.write_str("not found"),
            Self::Exists => f.write_str("already exists"),
            Self::NotDir => f.write_str("not a directory"),
            Self::IsDir => f.write_str("is a directory"),
            Self::NotEmpty => f.write_str("directory not empty"),
            Self::NoSpace => f.write_str("no space left on device"),
            Self::InvalidArgument(message) => write!(f, "invalid argument: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::NameTooLong => f.write_str("name too long"),
            Self::ReadOnly => f.write_str("read-only file system"),
            Self::Busy(message) => write!(f, "busy: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
