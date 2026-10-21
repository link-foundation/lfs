//! Mounting volumes as native file systems.
//!
//! FUSE serves Linux, macOS and FreeBSD (`fuse` feature) and Dokan serves
//! Windows x64 (`dokan` feature). Builds without either still export
//! [`spawn`] and [`mount`], which then report how to get a driver.
//!
//! Both drivers serve one [`Fs`] behind a mutex, commit it on `fsync`, on
//! `commit-every` and when the volume is unmounted, and answer a request
//! that panics with an I/O error instead of taking the mount down.

#[cfg(all(
    feature = "dokan",
    windows,
    any(target_arch = "x86_64", target_arch = "x86")
))]
mod dokan;
#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "macos", target_os = "freebsd")
))]
mod fuse;
#[cfg(not(any(
    all(
        feature = "dokan",
        windows,
        any(target_arch = "x86_64", target_arch = "x86")
    ),
    all(
        feature = "fuse",
        any(target_os = "linux", target_os = "macos", target_os = "freebsd")
    )
)))]
mod none;
pub mod nt;

#[cfg(all(
    feature = "dokan",
    windows,
    any(target_arch = "x86_64", target_arch = "x86")
))]
pub use self::dokan::{mount, spawn, Mounted};
#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "macos", target_os = "freebsd")
))]
pub use self::fuse::{mount, spawn, Mounted};
#[cfg(not(any(
    all(
        feature = "dokan",
        windows,
        any(target_arch = "x86_64", target_arch = "x86")
    ),
    all(
        feature = "fuse",
        any(target_os = "linux", target_os = "macos", target_os = "freebsd")
    )
)))]
pub use self::none::{mount, spawn, Mounted};

/// The mount driver in this build: `"fuse"`, `"dokan"` or `"none"`.
pub const DRIVER: &str = if cfg!(all(
    feature = "dokan",
    windows,
    any(target_arch = "x86_64", target_arch = "x86")
)) {
    "dokan"
} else if cfg!(all(
    feature = "fuse",
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd"
    )
)) {
    "fuse"
} else {
    "none"
};

use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::fs::Fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How to mount a volume.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountOptions {
    /// Refuse every change.
    pub read_only: bool,
    /// Let other users use the mount (FUSE `allow_other`).
    pub allow_other: bool,
    /// Volume name shown by the operating system.
    pub name: String,
}

/// A volume shared by the threads of a driver (also usable to write
/// drivers for other kernels).
pub struct Shared<D: BlockDevice> {
    fs: Mutex<Option<Fs<D>>>,
}

impl<D: BlockDevice> Shared<D> {
    /// Wraps an open volume.
    #[must_use]
    pub const fn new(fs: Fs<D>) -> Self {
        Self {
            fs: Mutex::new(Some(fs)),
        }
    }

    /// Runs `operation` on the volume. A panic becomes [`Error::Corrupt`]
    /// and leaves the volume closed to further requests, so nothing half
    /// done is ever committed.
    pub fn with<T>(&self, operation: impl FnOnce(&mut Fs<D>) -> Result<T>) -> Result<T> {
        let mut guard = self.fs.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(fs) = guard.as_mut() else {
            return Err(Error::corrupt("the volume was closed after a failure"));
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| operation(fs)));
        if outcome.is_err() {
            // Drop the volume without committing: the device keeps the last
            // committed state.
            guard.take();
        }
        drop(guard);
        outcome.unwrap_or_else(|_| Err(Error::corrupt("an operation failed unexpectedly")))
    }

    /// Commits and closes the volume (once).
    pub fn close(&self) -> Result<()> {
        let fs = self
            .fs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        fs.map_or(Ok(()), |fs| fs.close().map(drop))
    }
}

/// Splits a mode into permission bits after applying `umask`.
#[must_use]
pub const fn permissions(mode: u32, umask: u32) -> u16 {
    (mode & !umask & 0o7777) as u16
}

/// Converts nanoseconds since the Unix epoch (the volume's clock) into a
/// [`SystemTime`].
#[must_use]
pub fn system_time(nanos: i64) -> SystemTime {
    let span = Duration::from_nanos(nanos.unsigned_abs());
    if nanos >= 0 {
        UNIX_EPOCH + span
    } else {
        UNIX_EPOCH - span
    }
}

/// Converts a [`SystemTime`] into nanoseconds since the Unix epoch
/// (saturating outside ±292 years).
#[must_use]
pub fn nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos()).map_or(i64::MIN, |n| -n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FormatOptions;
    use crate::device::MemDevice;
    use crate::fs::{Creator, ROOT_INO};

    fn shared() -> Shared<MemDevice> {
        Shared::new(Fs::format(MemDevice::new(4 << 20), &FormatOptions::default(), "t").unwrap())
    }

    #[test]
    fn operations_run_on_the_volume() {
        let volume = shared();
        let ino = volume
            .with(|fs| fs.create(ROOT_INO, "a", 0o644, Creator::default()))
            .unwrap()
            .ino;
        assert_eq!(volume.with(|fs| fs.lookup(ROOT_INO, "a")).unwrap().ino, ino);
        assert!(matches!(
            volume.with(|fs| fs.lookup(ROOT_INO, "b")),
            Err(Error::NotFound)
        ));
        volume.close().unwrap();
        volume.close().unwrap();
    }

    #[test]
    fn a_panic_closes_the_volume_without_committing() {
        let volume = shared();
        let result: Result<()> = volume.with(|_| panic!("boom"));
        assert!(matches!(result, Err(Error::Corrupt(_))));
        assert!(matches!(
            volume.with(|fs| fs.getattr(ROOT_INO)),
            Err(Error::Corrupt(_))
        ));
        volume.close().unwrap();
    }

    #[test]
    fn times_round_trip() {
        for value in [0, 1, 1_700_000_000_123_456_789, -86_400_000_000_123] {
            assert_eq!(nanos(system_time(value)), value);
        }
    }

    #[test]
    fn permissions_apply_the_umask() {
        assert_eq!(permissions(0o100_666, 0o022), 0o644);
        assert_eq!(permissions(0o40_777, 0o077), 0o700);
        assert_eq!(permissions(0o4755, 0), 0o4755);
    }
}
