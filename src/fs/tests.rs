use super::*;
use crate::config::{IndexMode, NameEncoding};
use crate::device::{CrashDevice, MemDevice};

const MIB: u64 = 1 << 20;

fn options(block_size: u32, layout: Layout) -> FormatOptions {
    FormatOptions {
        block_size,
        layout,
        ..FormatOptions::default()
    }
}

fn fresh(bytes: u64) -> Fs<MemDevice> {
    Fs::format(MemDevice::new(bytes), &FormatOptions::default(), "test").unwrap()
}

fn reopen(fs: Fs<MemDevice>) -> Fs<MemDevice> {
    let device = fs.close().unwrap();
    Fs::open(device, true, &[]).unwrap()
}

fn assert_clean<D: BlockDevice>(fs: &Fs<D>) {
    let problems = fs.check();
    assert!(problems.is_empty(), "fsck found problems: {problems:#?}");
}

fn put<D: BlockDevice>(fs: &mut Fs<D>, path: &str, data: &[u8]) -> u64 {
    let (parent, name) = fs.resolve_parent(path).unwrap();
    let existing = fs.lookup(parent, name).map(|attr| attr.ino);
    let ino = existing.unwrap_or_else(|_| {
        fs.create(parent, name, 0o644, Creator::default())
            .unwrap()
            .ino
    });
    fs.setattr(
        ino,
        SetAttr {
            size: Some(0),
            ..SetAttr::default()
        },
    )
    .unwrap();
    fs.write(ino, 0, data).unwrap();
    fs.release(ino).unwrap();
    ino
}

