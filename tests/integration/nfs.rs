//! Serves volumes over NFS version 3 and drives them with an NFS client,
//! on every platform; then, where `LFS_REQUIRE_NFS=1` asks for it (CI on
//! macOS and Linux sets it), mounts one with the system's own NFS client.

use links_fs::config::FormatOptions;
use links_fs::device::{FileDevice, MemDevice};
use links_fs::driver::nfs::{serve, Served};
use links_fs::driver::MountOptions;
use links_fs::fs::{Fs, ROOT_INO};
use nfs3_client::nfs3_types::nfs3::{
    cookieverf3, createhow3, diropargs3, filename3, ftype3, nfs_fh3, nfspath3, nfsstat3, sattr3,
    set_mode3, set_size3, stable_how, symlinkdata3, COMMIT3args, CREATE3args, GETATTR3args,
    LINK3args, LOOKUP3args, MKDIR3args, Nfs3Option, Nfs3Result, READ3args, READDIRPLUS3args,
    READLINK3args, REMOVE3args, RENAME3args, RMDIR3args, SETATTR3args, SYMLINK3args, WRITE3args,
};
use nfs3_client::nfs3_types::rpc::{auth_unix, opaque_auth};
use nfs3_client::nfs3_types::xdr_codec::Opaque;
use nfs3_client::tokio::{TokioConnector, TokioIo};
use nfs3_client::{Nfs3Client, Nfs3ConnectionBuilder};
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::Path;
use tokio::net::TcpStream;

const UID: u32 = 1234;
const GID: u32 = 567;

type Client = Nfs3Client<TokioIo<TcpStream>>;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn local() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

fn volume() -> Fs<MemDevice> {
    Fs::format(MemDevice::new(8 << 20), &FormatOptions::default(), "nfs").unwrap()
}

async fn connect(served: &Served) -> (Client, nfs_fh3) {
    let credential = opaque_auth::auth_unix(&auth_unix {
        stamp: 1,
        machinename: Opaque::borrowed(b"test"),
        uid: UID,
        gid: GID,
        gids: vec![],
    });
    let connection = Nfs3ConnectionBuilder::new(TokioConnector, "127.0.0.1", "/")
        .connect_from_privileged_port(false)
        .mount_port(served.port())
        .nfs3_port(served.port())
        .credential(credential)
        .mount()
        .await
        .unwrap();
    let root = connection.root_nfs_fh3();
    (connection.into_nfs3_client(), root)
}

fn at<'a>(dir: &nfs_fh3, name: &'a str) -> diropargs3<'a> {
    diropargs3 {
        dir: dir.clone(),
        name: filename3::from(name.as_bytes()),
    }
}

const fn status<T, E>(result: &Nfs3Result<T, E>) -> nfsstat3 {
    match result {
        Nfs3Result::Ok(_) => nfsstat3::NFS3_OK,
        Nfs3Result::Err((status, _)) => *status,
    }
}

fn handle(handle: Nfs3Option<nfs_fh3>) -> nfs_fh3 {
    match handle {
        Nfs3Option::Some(handle) => handle,
        Nfs3Option::None => panic!("no file handle"),
    }
}

async fn create(client: &mut Client, dir: &nfs_fh3, name: &str, mode: u32) -> nfs_fh3 {
    let attributes = sattr3 {
        mode: set_mode3::Some(mode),
        ..sattr3::default()
    };
    let created = client
        .create(&CREATE3args {
            where_: at(dir, name),
            how: createhow3::GUARDED(attributes),
        })
        .await
        .unwrap()
        .unwrap();
    handle(created.obj)
}

