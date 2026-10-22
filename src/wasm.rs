//! WebAssembly entry points for the web visualizer.
//!
//! Built with
//! `cargo rustc --lib --crate-type cdylib --target wasm32-unknown-unknown --release --no-default-features`
//! (see `docs/visualizer/build-wasm.sh`). The page formats a volume of any
//! configuration in memory ([`Demo`]) or opens a dropped image, then walks
//! its [blueprint](crate::fs::blueprint) node by node, reads raw bytes for
//! the hex view and locates the node of any byte.
//!
//! The interface is a plain C ABI over linear memory, with no bindings
//! generator: the page copies its input into a buffer from [`lfs_alloc`],
//! calls a function that returns 0 on success and -1 on failure, then reads
//! the result (JSON, text or bytes; the error message on failure) from
//! [`lfs_output_ptr`] and [`lfs_output_len`].

#![allow(unsafe_code)]

use crate::config::{RuntimeConfig, RUNTIME_KEYS};
use crate::demo::{Demo, DEMO_KEYS};
use crate::device::MemDevice;
use crate::error::{Error, Result};
use crate::fs::map::json_string;
use crate::fs::Fs;
use std::cell::RefCell;
use std::fmt::Write as _;

thread_local! {
    static VOLUME: RefCell<Option<Fs<MemDevice>>> = const { RefCell::new(None) };
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn quote(text: &str) -> String {
    let mut out = String::new();
    json_string(&mut out, text);
    out
}

/// Stores the result (or the error message) and returns the status code.
fn finish(result: Result<Vec<u8>>) -> i32 {
    let (bytes, code) = match result {
        Ok(bytes) => (bytes, 0),
        Err(error) => (error.to_string().into_bytes(), -1),
    };
    OUTPUT.with(|output| *output.borrow_mut() = bytes);
    code
}

fn with_volume(run: impl FnOnce(&mut Fs<MemDevice>) -> Result<Vec<u8>>) -> i32 {
    finish(VOLUME.with(|volume| {
        volume.borrow_mut().as_mut().map_or_else(
            || Err(Error::invalid("no volume: build or open one first")),
            run,
        )
    }))
}

/// Copies `len` bytes the page wrote at `ptr`.
///
/// # Safety
///
/// `ptr` must come from [`lfs_alloc`] with at least `len` bytes.
unsafe fn input(ptr: *const u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr`.
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

fn text(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::invalid("input is not UTF-8"))
}

/// The volume summary shown next to the blueprint.
fn summary(fs: &Fs<MemDevice>, note: &str) -> Vec<u8> {
    let stats = fs.statfs();
    let sb = fs.superblock();
    format!(
        "{{\"image_len\":{},\"block_size\":{},\"blocks\":{},\"free\":{},\"files\":{},\"links\":{},\"layout\":{},\"label\":{},\"note\":{},\"config\":{}}}",
        fs.image_len(),
        stats.block_size,
        stats.blocks,
        stats.free,
        stats.files,
        stats.links,
        quote(sb.layout.name()),
        quote(&sb.label),
        quote(note),
        quote(&fs.config().to_lino()),
    )
    .into_bytes()
}

/// Reserves `len` bytes for an input and returns their address.
#[no_mangle]
pub extern "C" fn lfs_alloc(len: usize) -> *mut u8 {
    let mut buffer = vec![0u8; len.max(1)].into_boxed_slice();
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Releases a buffer from [`lfs_alloc`].
///
/// # Safety
///
/// `ptr` and `len` must be exactly those of one [`lfs_alloc`] call, and the
/// buffer must not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn lfs_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        // SAFETY: the caller passes back the allocation of `lfs_alloc(len)`.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len.max(1))) });
    }
}

/// Address of the last result.
#[no_mangle]
pub extern "C" fn lfs_output_ptr() -> *const u8 {
    OUTPUT.with(|output| output.borrow().as_ptr())
}

/// Length of the last result.
#[no_mangle]
pub extern "C" fn lfs_output_len() -> usize {
    OUTPUT.with(|output| output.borrow().len())
}

/// Sets the clock used for timestamps (ns since the Unix epoch).
#[no_mangle]
pub extern "C" fn lfs_set_time(ns: i64) {
    crate::fs::set_wasm_clock(ns);
}

/// The configuration keys with their help and defaults, as JSON:
/// `{"demo": [[key, help], ...], "runtime": [[key, help], ...],
/// "defaults": "key=value lines"}`.
#[no_mangle]
pub extern "C" fn lfs_keys() -> i32 {
    let list = |keys: &[(&str, &str)]| {
        keys.iter()
            .map(|(key, help)| format!("[{},{}]", quote(key), quote(help)))
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut defaults = Demo::default().to_text();
    for (key, value) in RuntimeConfig::default().pairs() {
        let _ = write!(defaults, "\n{key}={value}");
    }
    finish(Ok(format!(
        "{{\"demo\":[{}],\"runtime\":[{}],\"defaults\":{}}}",
        list(DEMO_KEYS),
        list(RUNTIME_KEYS),
        quote(&defaults)
    )
    .into_bytes()))
}

/// Builds a demo volume from `key=value` parameters; the result is the
/// volume summary.
///
/// # Safety
///
/// `ptr` must hold `len` bytes (see [`lfs_alloc`]).
#[no_mangle]
pub unsafe extern "C" fn lfs_demo(ptr: *const u8, len: usize) -> i32 {
    // SAFETY: forwarded from the caller.
    let params = unsafe { input(ptr, len) };
    finish(text(&params).and_then(|params| {
        let fs = Demo::parse(&params)?.build()?;
        let out = summary(&fs, "demo volume");
        VOLUME.with(|volume| *volume.borrow_mut() = Some(fs));
        Ok(out)
    }))
}

/// Opens an image (a copy: the page's file is never changed). A volume
/// whose journal holds a committed transaction is replayed in memory.
///
/// # Safety
///
/// `ptr` must hold `len` bytes (see [`lfs_alloc`]).
#[no_mangle]
pub unsafe extern "C" fn lfs_open(ptr: *const u8, len: usize) -> i32 {
    // SAFETY: forwarded from the caller.
    let bytes = unsafe { input(ptr, len) };
    let opened = match Fs::open(MemDevice::from_bytes(bytes.clone()), false, &[]) {
        Err(Error::Unsupported(_)) => Fs::open(MemDevice::from_bytes(bytes), true, &[])
            .map(|fs| (fs, "image (journal replayed in memory)")),
        other => other.map(|fs| (fs, "image")),
    };
    finish(opened.map(|(fs, note)| {
        let out = summary(&fs, note);
        VOLUME.with(|volume| *volume.borrow_mut() = Some(fs));
        out
    }))
}

/// The space map of the volume (the `lfs map` JSON).
#[no_mangle]
pub extern "C" fn lfs_map() -> i32 {
    with_volume(|fs| Ok(fs.map_json().into_bytes()))
}

/// The blueprint node at a path of child indices (`"3/0/2"`, empty for
/// the volume), expanded `depth` levels, as JSON.
///
/// # Safety
///
/// `ptr` must hold `len` bytes (see [`lfs_alloc`]).
#[no_mangle]
pub unsafe extern "C" fn lfs_node(ptr: *const u8, len: usize, depth: u32) -> i32 {
    // SAFETY: forwarded from the caller.
    let path = unsafe { input(ptr, len) };
    with_volume(|fs| {
        let path = parse_path(&text(&path)?)?;
        Ok(fs
            .blueprint_at(&path, depth as usize)?
            .to_json()
            .into_bytes())
    })
}

/// The path of the deepest blueprint node holding byte `offset`, as a JSON
/// array of child indices.
#[no_mangle]
pub extern "C" fn lfs_locate(offset: u64) -> i32 {
    with_volume(|fs| {
        let path = fs.blueprint_locate(offset)?;
        let items: Vec<String> = path.iter().map(ToString::to_string).collect();
        Ok(format!("[{}]", items.join(",")).into_bytes())
    })
}

/// Raw image bytes `offset..offset + len` (clamped to the image).
#[no_mangle]
pub extern "C" fn lfs_bytes(offset: u64, len: u64) -> i32 {
    with_volume(|fs| {
        let end = offset.saturating_add(len).min(fs.image_len());
        fs.read_image(offset, end.saturating_sub(offset))
    })
}

/// The whole committed image, for download.
#[no_mangle]
pub extern "C" fn lfs_image() -> i32 {
    with_volume(|fs| fs.read_image(0, fs.image_len()))
}

fn parse_path(text: &str) -> Result<Vec<usize>> {
    text.split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse()
                .map_err(|_| Error::invalid(format!("bad path component {part:?}")))
        })
        .collect()
}
