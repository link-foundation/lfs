use super::*;
use crate::links::external;

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn link(index: u64, source: u64, target: u64) -> Link {
    Link {
        index,
        source,
        target,
    }
}

/// A store with points, chains, externals and holes.
fn sample(layout: Layout) -> Links {
    let mut links = Links::new(layout, 512);
    let mut created = Vec::new();
    for _ in 0..4 {
        created.push(links.create_point().unwrap());
    }
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    while created.len() < 300 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let pick = |n: u64| created[(n % created.len() as u64) as usize];
        let source = pick(seed);
        let target = if seed.is_multiple_of(3) {
            external(seed % 100_000)
        } else {
            pick(seed >> 20)
        };
        if let Ok(index) = links.create(source, target) {
            created.push(index);
        }
    }
    // Holes: delete links nobody refers to.
    for index in (5..300).step_by(7) {
        if links.exists(index) && links.references(index) == 0 {
            links.delete(index).unwrap();
        }
    }
    assert_eq!(links.check(), Vec::<String>::new());
    links
}

#[test]
fn the_golden_archive_matches_link_cli() {
    // `(a: a a)`, `(é: é é)` at 3, `(ab: a é)` at 4, a hole at 2 (spec §10).
    let links = [link(1, 1, 1), link(3, 3, 3), link(4, 1, 3)];
    let names = [(1, "a".to_string()), (3, "é".into()), (4, "ab".into())];
    let bytes = write_archive(&links, &names).unwrap();
    assert_eq!(
        hex(&bytes),
        "12 02 20 01 24 01 02 01 01 03 03 01 03 13 02 21 02 30 01 ff ff 9f ff fd ff 17 ff fc 9f 9e"
    );
    let archive = read_archive(&bytes).unwrap();
    assert_eq!(archive.links, links);
    assert_eq!(archive.names, names);
    assert_eq!(Format::detect(&bytes), Format::Archive);
}

#[test]
fn the_empty_archive_is_four_bytes() {
    let bytes = write_archive(&[], &[]).unwrap();
    assert_eq!(hex(&bytes), "10 00 11 00");
    assert_eq!(read_archive(&bytes).unwrap(), Archive::default());
}

#[test]
fn bad_archives_are_rejected() {
    let good = write_archive(&[link(1, 1, 1)], &[]).unwrap();
    // Cut short, nothing after the links packet, trailing bytes.
    assert!(read_archive(&good[..good.len() - 1]).is_err());
    assert!(read_archive(&good[..good.len() - 2]).is_err());
    let mut trailing = good;
    trailing.push(0);
    assert!(read_archive(&trailing).is_err());
    // A triplet in the links packet.
    let triplet = LinksPacket::pack(false, &[(1, vec![Reference::Internal(1); 3])], true)
        .unwrap()
        .to_bytes()
        .unwrap();
    let mut bytes = triplet;
    bytes.extend([0x11, 0x00]);
    assert!(read_archive(&bytes).is_err());
    // An internal reference in a name.
    let mut bytes = write_archive(&[], &[]).unwrap()[..2].to_vec();
    bytes.extend(
        LinksPacket::pack(
            true,
            &[(1, vec![Reference::External(1), Reference::Internal(2)])],
            true,
        )
        .unwrap()
        .to_bytes()
        .unwrap(),
    );
    assert!(read_archive(&bytes).is_err());
    // A surrogate is not a character.
    let names = LinksPacket::pack(
        true,
        &[(1, vec![Reference::External(1), Reference::External(0xD800)])],
        true,
    )
    .unwrap()
    .to_bytes()
    .unwrap();
    let mut bytes = vec![0x10, 0x00];
    bytes.extend(names);
    assert!(read_archive(&bytes).is_err());
}

#[test]
fn every_format_round_trips_every_layout() {
    for from in [Layout::United, Layout::Split, Layout::DataOnly] {
        let store = sample(from);
        let all = store.all();
        for format in Format::ALL {
            let bytes = write(format, &all, &[(1, "one".into())]).unwrap();
            assert_eq!(Format::detect(&bytes), format, "{format:?}");
            let read_back = read(format, &bytes).unwrap();
            assert_eq!(read_back, all, "{format:?} from {from:?}");
            for into in [Layout::United, Layout::Split, Layout::DataOnly] {
                let mut copy = Links::new(into, 1024);
                copy.place_all(&read_back).unwrap();
                assert_eq!(copy.all(), all);
                assert_eq!(copy.allocated(), store.allocated());
                assert!(copy.check().is_empty(), "{:?}", copy.check());
                // Exporting the copy gives the same bytes again.
                assert_eq!(
                    write(format, &copy.all(), &[(1, "one".into())]).unwrap(),
                    bytes
                );
            }
        }
    }
}

#[test]
fn imported_holes_are_reused() {
    let mut copy = Links::new(Layout::United, 512);
    copy.place_all(&[link(3, 3, 3), link(5, 3, 5)]).unwrap();
    assert_eq!(copy.count(), 2);
    assert_eq!(copy.allocated(), 5);
    let mut fresh: Vec<u64> = (0..3).map(|_| copy.create_point().unwrap()).collect();
    fresh.sort_unstable();
    assert_eq!(fresh, [1, 2, 4]);
    assert_eq!(copy.create_point().unwrap(), 6);
    assert_eq!(copy.check(), Vec::<String>::new());
}

#[test]
fn placing_checks_before_changing_anything() {
    let mut store = Links::new(Layout::Split, 512);
    store.create_point().unwrap();
    let before = store.all();
    // Taken address, duplicate pair, missing reference, null link.
    for bad in [
        vec![link(1, 1, 1)],
        vec![link(2, 1, 1)],
        vec![link(2, 7, 7)],
        vec![link(2, 0, 0)],
        vec![link(2, 2, 2), link(3, 2, 2)],
    ] {
        assert!(store.place_all(&bad).is_err(), "{bad:?}");
        assert_eq!(store.all(), before);
        assert_eq!(store.check(), Vec::<String>::new());
    }
}

