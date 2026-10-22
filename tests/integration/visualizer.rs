//! The disk-space map visualizer (docs/visualizer) and the maps it shows.

use serde_json::Value;

const KINDS: [&str; 7] = [
    "superblock",
    "journal",
    "table",
    "chunk",
    "file",
    "free",
    "pending",
];

fn read(path: &str) -> String {
    std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|error| panic!("{path}: {error}"))
        .replace("\r\n", "\n")
}

fn number(map: &Value, key: &str) -> u64 {
    map[key].as_u64().unwrap_or_else(|| panic!("{key} missing"))
}

/// Checks what the page relies on: the header, and extents that cover every
/// block once and agree with the free-space summary.
fn check(map: &Value) {
    assert_eq!(map["format"], "lfs-map");
    assert_eq!(map["version"], 1);
    let extents = map["extents"].as_array().unwrap();
    let (mut next, mut free, mut free_extents, mut largest) = (0, 0, 0, 0);
    for extent in extents {
        let (start, len) = (number(extent, "start"), number(extent, "len"));
        let kind = extent["kind"].as_str().unwrap();
        assert_eq!(start, next, "extents must be contiguous: {extent}");
        assert!(len > 0, "{extent}");
        assert!(KINDS.contains(&kind), "unknown kind {kind}");
        match kind {
            "free" => {
                free += len;
                free_extents += 1;
                largest = largest.max(len);
            }
            "file" => assert!(extent["path"].is_string() && extent["ino"].is_u64()),
            "chunk" => assert!(extent["array"].is_string() && extent["chunk"].is_u64()),
            "table" => assert!(extent["array"].is_string()),
            _ => {}
        }
        next = start + len;
    }
    assert_eq!(next, number(map, "total_blocks"));
    assert_eq!(free, number(map, "free_blocks"));
    assert_eq!(free_extents, number(map, "free_extents"));
    assert_eq!(largest, number(map, "largest_free"));
}

#[test]
fn the_sample_maps_are_valid_and_show_compaction() {
    let before: Value = serde_json::from_str(&read("docs/visualizer/sample-map.json")).unwrap();
    let after: Value =
        serde_json::from_str(&read("docs/visualizer/sample-compacted.json")).unwrap();
    check(&before);
    check(&after);
    assert!(
        number(&before, "free_extents") > 1,
        "the first sample shows gaps"
    );
    assert_eq!(number(&after, "free_extents"), 1, "compaction joins them");
    assert_eq!(
        number(&before, "free_blocks"),
        number(&after, "free_blocks")
    );
    assert_eq!(number(&before, "files"), number(&after, "files"));
}

#[test]
fn the_page_knows_every_kind_and_loads_the_samples() {
    let page = read("docs/visualizer/index.html");
    for kind in KINDS {
        assert!(
            page.contains(&format!("\n  {kind}:")),
            "no style for {kind}"
        );
    }
    for sample in ["sample-map.json", "sample-compacted.json"] {
        assert!(
            page.contains(&format!("data-sample=\"{sample}\"")),
            "{sample}"
        );
    }
    assert!(page.contains("new URLSearchParams(location.search).get(\"map\")"));
}

#[test]
fn pages_deploy_the_visualizer() {
    let release = read(".github/workflows/release.yml");
    assert!(release.contains("cp -r docs/visualizer target/doc/visualizer"));
}

#[test]
fn a_fresh_map_passes_the_same_checks() {
    let dir = std::env::temp_dir().join(format!("lfs-visualizer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("v.lfs");
    let output = dir.join("map.json");
    let run = |args: &[&str]| {
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_lfs"))
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "lfs {args:?}");
    };
    let (image_arg, output_arg) = (image.to_str().unwrap(), output.to_str().unwrap());
    run(&["mkfs", image_arg, "--size", "4M", "--label", "fresh"]);
    run(&["mkdir", image_arg, "/dir"]);
    run(&["map", image_arg, "-o", output_arg]);
    let map: Value = serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    check(&map);
    assert_eq!(map["label"], "fresh");
}
