//! LFS link stores against the doublets library itself: our doublets
//! files open in `doublets::unit::Store<u64>` with working indexes, and
//! files doublets writes import into a volume.

use doublets::data::LinksConstants;
use doublets::mem::FileMapped;
use doublets::{unit, Doublets, DoubletsExt};
use links_fs::config::FormatOptions;
use links_fs::device::MemDevice;
use links_fs::fs::{Creator, Fs, ROOT_INO};
use links_fs::links::packet::{self, Format};
use links_fs::links::{external, Link};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("lfs-doublets-{name}-{}.links", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn volume() -> Fs<MemDevice> {
    let mut fs = Fs::format(MemDevice::new(8 << 20), &FormatOptions::default(), "d").unwrap();
    let dir = fs
        .mkdir(ROOT_INO, "docs", 0o755, Creator::default())
        .unwrap()
        .ino;
    for name in ["a.txt", "b.txt", "ünïcode ✓"] {
        let ino = fs.create(dir, name, 0o644, Creator::default()).unwrap().ino;
        fs.write(ino, 0, name.as_bytes()).unwrap();
        fs.release(ino).unwrap();
    }
    let point = fs.create_point().unwrap();
    fs.create_link(point, external(0)).unwrap();
    fs.create_link(point, external(u64::from(u32::MAX)))
        .unwrap();
    fs
}

const fn ours(link: &doublets::Link<u64>) -> Link {
    Link {
        index: link.index,
        source: link.source,
        target: link.target,
    }
}

type Store = unit::Store<u64, FileMapped<doublets::mem::unit::LinkPart<u64>>>;

/// Doublets 0.8 on platform-mem 0.3 cannot reopen a `FileMapped` store:
/// `RawMem::grow` returns only the grown tail, which the store takes for
/// its whole memory, and `grow_filled` overwrites bytes already in the
/// file. A fresh store therefore lives at record 8192 of its file (the
/// size of its first growth step) for as long as it stays open, which is
/// where these tests read and write doublets images. See
/// `docs/case-studies/issue-1` for the details.
const REGION: u64 = 8192 * 64;

/// Opens a fresh doublets store at `path`.
fn fresh(path: &PathBuf) -> Store {
    unit::Store::<u64, _>::with_constants(
        FileMapped::from_path(path).unwrap(),
        LinksConstants::external(),
    )
    .unwrap()
}

/// The doublets image of the open store at `path`.
fn image(path: &PathBuf) -> Vec<u8> {
    std::fs::read(path).unwrap()[REGION as usize..].to_vec()
}

/// Puts `bytes` (a doublets file) into the open store at `path`, keeping
/// the capacity the store reserved.
fn load(path: &PathBuf, mut bytes: Vec<u8>) {
    let reserved = image(path)[8..16].to_vec();
    bytes[8..16].copy_from_slice(&reserved);
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(REGION)).unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
}

#[test]
fn doublets_reads_and_searches_our_files() {
    let fs = volume();
    let path = scratch("ours");
    let all = fs.links().all();
    let any = LinksConstants::<u64>::external().any;
    let mut store = fresh(&path);
    load(&path, fs.export_links(Format::Doublets).unwrap());
    let seen: Vec<Link> = store.iter().map(|link| ours(&link)).collect();
    assert_eq!(seen, all);
    assert_eq!(store.count(), all.len() as u64);
    // The size balanced trees we wrote are the ones doublets walks.
    for link in &all {
        assert_eq!(store.search(link.source, link.target), Some(link.index));
        // Like doublets, we leave null (0) ends out of the trees.
        let indexed = |end: u64, pick: fn(&Link) -> u64| {
            all.iter().filter(|l| end != 0 && pick(l) == end).count() as u64
        };
        assert_eq!(
            store.count_by([any, link.source, any]),
            indexed(link.source, |l| l.source)
        );
        assert_eq!(
            store.count_by([any, any, link.target]),
            indexed(link.target, |l| l.target)
        );
    }
    // Doublets changes our file and we read its changes.
    let a = store.create_point().unwrap();
    let b = store.create_link(a, external(7)).unwrap();
    let last = all[all.len() - 1];
    store.delete(last.index).unwrap();
    let back = packet::read_doublets(&image(&path)).unwrap();
    assert_eq!(
        back,
        store.iter().map(|link| ours(&link)).collect::<Vec<_>>()
    );
    assert!(back.contains(&Link {
        index: a,
        source: a,
        target: a
    }));
    assert!(back.contains(&Link {
        index: b,
        source: a,
        target: external(7)
    }));
    assert!(!back.contains(&last));
    assert_eq!(back.len(), all.len() + 1);
    drop(store);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn files_written_by_doublets_import_into_a_volume() {
    let path = scratch("theirs");
    let mut store = fresh(&path);
    let marker = store.create_point().unwrap();
    let mut previous = marker;
    for value in 0..500u64 {
        previous = store.create_link(previous, external(value)).unwrap();
    }
    let gap = store.create_link(marker, marker).unwrap();
    store.delete(gap).unwrap();
    store.create_link(previous, marker).unwrap();
    let expected: Vec<Link> = store.iter().map(|link| ours(&link)).collect();
    let bytes = image(&path);
    drop(store);
    let _ = std::fs::remove_file(&path);
    assert_eq!(Format::detect(&bytes), Format::Doublets);
    assert_eq!(packet::read_doublets(&bytes).unwrap(), expected);

    let mut fs = volume();
    let before = fs.links().count();
    let added = fs.import_links(&bytes, Format::Doublets).unwrap();
    assert_eq!(added, expected.len() as u64);
    assert_eq!(fs.links().count(), before + added);
    assert_eq!(fs.check(), Vec::<String>::new());
    // The chain survives under its new addresses.
    let ends = fs.query_links(None, Some(external(499)));
    assert_eq!(ends.len(), 1);
}

/// Pins the upstream behaviour `REGION` works around; when this fails,
/// doublets reopens `FileMapped` stores and the tests can use plain files.
#[test]
fn doublets_file_mapped_stores_do_not_survive_a_reopen() {
    let path = scratch("reopen");
    {
        let mut store = fresh(&path);
        store.create_point().unwrap();
        store.create_point().unwrap();
        assert_eq!(store.count(), 2);
        assert_eq!(packet::read_doublets(&image(&path)).unwrap().len(), 2);
        // Nothing lands at the start of the file.
        assert!(std::fs::read(&path).unwrap()[..64].iter().all(|&b| b == 0));
    }
    assert_eq!(fresh(&path).count(), 0);
    let _ = std::fs::remove_file(&path);
}
