//! Stands in when this build has no mount driver: mounting reports how to
//! get one, while the rest of the library works as usual.

use super::MountOptions;
use crate::device::BlockDevice;
use crate::error::{Error, Result};
use crate::fs::Fs;
use std::path::Path;

/// A mounted volume (`spawn` never makes one in this build).
pub struct Mounted(());

impl Mounted {
    /// Whether the volume is still mounted.
    #[must_use]
    pub const fn is_mounted(&self) -> bool {
        false
    }

    /// Unmounts the volume.
    #[allow(clippy::unnecessary_wraps)]
    pub const fn unmount(self) -> Result<()> {
        Ok(())
    }

    /// Waits until the volume is unmounted.
    #[allow(clippy::unnecessary_wraps)]
    pub const fn wait(self) -> Result<()> {
        Ok(())
    }
}

/// Fails: this build has no mount driver.
pub fn spawn<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    mountpoint: &Path,
    options: &MountOptions,
) -> Result<Mounted> {
    let _ = (fs, mountpoint, options);
    Err(Error::Unsupported(
        "this build has no mount driver; rebuild with `--features fuse` (Linux, \
         FreeBSD, macOS with macFUSE) or `--features dokan` (Windows x64 with \
         Dokan 2)"
            .into(),
    ))
}

/// Fails: this build has no mount driver.
pub fn mount<D: BlockDevice + Send + 'static>(
    fs: Fs<D>,
    mountpoint: &Path,
    options: &MountOptions,
) -> Result<()> {
    spawn(fs, mountpoint, options)?.wait()
}
