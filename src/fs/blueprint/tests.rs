use super::*;
use crate::demo::Demo;
use crate::device::MemDevice;
use crate::superblock::ARRAY_LINKS;

/// Checks that the children of every expanded node cover it exactly, in
/// order, and returns the number of leaf bytes.
fn assert_partition(node: &Node) -> u64 {
    assert!(node.size > 0, "empty node {node:?}");
    if node.children.is_empty() {
        assert!(!node.expandable(), "not fully expanded: {}", node.name);
        return node.size;
    }
    let mut next = node.offset;
    let mut leaves = 0;
    for child in &node.children {
        assert_eq!(
            child.offset, next,
            "gap or overlap in {} before {}",
            node.name, child.name
        );
        next += child.size;
        leaves += assert_partition(child);
    }
    assert_eq!(
        next,
        node.offset + node.size,
        "{} is not covered",
        node.name
    );
    leaves
}

fn find<'a>(node: &'a Node, out: &mut Vec<&'a Node>, test: &dyn Fn(&Node) -> bool) {
    if test(node) {
        out.push(node);
    }
    for child in &node.children {
        find(child, out, test);
    }
}

fn all<'a>(node: &'a Node, test: &dyn Fn(&Node) -> bool) -> Vec<&'a Node> {
    let mut out = Vec::new();
    find(node, &mut out, test);
    out
}

fn child<'a>(node: &'a Node, name: &str) -> &'a Node {
    node.children
        .iter()
        .find(|child| child.name == name)
        .unwrap_or_else(|| panic!("{} has no {name}", node.name))
}

const CONFIGS: &[&str] = &[
    "layout=united; index=off",
    "layout=united; index=tree",
    "layout=united; index=bitstring",
    "layout=united; index=adaptive; bitstring-threshold=0.05",
    "layout=split; index=off",
    "layout=split; index=adaptive",
    "layout=data-only; index=tree",
    "layout=data-only; index=bitstring",
    "layout=united; journal-blocks=0; journal=off",
    "layout=split; names=packed; journal=full",
    "layout=united; delete-percent=50; compact=true",
    "layout=united; block-size=4096; size=4M",
    "layout=split; block-size=65536; size=8M; files=4",
];

fn demo(config: &str) -> Fs<MemDevice> {
    let text = format!("size=1M; block-size=1024; files=10; file-size=3000; {config}");
    Demo::parse(&text).unwrap().build().unwrap()
}

#[test]
fn the_blueprint_partitions_every_byte_of_every_configuration() {
    for config in CONFIGS {
        let mut fs = demo(config);
        let tree = fs.blueprint(usize::MAX).unwrap();
        assert_eq!(assert_partition(&tree), fs.image_len(), "{config}");
        assert!(tree.count() > 100, "{config}: only {} nodes", tree.count());
    }
}

#[test]
fn link_records_show_the_ends_of_every_live_link() {
    for config in CONFIGS {
        let mut fs = demo(config);
        let tree = fs.blueprint(usize::MAX).unwrap();
        let records = all(&tree, &|node| {
            node.kind == "record" && node.name.starts_with("link ")
        });
        let live = (1..=fs.links.allocated())
            .filter(|&link| fs.links.exists(link))
            .count();
        assert_eq!(records.len(), live, "{config}");
        for record in records {
            let link: u64 = record.name[5..].parse().unwrap();
            assert_eq!(
                child(record, "source").value.as_deref(),
                Some(format_ref(fs.links.source(link)).as_str()),
                "{config}: link {link}"
            );
            assert_eq!(
                child(record, "target").value.as_deref(),
                Some(format_ref(fs.links.target(link)).as_str())
            );
            assert_eq!(record.size, fs.record_size(ARRAY_LINKS));
        }
    }
}

#[test]
fn inode_records_and_file_extents_match_the_files() {
    let mut fs = demo("layout=united");
    let tree = fs.blueprint(usize::MAX).unwrap();
    let inodes = all(&tree, &|node| {
        node.kind == "record" && node.name.starts_with("inode ")
    });
    assert_eq!(inodes.len() as u64, fs.inodes.used());
    let files = all(&tree, &|node| node.kind == "file");
    assert_ne!(files.len(), 0);
    for file in files {
        let Part::File { ino } = file.part else {
            panic!("file node without inode")
        };
        let inode = fs.inodes.get(ino - 1);
        let data = child(file, "data");
        assert_eq!(data.size, inode.size);
        let bytes = fs.read_image(data.offset, data.size).unwrap();
        assert_eq!(bytes, fs.read(ino, 0, inode.size as usize).unwrap());
        let record = inodes
            .iter()
            .find(|record| record.name == format!("inode {ino}"))
            .unwrap();
        assert_eq!(
            child(record, "start").value.as_deref(),
            Some((data.offset / fs.bs()).to_string().as_str())
        );
        assert!(record.note.as_deref().unwrap().contains(&file.name));
    }
    assert!(
        !all(&tree, &|node| node.kind == "slack").is_empty(),
        "the demo grows a file sparsely"
    );
}

