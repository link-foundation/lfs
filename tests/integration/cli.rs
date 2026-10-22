//! End-to-end tests of the `lfs` command line tool.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lfs-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn lfs(args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lfs"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin.unwrap_or_default()).unwrap();
    drop(input);
    child.wait_with_output().unwrap()
}

fn ok(args: &[&str]) -> String {
    ok_with(args, None)
}

fn ok_with(args: &[&str], stdin: Option<&[u8]>) -> String {
    let output = lfs(args, stdin);
    assert!(
        output.status.success(),
        "lfs {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn fails(args: &[&str]) -> String {
    let output = lfs(args, None);
    assert!(!output.status.success(), "lfs {args:?} succeeded");
    String::from_utf8(output.stderr).unwrap()
}

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn mkfs_refuses_to_overwrite_a_volume_without_force() {
    let scratch = Scratch::new("mkfs");
    let image = scratch.path("v.img");
    let image = s(&image);
    ok(&["mkfs", image, "--size", "8M", "--label", "first"]);
    assert!(fails(&["mkfs", image, "--size", "8M"]).contains("--force"));
    ok(&[
        "mkfs", image, "--size", "8M", "--force", "--layout", "split",
    ]);
    let info = ok(&["info", image]);
    assert!(info.contains("(layout: split)"), "{info}");
    assert!(fails(&["mkfs", image, "--size", "8Q", "--force"]).contains("bad size"));
}

#[test]
fn a_volume_in_use_refuses_other_processes() {
    let scratch = Scratch::new("busy");
    let image = scratch.path("b.img");
    let image = s(&image);
    ok(&["mkfs", image, "--size", "4M"]);
    // Another process (here: this test) has the image open for writing,
    // as a mount would.
    let held = links_fs::device::FileDevice::open(image, true).unwrap();
    for args in [
        &["write", image, "/f"][..],
        &["ls", image, "/"],
        &["mkfs", image, "--size", "4M", "--force"],
    ] {
        let error = fails(args);
        assert!(
            error.contains("in use by another process"),
            "{args:?}: {error}"
        );
    }
    drop(held);
    ok_with(&["write", image, "/f"], Some(b"free again"));
    assert_eq!(ok(&["cat", image, "/f"]), "free again");
}

#[test]
fn mkfs_sets_the_root_owner() {
    let scratch = Scratch::new("owner");
    let image = scratch.path("o.lfs");
    ok(&[
        "mkfs",
        s(&image),
        "--size",
        "4M",
        "--root-owner",
        "1234:567",
    ]);
    assert!(fails(&["mkfs", s(&image), "--force", "--root-owner", "12"]).contains("UID:GID"));
}

#[test]
#[cfg(not(any(feature = "fuse", feature = "dokan")))]
fn mount_explains_how_to_get_a_driver() {
    let scratch = Scratch::new("nodriver");
    let image = scratch.path("n.lfs");
    ok(&["mkfs", s(&image), "--size", "4M"]);
    let error = fails(&["mount", s(&image), s(&scratch.path("mnt"))]);
    assert!(error.contains("--features fuse"), "{error}");
}

#[test]
#[cfg(not(feature = "nfs"))]
fn nfs_explains_how_to_get_it() {
    let scratch = Scratch::new("nonfs");
    let image = scratch.path("n.lfs");
    ok(&["mkfs", s(&image), "--size", "4M"]);
    let error = fails(&["serve", s(&image)]);
    assert!(error.contains("--features nfs"), "{error}");
    let error = fails(&["mount", "--nfs", s(&image), s(&scratch.path("mnt"))]);
    assert!(error.contains("--features nfs"), "{error}");
}

#[test]
fn files_round_trip_between_the_host_and_a_volume() {
    let scratch = Scratch::new("copy");
    let image = scratch.path("v.img");
    let image = s(&image);
    let tree = scratch.path("tree");
    std::fs::create_dir_all(tree.join("nested/deeper")).unwrap();
    std::fs::write(tree.join("small.txt"), b"tiny").unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(tree.join("nested/big.bin"), &big).unwrap();
    std::fs::write(tree.join("nested/deeper/empty"), b"").unwrap();

    ok(&["mkfs", image, "--size", "32M"]);
    ok(&["mkdir", image, "-p", "/backup/today"]);
    ok(&["put", image, s(&tree), "/backup/today"]);
    ok_with(&["write", image, "/note"], Some(b"from stdin\n"));
    assert_eq!(ok(&["cat", image, "/note"]), "from stdin\n");
    ok(&["ln", image, "-s", "backup/today/tree/small.txt", "/small"]);
    ok(&["ln", image, "/note", "/note2"]);
    ok(&["mv", image, "/note2", "/backup"]);

    let listing = ok(&["ls", image, "-lR", "/"]);
    assert!(
        listing.contains("small -> backup/today/tree/small.txt"),
        "{listing}"
    );
    assert!(
        listing.contains("/backup/today/tree/nested/deeper:"),
        "{listing}"
    );
    assert!(ok(&["ls", image, "/backup"]).contains("note2"));
    assert!(ok(&["fsck", image]).contains("clean"));

    let out = scratch.path("out");
    ok(&["get", image, "/backup/today/tree", s(&out)]);
    assert_eq!(std::fs::read(out.join("small.txt")).unwrap(), b"tiny");
    assert_eq!(std::fs::read(out.join("nested/big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(out.join("nested/deeper/empty")).unwrap(), b"");

    ok(&["rm", image, "/small", "/note"]);
    assert!(fails(&["rm", image, "/backup"]).contains("directory"));
    ok(&["rm", image, "-r", "/backup"]);
    assert_eq!(ok(&["ls", image]), ".lfs\n");
    ok(&["compact", image]);
    assert!(ok(&["fsck", image]).contains("clean"));
    assert!(fails(&["cat", image, "/missing"]).contains("not found"));
}

#[test]
fn configuration_changes_persist_and_overrides_do_not() {
    let scratch = Scratch::new("config");
    let image = scratch.path("v.img");
    let image = s(&image);
    ok(&["mkfs", image, "--size", "8M"]);
    let config = ok(&["config", image, "journal=off", "alloc=first-fit"]);
    assert!(config.contains("(journal: off)"), "{config}");
    let config = ok(&["config", image]);
    assert!(config.contains("(alloc: first-fit)"), "{config}");
    // `--set` applies to one run only.
    assert!(fails(&["--set", "read-only=on", "mkdir", image, "/x"]).contains("read-only"));
    ok(&["mkdir", image, "/x"]);
    assert!(fails(&["config", image, "no-such-key=1"]).contains("no-such-key"));
    assert!(ok(&["config", "--help"]).contains("protect-structure"));
}

#[test]
fn the_links_database_is_usable_from_the_command_line() {
    let scratch = Scratch::new("links");
    let image = scratch.path("v.img");
    let image = s(&image);
    ok(&["mkfs", image, "--size", "8M"]);
    let point = ok(&["links", image, "point"]);
    let index: u64 = point
        .trim()
        .trim_start_matches('(')
        .split(':')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let created = ok(&["links", image, "create", &index.to_string(), "$42"]);
    assert!(created.ends_with(&format!(" {index} $42)\n")), "{created}");
    let found = ok(&["links", image, "--target", "$42"]);
    assert_eq!(found, created);
    // The file system's own links are protected.
    assert!(fails(&["links", image, "delete", "6"]).contains("file system"));

    let lino = scratch.path("all.lino");
    let binary = scratch.path("all.lnb");
    let doublets = scratch.path("all.links");
    ok(&["export-links", image, s(&lino)]);
    ok(&["export-links", image, s(&binary), "--format", "binary"]);
    ok(&["export-links", image, s(&doublets)]);
    let text = std::fs::read_to_string(&lino).unwrap();
    assert!(text.contains(&format!(" {index} $42)")), "{text}");
    // Standard output gets a binary links notation archive.
    let piped = lfs(&["export-links", image, "-"], None);
    assert!(piped.status.success());
    assert_eq!(piped.stdout, std::fs::read(&binary).unwrap());

    let converted = scratch.path("converted.lino");
    ok(&["convert", s(&doublets), s(&converted)]);
    assert_eq!(std::fs::read_to_string(&converted).unwrap(), text);
    ok(&["convert", s(&binary), s(&converted)]);
    assert_eq!(std::fs::read_to_string(&converted).unwrap(), text);

    let other = scratch.path("other.img");
    let other = s(&other);
    ok(&["mkfs", other, "--size", "8M"]);
    let output = lfs(&["import-links", other, s(&binary)], None);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("imported"));
    assert!(ok(&["fsck", other]).contains("clean"));
    assert_eq!(ok(&["ls", other]), ".lfs\n");
    ok_with(&["import-links", other, "-"], Some(text.as_bytes()));
    assert!(ok(&["fsck", other]).contains("clean"));
    // An import is all or nothing: a bad last line keeps the good ones out.
    let before = ok(&["export-links", other, "-", "--format", "lino"]);
    let bad = "(500: 500 500)\n(501: 500 $7)\n(502: 2000 3000)\n";
    let output = lfs(
        &["import-links", other, "-", "--format", "lino"],
        Some(bad.as_bytes()),
    );
    assert!(!output.status.success());
    assert_eq!(
        ok(&["export-links", other, "-", "--format", "lino"]),
        before
    );
}

#[test]
fn the_space_map_is_json() {
    let scratch = Scratch::new("map");
    let image = scratch.path("v.img");
    let image = s(&image);
    ok(&["mkfs", image, "--size", "8M"]);
    ok_with(&["write", image, "/f"], Some(&[7; 10_000]));
    let map: serde_json::Value = serde_json::from_str(&ok(&["map", image])).unwrap();
    assert_eq!(map["format"], "lfs-map");
    assert!(map["extents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|extent| extent["kind"] == "file"));
}

#[test]
fn a_closed_pipe_is_not_an_error_message() {
    let scratch = Scratch::new("pipe");
    let image = scratch.path("v.img");
    let image = s(&image);
    ok(&["mkfs", image, "--size", "8M"]);
    ok_with(&["write", image, "/big"], Some(&vec![b'x'; 4 << 20]));
    let mut child = Command::new(env!("CARGO_BIN_EXE_lfs"))
        .args(["cat", image, "/big"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("lfs:"),
        "{output:?}"
    );
}

#[test]
fn resize_grows_and_shrinks_an_image() {
    let scratch = Scratch::new("resize");
    let image = scratch.path("v.img");
    let image = s(&image);
    let big: Vec<u8> = (0..6_000_000u32).map(|i| (i % 249) as u8).collect();
    let host = scratch.path("big.bin");
    std::fs::write(&host, &big).unwrap();

    ok(&["mkfs", image, "--size", "4M"]);
    assert!(fails(&["put", image, s(&host), "/big"]).contains("space"));
    let grown = ok(&["resize", image, "--size", "16M"]);
    assert!(grown.contains("1024 -> 4096 blocks"), "{grown}");
    assert_eq!(std::fs::metadata(image).unwrap().len(), 16 << 20);
    ok(&["put", image, s(&host), "/big"]);
    assert!(ok(&["fsck", image]).contains("clean"));

    // Too small for what is stored: nothing changes.
    assert!(fails(&["resize", image, "--size", "4M"]).contains("space"));
    assert_eq!(std::fs::metadata(image).unwrap().len(), 16 << 20);
    let shrunk = ok(&["resize", image, "--size", "8M"]);
    assert!(shrunk.contains("4096 -> 2048 blocks"), "{shrunk}");
    assert_eq!(std::fs::metadata(image).unwrap().len(), 8 << 20);
    assert!(ok(&["fsck", image]).contains("clean"));

    // Without --size the volume grows into an enlarged image file.
    std::fs::OpenOptions::new()
        .write(true)
        .open(image)
        .unwrap()
        .set_len(12 << 20)
        .unwrap();
    assert!(ok(&["resize", image]).contains("2048 -> 3072 blocks"));
    assert!(ok(&["info", image]).contains("3072"));
    let out = scratch.path("out.bin");
    ok(&["get", image, "/big", s(&out)]);
    assert_eq!(std::fs::read(out).unwrap(), big);
}

/// Checks that the children of `node` cover exactly its bytes, recursively.
fn assert_partitions(node: &serde_json::Value) -> usize {
    let offset = node["offset"].as_u64().unwrap();
    let size = node["size"].as_u64().unwrap();
    let Some(children) = node["children"].as_array() else {
        assert!(node["more"].is_null(), "unexpanded node {node}");
        return 1;
    };
    let mut next = offset;
    let mut leaves = 0;
    for child in children {
        assert_eq!(
            child["offset"].as_u64().unwrap(),
            next,
            "gap before {}",
            child["name"]
        );
        next += child["size"].as_u64().unwrap();
        leaves += assert_partitions(child);
    }
    assert_eq!(
        next,
        offset + size,
        "children of {} do not fill it",
        node["name"]
    );
    leaves
}

#[test]
fn demo_volumes_dissect_into_every_byte_in_every_configuration() {
    let scratch = Scratch::new("blueprint");
    let configurations: [&[&str]; 5] = [
        &[],
        &["layout=split", "index=adaptive"],
        &["layout=data-only", "journal-blocks=0", "journal=off"],
        &[
            "layout=united",
            "names=packed",
            "index=bitstring",
            "block-size=1024",
        ],
        &[
            "layout=split",
            "index=tree",
            "journal=full",
            "delete-percent=50",
            "compact=true",
        ],
    ];
    for (number, parameters) in configurations.iter().enumerate() {
        let image = scratch.path(&format!("demo{number}.img"));
        let image = s(&image);
        let mut args = vec!["demo", image];
        args.extend_from_slice(parameters);
        assert!(ok(&args).contains("created"), "{parameters:?}");
        assert!(ok(&["fsck", image]).contains("clean"), "{parameters:?}");

        let text = ok(&["blueprint", image]);
        assert!(text.starts_with("0x00000000"), "{text}");
        assert!(text.contains("magic = LINKSFS"), "{text}");
        let json = ok(&["blueprint", image, "--json", "--depth", "all"]);
        let root: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(root["offset"], 0);
        assert_eq!(
            root["size"].as_u64().unwrap(),
            std::fs::metadata(image).unwrap().len()
        );
        assert!(assert_partitions(&root) > 100, "{parameters:?}");

        let at = ok(&["blueprint", image, "--at", "@0x10"]);
        assert!(at.contains("total_blocks"), "{at}");
    }
    assert!(fails(&["demo", s(&scratch.path("x.img")), "colour=blue"]).contains("colour"));
}