fn get<D: BlockDevice>(fs: &mut Fs<D>, path: &str) -> Vec<u8> {
    let ino = fs.resolve(path).unwrap();
    let size = fs.getattr(ino).unwrap().size;
    fs.read(ino, 0, size as usize).unwrap()
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn names_of<D: BlockDevice>(fs: &Fs<D>, ino: u64) -> Vec<String> {
    let mut names: Vec<String> = fs
        .readdir(ino)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    names.sort();
    names
}

#[test]
fn format_creates_an_empty_root() {
    let fs = fresh(4 * MIB);
    assert_eq!(names_of(&fs, ROOT_INO), vec![CONTROL_DIR.to_string()]);
    let root = fs.getattr(ROOT_INO).unwrap();
    assert_eq!(root.kind, FileKind::Dir);
    assert_eq!(root.nlink, 2);
    // Points, the root inode link and the configuration string.
    assert!(fs.links().count() > RESERVED_POINTS + 1);
    assert_clean(&fs);
    let fs = reopen(fs);
    assert_eq!(fs.superblock().label, "test");
    assert_clean(&fs);
}

#[test]
fn rejects_bad_geometry() {
    let small = Fs::format(MemDevice::new(4096 * 4), &FormatOptions::default(), "x");
    assert!(small.is_err());
    let odd = Fs::format(MemDevice::new(4 * MIB), &options(3000, Layout::United), "x");
    assert!(odd.is_err());
    assert!(Fs::open(MemDevice::new(4 * MIB), true, &[]).is_err());
}

#[test]
fn files_round_trip_in_every_layout() {
    for layout in [Layout::United, Layout::Split, Layout::DataOnly] {
        for encoding in [NameEncoding::Chars, NameEncoding::Packed] {
            let format = FormatOptions {
                block_size: 1024,
                layout,
                name_encoding: encoding,
                journal_blocks: None,
            };
            let mut fs = Fs::format(MemDevice::new(4 * MIB), &format, "l").unwrap();
            let docs = fs
                .mkdir(ROOT_INO, "docs", 0o755, Creator::default())
                .unwrap();
            put(&mut fs, "/docs/a.txt", b"hello links");
            put(&mut fs, "/docs/ünïcødé ✓.bin", &pattern(5000, 1));
            put(&mut fs, "/top", &[]);
            let mut fs = reopen(fs);
            assert_eq!(get(&mut fs, "/docs/a.txt"), b"hello links");
            assert_eq!(get(&mut fs, "/docs/ünïcødé ✓.bin"), pattern(5000, 1));
            assert_eq!(get(&mut fs, "/top"), b"");
            assert_eq!(
                names_of(&fs, docs.ino),
                vec!["a.txt".to_string(), "ünïcødé ✓.bin".to_string()]
            );
            assert_eq!(
                fs.path_of(fs.resolve("/docs/a.txt").unwrap()).unwrap(),
                "/docs/a.txt"
            );
            assert_clean(&fs);
        }
    }
}

#[test]
fn names_are_unique_per_directory() {
    let mut fs = fresh(4 * MIB);
    fs.create(ROOT_INO, "a", 0o644, Creator::default()).unwrap();
    assert!(matches!(
        fs.create(ROOT_INO, "a", 0o644, Creator::default()),
        Err(Error::Exists)
    ));
    assert!(matches!(
        fs.mkdir(ROOT_INO, CONTROL_DIR, 0o755, Creator::default()),
        Err(Error::Exists)
    ));
    assert!(fs.create(ROOT_INO, "", 0o644, Creator::default()).is_err());
    assert!(fs
        .create(ROOT_INO, "a/b", 0o644, Creator::default())
        .is_err());
    let long = "x".repeat(256);
    assert!(matches!(
        fs.create(ROOT_INO, &long, 0o644, Creator::default()),
        Err(Error::NameTooLong)
    ));
    // The same name text in two directories is one shared string.
    let links_before = fs.links().count();
    let sub = fs
        .mkdir(ROOT_INO, "sub", 0o755, Creator::default())
        .unwrap();
    let after_mkdir = fs.links().count();
    fs.create(sub.ino, "a", 0o644, Creator::default()).unwrap();
    // inode link + entry + binding; the name "a" is reused.
    assert_eq!(fs.links().count(), after_mkdir + 3);
    assert!(after_mkdir > links_before);
    assert_clean(&fs);
}

#[test]
fn unlink_rmdir_and_rename() {
    let mut fs = fresh(4 * MIB);
    let a = fs.mkdir(ROOT_INO, "a", 0o755, Creator::default()).unwrap();
    let b = fs.mkdir(ROOT_INO, "b", 0o755, Creator::default()).unwrap();
    put(&mut fs, "/a/f", b"one");
    put(&mut fs, "/b/g", b"two");
    assert!(matches!(fs.rmdir(ROOT_INO, "a"), Err(Error::NotEmpty)));
    assert!(matches!(fs.unlink(ROOT_INO, "a"), Err(Error::IsDir)));
    assert!(matches!(fs.rmdir(a.ino, "f"), Err(Error::NotDir)));
    fs.rename(a.ino, "f", b.ino, "g", false).unwrap();
    assert_eq!(get(&mut fs, "/b/g"), b"one");
    assert!(fs.lookup(a.ino, "f").is_err());
    fs.rename(ROOT_INO, "b", a.ino, "b2", false).unwrap();
    assert_eq!(get(&mut fs, "/a/b2/g"), b"one");
    assert_eq!(fs.getattr(ROOT_INO).unwrap().nlink, 3);
    assert_eq!(fs.getattr(a.ino).unwrap().nlink, 3);
    // A directory cannot move into itself.
    assert!(fs.rename(ROOT_INO, "a", b.ino, "loop", false).is_err());
    put(&mut fs, "/x", b"x");
    assert!(matches!(
        fs.rename(ROOT_INO, "x", a.ino, "b2", false),
        Err(Error::IsDir)
    ));
    put(&mut fs, "/y", b"y");
    assert!(matches!(
        fs.rename(ROOT_INO, "x", ROOT_INO, "y", true),
        Err(Error::Exists)
    ));
    fs.unlink(b.ino, "g").unwrap();
    fs.rmdir(a.ino, "b2").unwrap();
    fs.rmdir(ROOT_INO, "a").unwrap();
    assert_eq!(fs.getattr(ROOT_INO).unwrap().nlink, 2);
    let mut fs = reopen(fs);
    assert_eq!(names_of(&fs, ROOT_INO), vec![".lfs", "x", "y"]);
    assert_eq!(get(&mut fs, "/y"), b"y");
    assert_clean(&fs);
}

#[test]
fn removing_everything_returns_all_space_and_links() {
    let mut fs = fresh(8 * MIB);
    let base_links = fs.links().count();
    for i in 0..40 {
        put(&mut fs, &format!("/f{i}"), &pattern(i * 997, i as u8));
    }
    fs.mkdir(ROOT_INO, "d", 0o755, Creator::default()).unwrap();
    fs.commit().unwrap();
    for i in 0..40 {
        fs.unlink(ROOT_INO, &format!("f{i}")).unwrap();
    }
    fs.rmdir(ROOT_INO, "d").unwrap();
    let fs = reopen(fs);
    assert_eq!(fs.links().count(), base_links);
    assert_eq!(fs.statfs().files, 1);
    assert_eq!(fs.space().free_extents().count(), 1);
    assert_clean(&fs);
}

#[test]
fn hard_links_and_open_unlinked_files() {
    let mut fs = fresh(4 * MIB);
    let ino = put(&mut fs, "/a", b"shared");
    fs.link(ino, ROOT_INO, "b").unwrap();
    assert_eq!(fs.getattr(ino).unwrap().nlink, 2);
    fs.unlink(ROOT_INO, "a").unwrap();
    assert_eq!(get(&mut fs, "/b"), b"shared");
    fs.open_file(ino).unwrap();
    fs.unlink(ROOT_INO, "b").unwrap();
    // Still readable while open.
    assert_eq!(fs.read(ino, 0, 6).unwrap(), b"shared");
    fs.release(ino).unwrap();
    assert!(fs.getattr(ino).is_err());
    assert_clean(&fs);
}

#[test]
fn orphans_are_removed_at_mount() {
    let mut fs = fresh(4 * MIB);
    let ino = put(&mut fs, "/a", &pattern(10_000, 3));
    fs.open_file(ino).unwrap();
    fs.unlink(ROOT_INO, "a").unwrap();
    fs.commit().unwrap();
    let device = fs.into_device();
    let fs = Fs::open(device, true, &[]).unwrap();
    assert_eq!(fs.statfs().files, 1);
    assert_clean(&fs);
}

#[test]
fn symlinks() {
    let mut fs = fresh(4 * MIB);
    let link = fs
        .symlink(ROOT_INO, "s", "../some/where", Creator::default())
        .unwrap();
    assert_eq!(link.kind, FileKind::Symlink);
    assert_eq!(fs.readlink(link.ino).unwrap(), "../some/where");
    let fs2 = reopen(fs);
    let mut fs = fs2;
    let ino = fs.resolve("/s").unwrap();
    assert_eq!(fs.readlink(ino).unwrap(), "../some/where");
    fs.unlink(ROOT_INO, "s").unwrap();
    assert_clean(&fs);
}

#[test]
fn sparse_writes_truncate_and_attributes() {
    let mut fs = fresh(4 * MIB);
    let ino = fs
        .create(ROOT_INO, "f", 0o600, Creator { uid: 7, gid: 8 })
        .unwrap()
        .ino;
    fs.write(ino, 10_000, b"end").unwrap();
    let data = fs.read(ino, 0, 20_000).unwrap();
    assert_eq!(data.len(), 10_003);
    assert!(data[..10_000].iter().all(|&b| b == 0));
    let attr = fs
        .setattr(
            ino,
            SetAttr {
                size: Some(4),
                perm: Some(0o640),
                mtime: Some(42),
                ..SetAttr::default()
            },
        )
        .unwrap();
    assert_eq!(
        (attr.size, attr.perm, attr.mtime, attr.uid, attr.gid),
        (4, 0o640, 42, 7, 8)
    );
    fs.setattr(
        ino,
        SetAttr {
            size: Some(8),
            ..SetAttr::default()
        },
    )
    .unwrap();
    assert_eq!(fs.read(ino, 0, 100).unwrap(), vec![0; 8]);
    assert_clean(&fs);
}

#[test]
fn every_file_stays_one_extent_while_growing() {
    let mut fs = Fs::format(MemDevice::new(2 * MIB), &options(1024, Layout::United), "g").unwrap();
    let mut inos = Vec::new();
    for i in 0..8 {
        inos.push(put(&mut fs, &format!("/f{i}"), &pattern(3000, i)));
    }
    // Grow every file in small appends so neighbours block each other.
    let mut expected: Vec<Vec<u8>> = (0..8).map(|i| pattern(3000, i)).collect();
    for round in 0..30u8 {
        for (i, &ino) in inos.iter().enumerate() {
            let chunk = pattern(700, round ^ i as u8);
            let offset = expected[i].len() as u64;
            fs.write(ino, offset, &chunk).unwrap();
            expected[i].extend_from_slice(&chunk);
        }
        // Names add links, which take blocks between files.
        put(&mut fs, &format!("/n{round}"), b"");
    }
    for (i, &ino) in inos.iter().enumerate() {
        fs.release(ino).unwrap();
        assert_eq!(fs.read(ino, 0, 1 << 20).unwrap(), expected[i]);
    }
    for (_, len, owner) in fs.owners() {
        if let Owner::File { ino } = owner {
            let inode = fs.inode(ino).unwrap();
            assert_eq!(inode.blocks, len);
        }
    }
    let mut fs = reopen(fs);
    for (i, &ino) in inos.iter().enumerate() {
        assert_eq!(fs.read(ino, 0, 1 << 20).unwrap(), expected[i]);
    }
    assert_clean(&fs);
}

#[test]
fn compaction_merges_free_space() {
    let mut fs = Fs::format(MemDevice::new(2 * MIB), &options(1024, Layout::United), "c").unwrap();
    for i in 0..60 {
        put(&mut fs, &format!("/f{i}"), &pattern(10_000, i as u8));
    }
    for i in (0..60).step_by(2) {
        fs.unlink(ROOT_INO, &format!("f{i}")).unwrap();
    }
    fs.commit().unwrap();
    let before = fs.space().free_extents().count();
    assert!(before > 10);
    let report = fs.compact().unwrap();
    assert!(report.largest_after > report.largest_before);
    assert!(fs.space().free_extents().count() < before);
    for i in (1..60).step_by(2) {
        assert_eq!(get(&mut fs, &format!("/f{i}")), pattern(10_000, i as u8));
    }
    assert_clean(&fs);
    let mut fs = reopen(fs);
    assert_eq!(get(&mut fs, "/f59"), pattern(10_000, 59));
    assert_clean(&fs);
}

#[test]
fn a_large_file_fits_after_automatic_compaction() {
    let mut fs = Fs::format(MemDevice::new(MIB), &options(1024, Layout::United), "a").unwrap();
    for i in 0..30 {
        put(&mut fs, &format!("/f{i}"), &pattern(20_000, i as u8));
    }
    for i in (0..30).step_by(2) {
        fs.unlink(ROOT_INO, &format!("f{i}")).unwrap();
    }
    fs.commit().unwrap();
    let largest = fs.statfs().largest_free * 1024;
    let size = (largest as usize) + 50_000;
    put(&mut fs, "/big", &pattern(size, 9));
    assert_eq!(get(&mut fs, "/big"), pattern(size, 9));
    assert_clean(&fs);
}

#[test]
fn a_full_volume_reports_no_space() {
    let mut fs = Fs::format(MemDevice::new(MIB / 2), &options(1024, Layout::United), "f").unwrap();
    let ino = fs
        .create(ROOT_INO, "f", 0o644, Creator::default())
        .unwrap()
        .ino;
    assert!(matches!(
        fs.write(ino, 0, &vec![1; MIB as usize]),
        Err(Error::NoSpace)
    ));
    fs.release(ino).unwrap();
    assert_clean(&fs);
    let fs = reopen(fs);
    assert_clean(&fs);
}

#[test]
fn runtime_configuration_is_persisted_and_applied() {
    let mut fs = fresh(4 * MIB);
    fs.set_config_value("journal", "full").unwrap();
    fs.set_config_value("growth-percent", "50").unwrap();
    fs.set_config_value("index", "adaptive").unwrap();
    assert!(fs.set_config_value("journal", "sometimes").is_err());
    put(&mut fs, "/a", b"x");
    let fs = reopen(fs);
    assert_eq!(fs.config().journal, JournalMode::Full);
    assert_eq!(fs.config().growth_percent, 50);
    assert_eq!(fs.config().index, IndexMode::Adaptive);
    assert_clean(&fs);
    // Mount overrides apply without changing the stored text.
    let device = fs.close().unwrap();
    let fs = Fs::open(device, true, &[("commit-every".into(), "1".into())]).unwrap();
    assert_eq!(fs.config().commit_every, 1);
    assert_clean(&fs);
}

#[test]
fn index_modes_switch_at_runtime() {
    let mut fs = fresh(8 * MIB);
    for i in 0..50 {
        put(&mut fs, &format!("/f{i}"), b"");
    }
    for mode in ["tree", "bitstring", "adaptive", "off", "tree"] {
        fs.set_config_value("index", mode).unwrap();
        put(&mut fs, &format!("/after-{mode}"), b"z");
        assert_clean(&fs);
    }
    let mut fs = reopen(fs);
    assert_eq!(fs.config().index, IndexMode::Tree);
    assert_eq!(get(&mut fs, "/after-tree"), b"z");
    assert_clean(&fs);
}

#[test]
fn control_files() {
    let mut fs = fresh(4 * MIB);
    let dir = fs.lookup(ROOT_INO, CONTROL_DIR).unwrap();
    assert_eq!(dir.kind, FileKind::Dir);
    assert_eq!(
        names_of(&fs, dir.ino),
        vec!["config", "links", "map.json", "stats"]
    );
    let config = fs.lookup(dir.ino, "config").unwrap().ino;
    let text = String::from_utf8(fs.read(config, 0, 1 << 16).unwrap()).unwrap();
    assert!(text.contains("(journal: metadata)"));
    fs.setattr(
        config,
        SetAttr {
            size: Some(0),
            ..SetAttr::default()
        },
    )
    .unwrap();
    fs.write(config, 0, b"(journal: off)\n(gap-fill: off)\n")
        .unwrap();
    fs.release(config).unwrap();
    assert_eq!(fs.config().journal, JournalMode::Off);
    assert!(!fs.config().gap_fill);
    let map = fs.lookup(dir.ino, "map.json").unwrap();
    let json = String::from_utf8(fs.read(map.ino, 0, 1 << 20).unwrap()).unwrap();
    assert_eq!(json.len() as u64, map.size);
    assert!(json.starts_with("{\"format\":\"lfs-map\""));
    assert!(json.contains("\"kind\":\"superblock\""));
    let links = fs.lookup(dir.ino, "links").unwrap().ino;
    let links = String::from_utf8(fs.read(links, 0, 1 << 20).unwrap()).unwrap();
    assert!(links.starts_with("(1: 1 1)"));
    assert!(fs.write(map.ino, 0, b"x").is_err());
    assert_eq!(fs.parent(dir.ino).unwrap(), ROOT_INO);
    // A bad configuration is rejected and nothing changes.
    fs.write(config, 0, b"(journal: maybe)").unwrap();
    assert!(fs.release(config).is_err());
    assert_eq!(fs.config().journal, JournalMode::Off);
}

#[test]
fn space_map_covers_every_block_once() {
    let mut fs = fresh(4 * MIB);
    for i in 0..10 {
        put(&mut fs, &format!("/f{i}"), &pattern(i * 5000, 0));
    }
    fs.commit().unwrap();
    fs.unlink(ROOT_INO, "f3").unwrap();
    let map = fs.space_map();
    let mut next = 0;
    for extent in &map.extents {
        assert_eq!(extent.start, next);
        assert!(extent.len > 0);
        next += extent.len;
    }
    assert_eq!(next, map.total_blocks);
    assert!(map.extents.iter().any(|e| e.owner == Owner::Pending));
    fs.commit().unwrap();
    assert!(!fs
        .space_map()
        .extents
        .iter()
        .any(|e| e.owner == Owner::Pending));
}

#[test]
fn read_only_mounts_refuse_changes() {
    let mut fs = fresh(4 * MIB);
    put(&mut fs, "/a", b"keep");
    let device = fs.close().unwrap();
    let mut fs = Fs::open(device, false, &[]).unwrap();
    assert_eq!(get(&mut fs, "/a"), b"keep");
    assert!(matches!(
        fs.create(ROOT_INO, "b", 0o644, Creator::default()),
        Err(Error::ReadOnly)
    ));
    let ino = fs.resolve("/a").unwrap();
    assert!(matches!(fs.write(ino, 0, b"x"), Err(Error::ReadOnly)));
    let mut fs = Fs::open(fs.into_device(), true, &[("read-only".into(), "on".into())]).unwrap();
    assert!(matches!(fs.write(ino, 0, b"x"), Err(Error::ReadOnly)));
}

#[test]
fn split_layout_with_large_blocks() {
    let mut fs = Fs::format(
        MemDevice::new(16 * MIB),
        &options(65536, Layout::Split),
        "big",
    )
    .unwrap();
    put(&mut fs, "/a", &pattern(300_000, 5));
    let mut fs = reopen(fs);
    assert_eq!(get(&mut fs, "/a"), pattern(300_000, 5));
    assert_clean(&fs);
}

/// Runs `step` on a committed volume, crashing after every possible number
/// of device writes; each surviving image must mount cleanly and show
/// either the state before or after the step.
fn crash_everywhere(
    journal: &str,
    setup: impl Fn(&mut Fs<CrashDevice>),
    step: impl Fn(&mut Fs<CrashDevice>),
    verify: impl Fn(&mut Fs<MemDevice>) -> bool,
    verify_old: impl Fn(&mut Fs<MemDevice>) -> bool,
) {
    let device = CrashDevice::new(vec![0; MIB as usize], None);
    let mut fs = Fs::format(device, &options(1024, Layout::United), "crash").unwrap();
    fs.set_config_value("journal", journal).unwrap();
    fs.set_config_value("commit-every", "0").unwrap();
    setup(&mut fs);
    fs.commit().unwrap();
    let base = fs.into_device().image();
    // Count the writes of the step.
    let mut probe = Fs::open(CrashDevice::new(base.clone(), None), true, &[]).unwrap();
    step(&mut probe);
    probe.commit().unwrap();
    let writes = probe.into_device().writes_done();
    assert!(writes > 0);
    let (mut old, mut new) = (0, 0);
    for budget in 0..=writes {
        let mut fs = Fs::open(CrashDevice::new(base.clone(), Some(budget)), true, &[]).unwrap();
        step(&mut fs);
        let _ = fs.commit();
        let image = fs.into_device().image();
        let mut fs = Fs::open(MemDevice::from_bytes(image), true, &[])
            .unwrap_or_else(|error| panic!("{journal}: crash after {budget} writes: {error}"));
        let problems = fs.check();
        assert!(
            problems.is_empty(),
            "{journal}: crash after {budget} writes: {problems:#?}"
        );
        if verify(&mut fs) {
            new += 1;
        } else {
            assert!(
                verify_old(&mut fs),
                "{journal}: crash after {budget} writes: mixed state"
            );
            old += 1;
        }
    }
    assert!(old > 0 && new > 0);
}

#[test]
fn crashes_never_corrupt_metadata() {
    for journal in ["metadata", "full", "off"] {
        crash_everywhere(
            journal,
            |fs| {
                for i in 0..5 {
                    put(fs, &format!("/old{i}"), &pattern(3000, i));
                }
            },
            |fs| {
                fs.mkdir(ROOT_INO, "new", 0o755, Creator::default())
                    .unwrap();
                put(fs, "/new/file", &pattern(9000, 7));
                fs.unlink(ROOT_INO, "old1").unwrap();
                fs.rename(ROOT_INO, "old2", ROOT_INO, "renamed", false)
                    .unwrap();
            },
            |fs| {
                fs.resolve("/new/file").is_ok()
                    && get(fs, "/new/file") == pattern(9000, 7)
                    && fs.resolve("/old1").is_err()
                    && get(fs, "/renamed") == pattern(3000, 2)
            },
            |fs| {
                fs.resolve("/new").is_err()
                    && get(fs, "/old1") == pattern(3000, 1)
                    && get(fs, "/old2") == pattern(3000, 2)
            },
        );
    }
}

#[test]
fn full_journal_makes_overwrites_atomic() {
    crash_everywhere(
        "full",
        |fs| {
            put(fs, "/f", &pattern(8000, 1));
        },
        |fs| {
            let ino = fs.resolve("/f").unwrap();
            fs.write(ino, 0, &pattern(8000, 2)).unwrap();
        },
        |fs| get(fs, "/f") == pattern(8000, 2),
        |fs| get(fs, "/f") == pattern(8000, 1),
    );
}

#[test]
fn random_operations_match_a_model() {
    use std::collections::BTreeMap;
    let mut fs = Fs::format(MemDevice::new(4 * MIB), &options(1024, Layout::United), "r").unwrap();
    fs.set_config_value("commit-every", "7").unwrap();
    let mut model: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for step in 0..600 {
        let name = format!("/f{}", next() % 25);
        match next() % 6 {
            0 | 1 => {
                let data = pattern((next() % 20_000) as usize, step as u8);
                put(&mut fs, &name, &data);
                model.insert(name, data);
            }
            2 => {
                if let Some(data) = model.get_mut(&name) {
                    let ino = fs.resolve(&name).unwrap();
                    let offset = next() % (data.len() as u64 + 100);
                    let chunk = pattern((next() % 3000) as usize, !(step as u8));
                    fs.write(ino, offset, &chunk).unwrap();
                    fs.release(ino).unwrap();
                    let end = offset as usize + chunk.len();
                    if data.len() < end {
                        data.resize(end, 0);
                    }
                    data[offset as usize..end].copy_from_slice(&chunk);
                }
            }
            3 => {
                let removed = fs.unlink(ROOT_INO, &name[1..]).is_ok();
                assert_eq!(removed, model.remove(&name).is_some());
            }
            4 => {
                let to = format!("/f{}", next() % 25);
                if model.contains_key(&name) {
                    fs.rename(ROOT_INO, &name[1..], ROOT_INO, &to[1..], false)
                        .unwrap();
                    let data = model.remove(&name).unwrap();
                    model.insert(to, data);
                }
            }
            _ => {
                if next() % 4 == 0 {
                    fs = reopen(fs);
                }
            }
        }
        if step % 50 == 0 {
            assert_clean(&fs);
        }
    }
    let mut fs = reopen(fs);
    for (name, data) in &model {
        assert_eq!(&get(&mut fs, name), data, "{name}");
    }
    let mut listed = names_of(&fs, ROOT_INO);
    listed.retain(|name| name != CONTROL_DIR);
    let expected: Vec<String> = model.keys().map(|name| name[1..].to_string()).collect();
    assert_eq!(listed, expected);
    assert_clean(&fs);
}

/// Opens, checks and walks an untrusted image; must return, not hang.
fn survey(image: Vec<u8>) {
    let Ok(mut fs) = Fs::open(MemDevice::from_bytes(image), true, &[]) else {
        return;
    };
    let _ = fs.check();
    let mut stack = vec![ROOT_INO];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        seen += 1;
        if seen > 10_000 {
            break;
        }
        for entry in fs.readdir(dir).unwrap_or_default() {
            match entry.kind {
                FileKind::Dir if entry.name != CONTROL_DIR => stack.push(entry.ino),
                FileKind::File => {
                    let _ = fs.read(entry.ino, 0, 1 << 16);
                }
                _ => {}
            }
        }
    }
    let _ = fs.commit();
}