#[test]
fn usage_sets_show_tree_nodes_and_bits() {
    let mut fs = demo("layout=data-only; index=tree");
    let tree = fs.blueprint(usize::MAX).unwrap();
    let sets = all(&tree, &|node| node.name == "tree");
    assert_ne!(sets.len(), 0);
    for set in sets {
        let count: usize = child(set, "count")
            .value
            .as_deref()
            .unwrap()
            .parse()
            .unwrap();
        let nodes = set
            .children
            .iter()
            .filter(|c| c.name.starts_with("node "))
            .count();
        assert_eq!(nodes, count);
        for node in set.children.iter().filter(|c| c.name.starts_with("node ")) {
            let names: Vec<_> = node.children.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names, ["link", "left", "right", "size", "unused"]);
        }
    }
    let mut fs = demo("layout=data-only; index=bitstring");
    let tree = fs.blueprint(usize::MAX).unwrap();
    let bytes = all(&tree, &|node| {
        node.kind == "bits" && node.size == 1 && node.value.as_deref() != Some("0")
    });
    assert_ne!(bytes.len(), 0);
    for byte in bytes {
        assert!(byte.note.as_deref().unwrap().starts_with("users "));
    }
}

#[test]
fn the_superblock_and_journal_are_dissected_field_by_field() {
    let mut fs = demo("layout=split; journal=full");
    let tree = fs.blueprint(usize::MAX).unwrap();
    let sb = child(&tree, "superblock");
    assert_eq!(child(sb, "magic").value.as_deref(), Some("LINKSFS\\x00"));
    assert_eq!(child(sb, "layout").note.as_deref(), Some("split"));
    assert_eq!(child(sb, "array descriptors").children.len(), ARRAYS);
    assert_eq!(child(sb, "crc32").size, 4);
    assert_eq!(child(sb, "layout").size, 1, "one-byte precision");
    let journal = child(&tree, "journal");
    let commit = child(journal, "commit");
    assert_eq!(child(commit, "magic").note.as_deref(), Some("commit block"));
    assert_eq!(
        child(commit, "seq").value.as_deref(),
        Some(fs.sb.seq.to_string().as_str())
    );
    assert!(journal.children[0]
        .note
        .as_deref()
        .unwrap()
        .contains("already checkpointed"));
    let mut fs = demo("journal-blocks=0; journal=off");
    let tree = fs.blueprint(1).unwrap();
    assert!(tree.children.iter().all(|child| child.name != "journal"));
}

#[test]
fn locating_a_byte_finds_the_deepest_node() {
    let mut fs = demo("layout=united; index=adaptive");
    let len = fs.image_len();
    for offset in (0..len).step_by(997).chain([0, 40, 41, len - 1]) {
        let path = fs.blueprint_locate(offset).unwrap();
        let node = fs.blueprint_at(&path, 0).unwrap();
        assert!(
            (node.offset..node.offset + node.size).contains(&offset),
            "{offset} not in {node:?}"
        );
        assert!(
            !node.expandable() || fs.blueprint_at(&path, 1).unwrap().children.is_empty(),
            "{offset}: {} could go deeper",
            node.name
        );
    }
    assert!(fs.blueprint_locate(len).is_err());
    assert!(fs.blueprint_at(&[10_000], 0).is_err());
}

#[test]
fn lazy_expansion_matches_the_full_tree() {
    let mut fs = demo("layout=split; index=tree");
    let full = fs.blueprint(usize::MAX).unwrap();
    let shallow = fs.blueprint(1).unwrap();
    assert!(shallow
        .children
        .iter()
        .all(|child| child.children.is_empty()));
    for (index, region) in full.children.iter().enumerate() {
        assert_eq!(&fs.blueprint_at(&[index], usize::MAX).unwrap(), region);
    }
}

#[test]
fn json_and_text_render_the_tree() {
    let mut fs = demo("layout=united");
    let mut tree = fs.blueprint(3).unwrap();
    fs.blueprint_hex(&mut tree, 16).unwrap();
    let json: serde_json::Value = serde_json::from_str(&tree.to_json()).unwrap();
    assert_eq!(json["offset"], 0);
    assert_eq!(json["size"], fs.image_len());
    let magic = &json["children"][0]["children"][0];
    assert_eq!(magic["name"], "magic");
    assert_eq!(magic["hex"], "4c494e4b53465300");
    let text = tree.to_text();
    assert!(text.starts_with("0x00000000"));
    assert!(text.contains("superblock"));
    assert_eq!(text.lines().count(), tree.count());
}

#[test]
fn dirty_volumes_are_committed_before_the_blueprint() {
    let mut fs = demo("layout=united");
    fs.create_point().unwrap();
    let tree = fs.blueprint(usize::MAX).unwrap();
    let last = fs.links.allocated();
    assert_ne!(
        all(&tree, &|node| node.name == format!("link {last}")).len(),
        0
    );
}

#[test]
fn times_render_as_utc_dates() {
    assert_eq!(format_time(0), "1970-01-01 00:00:00.000000000 UTC");
    assert_eq!(
        format_time(1_700_000_000_123_456_789),
        "2023-11-14 22:13:20.123456789 UTC"
    );
    assert_eq!(format_time(-1), "1969-12-31 23:59:59.999999999 UTC");
}
