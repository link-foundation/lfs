//! Mounts a volume with FUSE and uses it through the host's file API.
//!
//! Skipped (with a note) where FUSE is unavailable, unless
//! `LFS_REQUIRE_FUSE=1` asks for a failure instead (CI sets it).

use links_fs::config::FormatOptions;
use links_fs::device::FileDevice;
use links_fs::driver::{spawn, MountOptions};
use links_fs::fs::Fs;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::process::Command::new("fusermount3")
            .arg("-u")
            .arg(self.0.join("mnt"))
            .status();
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fuse_required() -> bool {
    std::env::var("LFS_REQUIRE_FUSE").is_ok_and(|value| value == "1")
}

fn format(image: &Path) -> Fs<FileDevice> {
    let meta = fs::metadata(image.parent().unwrap()).unwrap();
    let options = FormatOptions {
        root_owner: (meta.uid(), meta.gid()),
        ..FormatOptions::default()
    };
    Fs::format(FileDevice::create(image, 16 << 20).unwrap(), &options, "m").unwrap()
}

#[test]
fn a_mounted_volume_works_with_ordinary_file_operations() {
    if !Path::new("/dev/fuse").exists() && !fuse_required() {
        eprintln!("skipped: /dev/fuse is missing");
        return;
    }
    let scratch = Scratch(std::env::temp_dir().join(format!("lfs-mount-{}", std::process::id())));
    let _ = fs::remove_dir_all(&scratch.0);
    let mnt = scratch.0.join("mnt");
    fs::create_dir_all(&mnt).unwrap();
    let image = scratch.0.join("m.lfs");
    let mounted = match spawn(format(&image), &mnt, &MountOptions::default()) {
        Ok(mounted) => mounted,
        Err(error) if !fuse_required() => {
            eprintln!("skipped: FUSE mount failed: {error}");
            return;
        }
        Err(error) => panic!("FUSE mount failed: {error}"),
    };
    assert!(mounted.is_mounted());

    fs::write(mnt.join("a.txt"), b"hello").unwrap();
    fs::create_dir_all(mnt.join("d/e")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(mnt.join("d/e/big"), &big).unwrap();
    symlink("a.txt", mnt.join("s")).unwrap();
    fs::hard_link(mnt.join("a.txt"), mnt.join("d/h")).unwrap();
    assert_eq!(fs::metadata(mnt.join("a.txt")).unwrap().nlink(), 2);
    fs::rename(mnt.join("d/h"), mnt.join("h")).unwrap();
    assert_eq!(fs::read(mnt.join("s")).unwrap(), b"hello");
    assert_eq!(fs::read(mnt.join("d/e/big")).unwrap(), big);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(mnt.join("h"))
        .unwrap();
    file.set_len(3).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(fs::read(mnt.join("a.txt")).unwrap(), b"hel");
    assert!(fs::remove_dir(mnt.join("d")).is_err());
    fs::remove_file(mnt.join("h")).unwrap();
    fs::write(mnt.join(".lfs/config"), "(atime: on)\n").unwrap();
    let config = fs::read_to_string(mnt.join(".lfs/config")).unwrap();
    assert!(config.contains("(atime: on)"), "{config}");
    let mut names: Vec<String> = fs::read_dir(&mnt)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, [".lfs", "a.txt", "d", "s"]);

    mounted.unmount().unwrap();
    let fs = Fs::open(FileDevice::open(&image, false).unwrap(), false, &[]).unwrap();
    assert!(fs.check().is_empty(), "{:?}", fs.check());
    let root = links_fs::fs::ROOT_INO;
    let d = fs.lookup(root, "d").unwrap().ino;
    let e = fs.lookup(d, "e").unwrap().ino;
    assert_eq!(fs.lookup(e, "big").unwrap().size, 3_000_000);
    assert!(fs.config().to_lino().contains("(atime: on)"));
}
