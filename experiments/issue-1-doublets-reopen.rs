#!/usr/bin/env rust-script
//! Issue #1: a doublets 0.8 store on `FileMapped` (platform-mem 0.3) is
//! empty after a reopen, and while it is open it lives at byte 524288
//! (8192 records of 64 bytes) of its file instead of byte 0. LFS keeps its
//! own block-level store and only uses doublets for compatibility tests
//! (tests/integration/doublets.rs), which work around this. Reported
//! upstream as linksplatform/mem-rs#36.
//!
//! ```text
//! rust-script experiments/issue-1-doublets-reopen.rs
//! ```
//!
//! Prints, with doublets 0.8.0:
//!
//! ```text
//! open: 2 links; first 64 bytes zero: true; header at 524288: 2 allocated
//! reopened: 0 links
//! ```
//!
//! ```cargo
//! [dependencies]
//! doublets = "=0.8.0"
//! ```

use doublets::data::LinksConstants;
use doublets::mem::FileMapped;
use doublets::{unit, Doublets};

const REGION: usize = 8192 * 64;

type Store = unit::Store<u64, FileMapped<doublets::mem::unit::LinkPart<u64>>>;

fn open(path: &std::path::Path) -> Store {
    unit::Store::<u64, _>::with_constants(
        FileMapped::from_path(path).unwrap(),
        LinksConstants::external(),
    )
    .unwrap()
}

fn main() {
    let path =
        std::env::temp_dir().join(format!("doublets-reopen-{}.links", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let mut store = open(&path);
        store.create_point().unwrap();
        store.create_point().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // The doublets header's first cell counts the allocated links.
        let allocated = u64::from_le_bytes(bytes[REGION..REGION + 8].try_into().unwrap());
        println!(
            "open: {} links; first 64 bytes zero: {}; header at {REGION}: {allocated} allocated",
            store.count(),
            bytes[..64].iter().all(|&b| b == 0)
        );
    }
    println!("reopened: {} links", open(&path).count());
    let _ = std::fs::remove_file(&path);
}
