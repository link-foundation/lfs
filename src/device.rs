//! Block devices: where an LFS image lives.
//!
//! The file system addresses the device in whole blocks. [`FileDevice`] works
//! with regular image files and raw devices (`/dev/sdb1`, `\\.\E:`) on every
//! platform, [`MemDevice`] keeps the image in memory (tests, the web
//! visualizer) and [`CrashDevice`] simulates power loss for journal tests.

use crate::error::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// A fixed-size, block-addressed storage device.
pub trait BlockDevice {
    /// Size of the device in bytes.
    fn len_bytes(&self) -> u64;
    /// Reads `buf.len()` bytes starting at byte `offset`.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()>;
    /// Writes `buf` starting at byte `offset`.
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()>;
    /// Makes every completed write durable.
    fn sync(&mut self) -> Result<()>;
}

impl<T: BlockDevice + ?Sized> BlockDevice for Box<T> {
    fn len_bytes(&self) -> u64 {
        (**self).len_bytes()
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        (**self).read_at(offset, buf)
    }
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        (**self).write_at(offset, buf)
    }
    fn sync(&mut self) -> Result<()> {
        (**self).sync()
    }
}

fn check_range(len: u64, offset: u64, size: usize) -> Result<()> {
    match offset.checked_add(size as u64) {
        Some(end) if end <= len => Ok(()),
        _ => Err(Error::invalid(format!(
            "access {offset}+{size} is outside the {len}-byte device"
        ))),
    }
}

/// An in-memory device.
#[derive(Clone, Debug, Default)]
pub struct MemDevice {
    bytes: Vec<u8>,
}

impl MemDevice {
    /// A zero-filled device of `len` bytes.
    #[must_use]
    pub fn new(len: u64) -> Self {
        Self {
            bytes: vec![0; len as usize],
        }
    }

    /// Wraps an existing image.
    #[must_use]
    pub const fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The raw image.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the device and returns the raw image.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl BlockDevice for MemDevice {
    fn len_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        check_range(self.len_bytes(), offset, buf.len())?;
        let start = offset as usize;
        buf.copy_from_slice(&self.bytes[start..start + buf.len()]);
        Ok(())
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        check_range(self.len_bytes(), offset, buf.len())?;
        let start = offset as usize;
        self.bytes[start..start + buf.len()].copy_from_slice(buf);
        Ok(())
    }

    fn sync(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A device backed by an image file or a raw block device.
#[derive(Debug)]
pub struct FileDevice {
    file: File,
    len: u64,
}

impl FileDevice {
    /// Creates (or truncates) an image file of `len` bytes.
    pub fn create(path: impl AsRef<Path>, len: u64) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(len)?;
        Ok(Self { file, len })
    }

    /// Opens an existing image file or device.
    pub fn open(path: impl AsRef<Path>, writable: bool) -> Result<Self> {
        let mut file = OpenOptions::new().read(true).write(writable).open(path)?;
        // Raw devices report a zero length in metadata; seeking finds the size.
        let len = file.seek(SeekFrom::End(0))?;
        Ok(Self { file, len })
    }
}

impl BlockDevice for FileDevice {
    fn len_bytes(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        check_range(self.len, offset, buf.len())?;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(())
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        check_range(self.len, offset, buf.len())?;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(buf)?;
        Ok(())
    }

    fn sync(&mut self) -> Result<()> {
        self.file.sync_data()?;
        Ok(())
    }
}

/// A memory device that stops persisting writes after a budget, simulating a
/// power cut in the middle of an operation.
///
/// Writes past the budget are silently dropped, exactly like data still in a
/// volatile cache when power is lost. [`CrashDevice::image`] returns what
/// would be on the platter.
#[derive(Clone, Debug)]
pub struct CrashDevice {
    inner: MemDevice,
    writes_left: Option<usize>,
    writes_done: usize,
}

impl CrashDevice {
    /// Wraps an image; `writes_left = None` never crashes.
    #[must_use]
    pub const fn new(image: Vec<u8>, writes_left: Option<usize>) -> Self {
        Self {
            inner: MemDevice::from_bytes(image),
            writes_left,
            writes_done: 0,
        }
    }

    /// Number of writes attempted so far.
    #[must_use]
    pub const fn writes_done(&self) -> usize {
        self.writes_done
    }

    /// The surviving image.
    #[must_use]
    pub fn image(&self) -> Vec<u8> {
        self.inner.as_bytes().to_vec()
    }
}

impl BlockDevice for CrashDevice {
    fn len_bytes(&self) -> u64 {
        self.inner.len_bytes()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.inner.read_at(offset, buf)
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        self.writes_done += 1;
        match &mut self.writes_left {
            Some(0) => Ok(()),
            Some(left) => {
                *left -= 1;
                self.inner.write_at(offset, buf)
            }
            None => self.inner.write_at(offset, buf),
        }
    }

    fn sync(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_device_rejects_out_of_range_access() {
        let mut device = MemDevice::new(16);
        device.write_at(8, &[1; 8]).unwrap();
        assert!(device.write_at(9, &[1; 8]).is_err());
        let mut buf = [0; 8];
        device.read_at(8, &mut buf).unwrap();
        assert_eq!(buf, [1; 8]);
    }

    #[test]
    fn crash_device_drops_writes_after_budget() {
        let mut device = CrashDevice::new(vec![0; 8], Some(1));
        device.write_at(0, &[1]).unwrap();
        device.write_at(1, &[1]).unwrap();
        assert_eq!(device.image()[..2], [1, 0]);
        assert_eq!(device.writes_done(), 2);
    }

    #[test]
    fn file_device_round_trips() {
        let path = std::env::temp_dir().join(format!("lfs-device-{}.img", std::process::id()));
        {
            let mut device = FileDevice::create(&path, 4096).unwrap();
            device.write_at(100, b"links").unwrap();
            device.sync().unwrap();
        }
        let mut device = FileDevice::open(&path, false).unwrap();
        assert_eq!(device.len_bytes(), 4096);
        let mut buf = [0; 5];
        device.read_at(100, &mut buf).unwrap();
        assert_eq!(&buf, b"links");
        drop(device);
        std::fs::remove_file(path).unwrap();
    }
}
