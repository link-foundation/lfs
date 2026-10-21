use super::*;
use crate::config::FormatOptions;
use crate::device::MemDevice;
use crate::fs::{Creator, ROOT_INO, SYMLINK};
use crate::links::external;

fn fresh() -> Fs<MemDevice> {
    Fs::format(MemDevice::new(4 << 20), &FormatOptions::default(), "links").unwrap()
}

fn reopen(fs: Fs<MemDevice>) -> Fs<MemDevice> {
    Fs::open(fs.close().unwrap(), true, &[]).unwrap()
}

fn assert_clean(fs: &Fs<MemDevice>) {
    let problems = fs.check();
    assert!(problems.is_empty(), "fsck found problems: {problems:#?}");
}

fn populate(fs: &mut Fs<MemDevice>) -> u64 {
    let docs = fs
        .mkdir(ROOT_INO, "docs", 0o755, Creator::default())
        .unwrap()
        .ino;
    let file = fs
        .create(docs, "notes.txt", 0o644, Creator::default())
        .unwrap()
        .ino;
    fs.write(file, 0, b"links all the way down").unwrap();
    fs.release(file).unwrap();
    fs.symlink(ROOT_INO, "notes", "docs/notes.txt", Creator::default())
        .unwrap();
    file
}

#[test]
fn user_links_share_the_volume_and_survive_remounts() {
    let mut fs = fresh();
    populate(&mut fs);
    let tag = fs.create_point().unwrap();
    let red = fs.create_link(tag, external(0xFF_00_00)).unwrap();
    let marked = fs
        .create_link(links_notation::binary::packet::STRING, red)
        .unwrap();
    assert!(!fs.is_structure(tag) && !fs.is_structure(red) && !fs.is_structure(marked));
    let mut fs = reopen(fs);
    assert_clean(&fs);
    // The point starts at itself, too.
    assert_eq!(fs.query_links(Some(tag), None).len(), 2);
    assert_eq!(fs.query_links(None, Some(red))[0].index, marked);
    assert_eq!(
        fs.query_links(Some(tag), Some(external(0xFF_00_00)))[0].index,
        red
    );
    assert_eq!(fs.query_links(Some(red), Some(tag)), Vec::new());
    let all = fs.query_links(None, None);
    assert_eq!(all.len() as u64, fs.links().count());
    let other = fs.create_point().unwrap();
    assert!(matches!(fs.delete_link(red), Err(Error::Busy(_))));
    assert!(matches!(
        fs.update_link(red, other, other),
        Err(Error::Busy(_))
    ));
    fs.update_link(marked, other, red).unwrap();
    fs.delete_link(marked).unwrap();
    fs.delete_link(red).unwrap();
    let fs = reopen(fs);
    assert!(!fs.links().exists(red));
    assert_clean(&fs);
}

#[test]
fn the_file_system_structure_is_protected() {
    let mut fs = fresh();
    let file = populate(&mut fs);
    let user = fs.create_point().unwrap();
    let inode = fs.inode_link(file).unwrap();
    let root = fs.inode_link(ROOT_INO).unwrap();
    let mut structure = vec![inode, root, FILE, SYMLINK, CONFIG];
    fs.links().each_source(root, &mut |entry| {
        structure.push(entry);
        true
    });
    let entry = structure[5];
    structure.extend(fs.query_links(Some(entry), None).iter().map(|l| l.index));
    // Two entries in the root and the binding of the first.
    assert_eq!(structure.len(), 8);
    for &link in &structure {
        assert!(fs.is_structure(link), "{link}");
        assert!(fs.create_link(user, link).is_err());
        assert!(fs.create_link(link, user).is_err());
        assert!(fs.update_link(link, user, user).is_err());
        assert!(fs.delete_link(link).is_err());
    }
    for point in 1..=RESERVED_POINTS {
        assert!(fs.delete_link(point).is_err());
    }
    // Markers stay usable.
    fs.create_link(1, external(42)).unwrap();
    assert_clean(&fs);
    // Expert mode: anything goes, and fsck tells what it broke.
    fs.set_config_value("protect-structure", "off").unwrap();
    let fake = fs.create_link(user, inode).unwrap();
    let problems = fs.check();
    assert!(!problems.is_empty(), "{problems:?}");
    fs.delete_link(fake).unwrap();
    assert_clean(&fs);
}

#[test]
fn every_format_exports_the_whole_store() {
    let mut fs = fresh();
    populate(&mut fs);
    fs.create_point().unwrap();
    let all = fs.links().all();
    for format in Format::ALL {
        let bytes = fs.export_links(format).unwrap();
        assert_eq!(Format::detect(&bytes), format);
        assert_eq!(packet::read(format, &bytes).unwrap(), all);
    }
    let archive = packet::read_archive(&fs.export_links(Format::Archive).unwrap()).unwrap();
    assert_eq!(archive.names.len(), RESERVED_POINTS as usize);
    assert_eq!(archive.names[2], (3, "string".to_string()));
}

#[test]
fn imports_never_become_structure() {
    let mut source = fresh();
    populate(&mut source);
    let tag = source.create_point().unwrap();
    source.create_link(tag, external(7)).unwrap();
    let archive = source.export_links(Format::Archive).unwrap();
    for format in Format::ALL {
        let bytes = source.export_links(format).unwrap();
        let mut target = fresh();
        let ino = target
            .create(ROOT_INO, "kept", 0o644, Creator::default())
            .unwrap()
            .ino;
        target.release(ino).unwrap();
        let before = target.links().all();
        let added = target.import_links(&bytes, format).unwrap();
        assert!(added > 0);
        for link in target.links().all() {
            if !before.contains(&link) {
                assert!(!target.is_structure(link.index), "{format:?} {link:?}");
            }
        }
        let names: Vec<String> = target
            .readdir(ROOT_INO)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["kept", crate::fs::CONTROL_DIR]);
        let target = reopen(target);
        assert_clean(&target);
    }
    // Names shared through the marker points are reused: a second import
    // of the same archive only adds what hangs off fresh points.
    let mut target = fresh();
    let first = target.import_links(&archive, Format::Archive).unwrap();
    let second = target.import_links(&archive, Format::Archive).unwrap();
    assert!(second < first, "{second} < {first}");
    assert_clean(&target);
}

#[test]
fn read_only_volumes_refuse_link_changes() {
    let mut fs = fresh();
    let point = fs.create_point().unwrap();
    let archive = fs.export_links(Format::Archive).unwrap();
    let fs = Fs::open(fs.close().unwrap(), false, &[]).unwrap();
    let mut fs = fs;
    assert!(matches!(fs.create_point(), Err(Error::ReadOnly)));
    assert!(matches!(fs.create_link(point, point), Err(Error::ReadOnly)));
    assert!(matches!(fs.delete_link(point), Err(Error::ReadOnly)));
    assert!(matches!(
        fs.import_links(&archive, Format::Archive),
        Err(Error::ReadOnly)
    ));
    assert!(fs.export_links(Format::Lino).is_ok());
}
