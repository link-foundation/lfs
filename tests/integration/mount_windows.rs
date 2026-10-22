//! Mounts a volume with Dokan at a free drive letter and uses it through the
//! host's file API.
//!
//! Skipped (with a note) where the Dokan driver is unavailable, unless
//! `LFS_REQUIRE_DOKAN=1` asks for a failure instead (CI sets it).

use links_fs::config::FormatOptions;
use links_fs::device::FileDevice;
use links_fs::driver::{spawn, MountOptions};
use links_fs::fs::Fs;
use std::fs;
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn dokan_required() -> bool {
    std::env::var("LFS_REQUIRE_DOKAN").is_ok_and(|value| value == "1")
}

fn free_drive_letter() -> Option<char> {
    ('L'..='Z')
        .rev()
        .find(|letter| !Path::new(&format!("{letter}:\\")).exists())
}

#[test]
fn a_mounted_volume_works_with_ordinary_file_operations() {
    let scratch = Scratch(std::env::temp_dir().join(format!("lfs-dokan-{}", std::process::id())));
    let _ = fs::remove_dir_all(&scratch.0);
    fs::create_dir_all(&scratch.0).unwrap();
    let image = scratch.0.join("m.lfs");
    let volume = Fs::format(
        FileDevice::create(&image, 16 << 20).unwrap(),
        &FormatOptions::default(),
        "m",
    )
    .unwrap();
    let letter = free_drive_letter().expect("a free drive letter");
    let mounted = match spawn(
        volume,
        Path::new(&format!("{letter}:")),
        &MountOptions::default(),
    ) {
        Ok(mounted) => mounted,
        Err(error) if !dokan_required() => {
            eprintln!("skipped: Dokan mount failed: {error}");
            return;
        }
        Err(error) => panic!("Dokan mount failed: {error}"),
    };
    assert!(mounted.is_mounted());
    let root = PathBuf::from(format!("{letter}:\\"));

    fs::write(root.join("a.txt"), b"hello").unwrap();
    fs::create_dir_all(root.join("d").join("e")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(root.join("d").join("e").join("big"), &big).unwrap();
    fs::copy(root.join("a.txt"), root.join("d").join("copy")).unwrap();
    fs::rename(root.join("d").join("copy"), root.join("h")).unwrap();
    assert_eq!(fs::read(root.join("h")).unwrap(), b"hello");
    assert_eq!(fs::read(root.join("d").join("e").join("big")).unwrap(), big);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(root.join("a.txt"))
        .unwrap();
    file.set_len(3).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"hel");
    assert_eq!(fs::metadata(root.join("a.txt")).unwrap().len(), 3);
    assert!(fs::remove_dir(root.join("d")).is_err());
    fs::remove_file(root.join("h")).unwrap();
    fs::write(root.join(".lfs").join("config"), "(atime: on)\n").unwrap();
    let config = fs::read_to_string(root.join(".lfs").join("config")).unwrap();
    assert!(config.contains("(atime: on)"), "{config}");
    let mut names: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, [".lfs", "a.txt", "d"]);

    mounted.unmount().unwrap();
    let fs = Fs::open(FileDevice::open(&image, false).unwrap(), false, &[]).unwrap();
    assert!(fs.check().is_empty(), "{:?}", fs.check());
    let root = links_fs::fs::ROOT_INO;
    let d = fs.lookup(root, "d").unwrap().ino;
    let e = fs.lookup(d, "e").unwrap().ino;
    assert_eq!(fs.lookup(e, "big").unwrap().size, 3_000_000);
    assert_eq!(fs.lookup(root, "a.txt").unwrap().size, 3);
    assert!(fs.config().to_lino().contains("(atime: on)"));
}