async fn write(client: &mut Client, file: &nfs_fh3, offset: u64, data: &[u8], stable: stable_how) {
    let written = client
        .write(&WRITE3args {
            file: file.clone(),
            offset,
            count: u32::try_from(data.len()).unwrap(),
            stable,
            data: Opaque::borrowed(data),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(written.count as usize, data.len());
}

async fn read_all(client: &mut Client, file: &nfs_fh3) -> Vec<u8> {
    let mut data = Vec::new();
    loop {
        let read = client
            .read(&READ3args {
                file: file.clone(),
                offset: data.len() as u64,
                count: 64 << 10,
            })
            .await
            .unwrap()
            .unwrap();
        data.extend_from_slice(read.data.as_ref());
        if read.eof {
            return data;
        }
    }
}

/// Lists a directory a few entries at a time, so cookies are exercised.
async fn list(client: &mut Client, dir: &nfs_fh3) -> Vec<String> {
    let mut names = Vec::new();
    let mut cookie = 0;
    loop {
        let page = client
            .readdirplus(&READDIRPLUS3args {
                dir: dir.clone(),
                cookie,
                cookieverf: cookieverf3::default(),
                dircount: 128,
                maxcount: 600,
            })
            .await
            .unwrap()
            .unwrap();
        for entry in &page.reply.entries.0 {
            names.push(String::from_utf8(entry.name.as_ref().to_vec()).unwrap());
            cookie = entry.cookie;
        }
        if page.reply.eof {
            return names;
        }
    }
}

async fn lookup(client: &mut Client, dir: &nfs_fh3, name: &str) -> Result<nfs_fh3, nfsstat3> {
    match client
        .lookup(&LOOKUP3args {
            what: at(dir, name),
        })
        .await
        .unwrap()
    {
        Nfs3Result::Ok(found) => Ok(found.object),
        Nfs3Result::Err((status, _)) => Err(status),
    }
}

#[test]
fn an_nfs_client_creates_reads_lists_renames_and_removes() {
    let served = serve(volume(), local(), &MountOptions::default()).unwrap();
    assert!(served.is_serving());
    runtime().block_on(async {
        let (mut client, root) = connect(&served).await;

        let file = create(&mut client, &root, "a.txt", 0o640).await;
        let attributes = client
            .getattr(&GETATTR3args::from(file.clone()))
            .await
            .unwrap()
            .unwrap()
            .obj_attributes;
        assert_eq!(attributes.type_, ftype3::NF3REG);
        assert_eq!(attributes.mode & 0o7777, 0o640);
        assert_eq!((attributes.uid, attributes.gid), (UID, GID));
        let again = client
            .create(&CREATE3args {
                where_: at(&root, "a.txt"),
                how: createhow3::GUARDED(sattr3::default()),
            })
            .await
            .unwrap();
        assert_eq!(status(&again), nfsstat3::NFS3ERR_EXIST);

        // Unstable writes, then a commit, then a stable write.
        let big: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        for (index, chunk) in big.chunks(32 << 10).enumerate() {
            write(
                &mut client,
                &file,
                index as u64 * (32 << 10),
                chunk,
                stable_how::UNSTABLE,
            )
            .await;
        }
        client
            .commit(&COMMIT3args {
                file: file.clone(),
                offset: 0,
                count: 0,
            })
            .await
            .unwrap()
            .unwrap();
        write(&mut client, &file, 0, b"HEAD", stable_how::FILE_SYNC).await;
        let mut expected = big.clone();
        expected[..4].copy_from_slice(b"HEAD");
        assert_eq!(read_all(&mut client, &file).await, expected);

        // Truncate with SETATTR.
        client
            .setattr(&SETATTR3args {
                object: file.clone(),
                new_attributes: sattr3 {
                    size: set_size3::Some(10),
                    ..sattr3::default()
                },
                guard: Nfs3Option::None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read_all(&mut client, &file).await, &expected[..10]);

        // Directories, symbolic links, renames.
        let dir = handle(
            client
                .mkdir(&MKDIR3args {
                    where_: at(&root, "d"),
                    attributes: sattr3::default(),
                })
                .await
                .unwrap()
                .unwrap()
                .obj,
        );
        for index in 0..40 {
            create(&mut client, &dir, &format!("file-{index:02}"), 0o600).await;
        }
        let names = list(&mut client, &dir).await;
        assert_eq!(names.len(), 42, "{names:?}");
        assert_eq!(&names[..2], [".", ".."]);
        assert!(names.contains(&"file-39".to_string()));
        let link = client
            .symlink(&SYMLINK3args {
                where_: at(&root, "s"),
                symlink: symlinkdata3 {
                    symlink_attributes: sattr3::default(),
                    symlink_data: nfspath3::from(b"a.txt".as_slice()),
                },
            })
            .await
            .unwrap()
            .unwrap();
        let target = client
            .readlink(&READLINK3args {
                symlink: handle(link.obj),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(target.data.as_ref(), b"a.txt");
        client
            .rename(&RENAME3args {
                from: at(&dir, "file-00"),
                to: at(&root, "moved"),
            })
            .await
            .unwrap()
            .unwrap();
        assert!(lookup(&mut client, &root, "moved").await.is_ok());
        assert_eq!(
            lookup(&mut client, &dir, "file-00").await,
            Err(nfsstat3::NFS3ERR_NOENT)
        );
        assert_eq!(lookup(&mut client, &dir, "..").await.unwrap(), root);

        // The server library answers LINK as an unavailable procedure.
        let linked = client
            .link(&LINK3args {
                file: file.clone(),
                link: at(&root, "h"),
            })
            .await;
        assert!(linked.map_or(true, |reply| status(&reply) != nfsstat3::NFS3_OK));
        assert_eq!(
            lookup(&mut client, &root, "h").await,
            Err(nfsstat3::NFS3ERR_NOENT)
        );

        // Removal: a full directory refuses, then empties and goes.
        let full = client
            .rmdir(&RMDIR3args {
                object: at(&root, "d"),
            })
            .await
            .unwrap();
        assert_eq!(status(&full), nfsstat3::NFS3ERR_NOTEMPTY);
        for name in list(&mut client, &dir).await.iter().skip(2) {
            client
                .remove(&REMOVE3args {
                    object: at(&dir, name),
                })
                .await
                .unwrap()
                .unwrap();
        }
        client
            .rmdir(&RMDIR3args {
                object: at(&root, "d"),
            })
            .await
            .unwrap()
            .unwrap();
        let mut names = list(&mut client, &root).await;
        names.sort();
        assert_eq!(names, [".", "..", ".lfs", "a.txt", "moved", "s"]);
    });
    served.stop().unwrap();
}

#[test]
fn a_read_only_server_refuses_changes() {
    let mut fs = volume();
    let file = fs
        .create(
            ROOT_INO,
            "kept",
            0o644,
            links_fs::fs::Creator { uid: 0, gid: 0 },
        )
        .unwrap()
        .ino;
    fs.write(file, 0, b"kept").unwrap();
    fs.release(file).unwrap();
    fs.commit().unwrap();
    let options = MountOptions {
        read_only: true,
        ..MountOptions::default()
    };
    let served = serve(fs, local(), &options).unwrap();
    runtime().block_on(async {
        let (mut client, root) = connect(&served).await;
        let kept = lookup(&mut client, &root, "kept").await.unwrap();
        assert_eq!(read_all(&mut client, &kept).await, b"kept");
        let made = client
            .create(&CREATE3args {
                where_: at(&root, "new"),
                how: createhow3::UNCHECKED(sattr3::default()),
            })
            .await
            .unwrap();
        assert_eq!(status(&made), nfsstat3::NFS3ERR_ROFS);
        let removed = client
            .remove(&REMOVE3args {
                object: at(&root, "kept"),
            })
            .await
            .unwrap();
        assert_eq!(status(&removed), nfsstat3::NFS3ERR_ROFS);
    });
    served.stop().unwrap();
}

#[test]
fn stopping_the_server_commits_a_volume_that_reopens_clean() {
    let dir = std::env::temp_dir().join(format!("lfs-nfs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("n.lfs");
    let fs = Fs::format(
        FileDevice::create(&image, 8 << 20).unwrap(),
        &FormatOptions::default(),
        "n",
    )
    .unwrap();
    let served = serve(fs, local(), &MountOptions::default()).unwrap();
    runtime().block_on(async {
        let (mut client, root) = connect(&served).await;
        let file = create(&mut client, &root, "unstable", 0o644).await;
        // Never committed by the client: stopping the server must keep it.
        write(&mut client, &file, 0, &[7; 5000], stable_how::UNSTABLE).await;
        let config = lookup(&mut client, &root, ".lfs").await.unwrap();
        let config = lookup(&mut client, &config, "config").await.unwrap();
        write(
            &mut client,
            &config,
            0,
            b"(atime: on)\n",
            stable_how::FILE_SYNC,
        )
        .await;
    });
    served.stop().unwrap();
    let fs = Fs::open(FileDevice::open(&image, false).unwrap(), false, &[]).unwrap();
    assert!(fs.check().is_empty(), "{:?}", fs.check());
    let file = fs.lookup(ROOT_INO, "unstable").unwrap();
    assert_eq!(file.size, 5000);
    assert!(fs.config().to_lino().contains("(atime: on)"));
    drop(fs);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Kills the server process if a check fails first.
struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn lfs_serve_prints_where_it_listens_and_serves_the_volume() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let dir = std::env::temp_dir().join(format!("lfs-nfs-serve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("s.lfs");
    let lfs = env!("CARGO_BIN_EXE_lfs");
    let made = Command::new(lfs)
        .args(["mkfs", "--size", "4M"])
        .arg(&image)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let mut child = Child(
        Command::new(lfs)
            .arg("serve")
            .arg(&image)
            .arg("--read-only")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut lines = BufReader::new(child.0.stdout.take().unwrap()).lines();
    let serving = lines.next().unwrap().unwrap();
    let address: SocketAddr = serving.rsplit(' ').next().unwrap().parse().unwrap();
    let mount = lines.next().unwrap().unwrap();
    assert!(mount.starts_with("mount it with: "), "{mount}");
    assert!(
        mount.contains(&format!("port={}", address.port())),
        "{mount}"
    );
    runtime().block_on(async {
        let credential = opaque_auth::auth_unix(&auth_unix::default());
        let connection = Nfs3ConnectionBuilder::new(TokioConnector, "127.0.0.1", "/")
            .connect_from_privileged_port(false)
            .mount_port(address.port())
            .nfs3_port(address.port())
            .credential(credential)
            .mount()
            .await
            .unwrap();
        let root = connection.root_nfs_fh3();
        let mut client = connection.into_nfs3_client();
        assert_eq!(list(&mut client, &root).await, [".", "..", ".lfs"]);
    });
    drop(child);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
fn nfs_required() -> bool {
    std::env::var("LFS_REQUIRE_NFS").is_ok_and(|value| value == "1")
}

#[cfg(unix)]
fn sudo(command: &std::process::Command) -> std::process::Command {
    let mut sudo = std::process::Command::new("sudo");
    sudo.arg("-n")
        .arg(command.get_program())
        .args(command.get_args());
    sudo
}

/// Runs `command`; macOS lets a user mount on a directory they own, so
/// `sudo` is used only where that is refused (and always elsewhere).
#[cfg(unix)]
fn run(mut command: std::process::Command) {
    let mut output = command.output().unwrap();
    if !output.status.success() {
        eprintln!(
            "{command:?} failed ({}), retrying with sudo",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        output = sudo(&command).output().unwrap();
    }
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Uses a mounted volume through the host's file API.
#[cfg(unix)]
fn exercise(mnt: &Path) {
    use std::fs;
    use std::os::unix::fs::symlink;

    fs::write(mnt.join("a.txt"), b"hello").unwrap();
    fs::create_dir_all(mnt.join("d/e")).unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(mnt.join("d/e/big"), &big).unwrap();
    symlink("a.txt", mnt.join("s")).unwrap();
    assert_eq!(fs::read(mnt.join("s")).unwrap(), b"hello");
    assert_eq!(fs::read(mnt.join("d/e/big")).unwrap(), big);
    fs::rename(mnt.join("d/e/big"), mnt.join("d/e/big2")).unwrap();
    fs::rename(mnt.join("d/e/big2"), mnt.join("d/e/big")).unwrap();
    let file = fs::OpenOptions::new()
        .write(true)
        .open(mnt.join("a.txt"))
        .unwrap();
    file.set_len(3).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(fs::read(mnt.join("a.txt")).unwrap(), b"hel");
    assert!(fs::remove_dir(mnt.join("d")).is_err());
    fs::write(mnt.join("gone"), b"x").unwrap();
    fs::remove_file(mnt.join("gone")).unwrap();
    fs::write(mnt.join(".lfs/config"), "(atime: on)\n").unwrap();
    let config = fs::read_to_string(mnt.join(".lfs/config")).unwrap();
    assert!(config.contains("(atime: on)"), "{config}");
    let mut names: Vec<String> = fs::read_dir(mnt)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.starts_with("._"))
        .collect();
    names.sort();
    assert_eq!(names, [".lfs", "a.txt", "d", "s"]);
}

/// Mounts with the system's NFS client: as the user on macOS, through
/// `sudo` where that is refused or elsewhere.
#[cfg(unix)]
#[test]
fn the_system_nfs_client_mounts_a_volume() {
    use links_fs::driver::nfs::{mount_command, unmount_command};
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    if !nfs_required() {
        eprintln!("skipped: set LFS_REQUIRE_NFS=1 to mount with the system NFS client");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("lfs-nfs-mount-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(scratch.join("mnt")).unwrap();
    let mnt = scratch.join("mnt").canonicalize().unwrap();
    let image = scratch.join("m.lfs");
    let meta = fs::metadata(&scratch).unwrap();
    let options = FormatOptions {
        root_owner: (meta.uid(), meta.gid()),
        ..FormatOptions::default()
    };
    let fs = Fs::format(FileDevice::create(&image, 16 << 20).unwrap(), &options, "m").unwrap();
    let served = serve(fs, local(), &MountOptions::default()).unwrap();
    run(mount_command(served.port(), &mnt));
    let result = std::panic::catch_unwind(|| exercise(&mnt));
    run(unmount_command(&mnt));
    served.stop().unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }

    let fs = Fs::open(FileDevice::open(&image, false).unwrap(), false, &[]).unwrap();
    assert!(fs.check().is_empty(), "{:?}", fs.check());
    let d = fs.lookup(ROOT_INO, "d").unwrap().ino;
    let e = fs.lookup(d, "e").unwrap().ino;
    assert_eq!(fs.lookup(e, "big").unwrap().size, 3_000_000);
    assert_eq!(fs.lookup(ROOT_INO, "a.txt").unwrap().size, 3);
    assert!(fs.config().to_lino().contains("(atime: on)"));
    drop(fs);
    let _ = fs::remove_dir_all(&scratch);
}