#[test]
fn corrupted_images_never_hang() {
    let mut fs = Fs::format(MemDevice::new(MIB), &options(1024, Layout::United), "z").unwrap();
    fs.mkdir(ROOT_INO, "d", 0o755, Creator::default()).unwrap();
    for i in 0..30 {
        put(&mut fs, &format!("/d/f{i}"), &pattern(i * 300, i as u8));
    }
    let metadata: Vec<u64> = fs
        .owners()
        .into_iter()
        .filter(|(_, _, owner)| matches!(owner, Owner::Chunk { .. } | Owner::Table { .. }))
        .map(|(start, _, _)| start)
        .collect();
    let image = fs.close().unwrap().into_bytes();
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    for round in 0..300 {
        let mut corrupt = image.clone();
        for _ in 0..=round % 4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let block = metadata[(seed % metadata.len() as u64) as usize];
            let offset = block as usize * 1024 + ((seed >> 20) % 1024) as usize;
            corrupt[offset] ^= 1 << ((seed >> 40) % 8);
            if round % 3 == 0 {
                corrupt[offset & !7..(offset & !7) + 8]
                    .copy_from_slice(&(seed >> 50).to_le_bytes());
            }
        }
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(std::panic::catch_unwind(|| survey(corrupt)).is_ok());
        });
        let survived = wait
            .recv_timeout(std::time::Duration::from_secs(20))
            .unwrap_or_else(|error| panic!("round {round} hangs: {error}"));
        assert!(survived, "round {round} panics");
    }
}