#[test]
fn doublets_files_follow_the_unused_rule() {
    let store = sample(Layout::United);
    let bytes = write_doublets(&store.all()).unwrap();
    assert_eq!(
        bytes.len(),
        (store.allocated() as usize + 1) * DOUBLETS_RECORD
    );
    assert_eq!(read_doublets(&bytes).unwrap(), store.all());
    assert!(read_doublets(&bytes[..100]).is_err());
    let mut lying = bytes[..DOUBLETS_RECORD * 2].to_vec();
    lying[0] = 5;
    assert!(read_doublets(&lying).is_err());
}

#[test]
fn lino_text_uses_dollar_externals() {
    let links = [
        link(1, 1, 1),
        link(2, 1, external(7)),
        link(3, external(0), 2),
    ];
    let text = write_lino(&links);
    assert_eq!(text, "(1: 1 1)\n(2: 1 $7)\n(3: $0 2)\n");
    assert_eq!(read_lino(&text).unwrap(), links);
    // Indented and unordered input reads the same.
    assert_eq!(
        read_lino("3: $0 2\n(1: 1 1)\n2:\n  1\n  $7\n").unwrap(),
        links
    );
    for bad in [
        "(1 1)",
        "(1: 1 1 1)",
        "(1: (1 1) 1)",
        "(x: 1 1)",
        "(1: 1 1)\n(1: 1 1)",
    ] {
        assert!(read_lino(bad).is_err(), "{bad}");
    }
}

#[test]
fn remapped_imports_reuse_existing_links() {
    let source = sample(Layout::United);
    let all = source.all();
    let mut store = Links::new(Layout::Split, 512);
    for _ in 0..9 {
        store.create_point().unwrap();
    }
    let map = import_remapped(&mut store, &all, &HashMap::new()).unwrap();
    assert_eq!(map.len(), all.len());
    assert!(store.check().is_empty(), "{:?}", store.check());
    let translate = |end: u64| if is_internal(end) { map[&end] } else { end };
    for link in &all {
        let got = store.get(map[&link.index]).unwrap();
        assert_eq!(
            (got.source, got.target),
            (translate(link.source), translate(link.target))
        );
    }
    // Acyclic links are deduplicated by content: importing them twice adds
    // nothing the second time.
    let acyclic = [
        link(10, external(1), external(2)),
        link(11, 10, external(3)),
    ];
    let count = store.count();
    import_remapped(&mut store, &acyclic, &HashMap::new()).unwrap();
    assert_eq!(store.count(), count + 2);
    import_remapped(&mut store, &acyclic, &HashMap::new()).unwrap();
    assert_eq!(store.count(), count + 2);
    // Cycles of two links.
    let cycle = [link(1, 2, external(1)), link(2, 1, external(1))];
    let map = import_remapped(&mut store, &cycle, &HashMap::new()).unwrap();
    assert_eq!(store.get(map[&1]).unwrap().source, map[&2]);
    assert_eq!(store.get(map[&2]).unwrap().source, map[&1]);
    assert_eq!(store.check(), Vec::<String>::new());
    // Dangling references and duplicate pairs are refused untouched.
    let count = store.count();
    assert!(import_remapped(&mut store, &[link(1, 5, 5)], &HashMap::new()).is_err());
    assert!(import_remapped(&mut store, &[link(1, 1, 1), link(2, 1, 1)], &HashMap::new()).is_err());
    assert_eq!(store.count(), count);
}

#[test]
fn fixed_links_map_onto_existing_ones() {
    let mut store = Links::new(Layout::United, 4096);
    let point = store.create_point().unwrap();
    let tagged = store.create(point, external(5)).unwrap();
    let fixed = HashMap::from([(7, point)]);
    let map = import_remapped(
        &mut store,
        &[link(7, 7, 7), link(8, 7, external(5)), link(9, 8, 7)],
        &fixed,
    )
    .unwrap();
    assert_eq!((map[&7], map[&8]), (point, tagged));
    assert_eq!(store.get(map[&9]).unwrap().source, tagged);
    assert_eq!(store.count(), 3);
    assert!(import_remapped(&mut store, &[link(1, 1, 1)], &HashMap::from([(2, point)])).is_err());
    assert!(import_remapped(&mut store, &[link(1, 1, 1)], &HashMap::from([(1, 99)])).is_err());
}

#[test]
fn long_chains_import_without_recursion() {
    let mut chain = vec![link(1, 1, 1)];
    for index in 2..=50_000 {
        chain.push(link(index, index - 1, external(index)));
    }
    let mut store = Links::new(Layout::DataOnly, 4096);
    let map = import_remapped(&mut store, &chain, &HashMap::new()).unwrap();
    assert_eq!(store.count(), 50_000);
    assert_eq!(map.len(), 50_000);
}

#[test]
fn formats_have_names_and_extensions() {
    for format in Format::ALL {
        assert_eq!(Format::parse(format.name()).unwrap(), format);
    }
    assert!(Format::parse("xml").is_err());
    assert_eq!(
        Format::of_path(std::path::Path::new("a/b.links")),
        Some(Format::Doublets)
    );
    assert_eq!(
        Format::of_path(std::path::Path::new("x.lino")),
        Some(Format::Lino)
    );
    assert_eq!(
        Format::of_path(std::path::Path::new("x.lnb")),
        Some(Format::Archive)
    );
    assert_eq!(Format::of_path(std::path::Path::new("x")), None);
}
