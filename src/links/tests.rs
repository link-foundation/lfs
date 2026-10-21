use super::*;
use crate::config::Threshold;
use std::collections::HashMap;

fn layouts() -> [Layout; 3] {
    [Layout::United, Layout::Split, Layout::DataOnly]
}

/// A small deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn model_check(links: &Links, model: &HashMap<u64, (u64, u64)>) {
    assert_eq!(links.count(), model.len() as u64);
    for (&index, &(source, target)) in model {
        let link = links.get(index).unwrap();
        assert_eq!((link.source, link.target), (source, target));
        assert_eq!(links.search(source, target), Some(index));
    }
    for role in [Role::Source, Role::Target] {
        let mut ends: Vec<u64> = model
            .values()
            .map(|&(s, t)| if role == Role::Source { s } else { t })
            .collect();
        ends.sort_unstable();
        ends.dedup();
        for end in ends {
            let mut expected: Vec<u64> = model
                .iter()
                .filter(|(_, &(s, t))| (if role == Role::Source { s } else { t }) == end)
                .map(|(&i, _)| i)
                .collect();
            expected.sort_unstable();
            let mut got = links.users(role, end);
            got.sort_unstable();
            assert_eq!(got, expected, "{role:?} users of {end}");
            assert_eq!(links.count_role(role, end), expected.len() as u64);
        }
    }
    let problems = links.check();
    assert!(problems.is_empty(), "{problems:?}");
}

fn random_ops(layout: Layout, mode: IndexMode, seed: u64, steps: usize) {
    let mut links = Links::new(layout, 512);
    let mut space = Space::new(1 << 20);
    let threshold = Threshold::Auto.fraction();
    links.set_index(mode, threshold, 0.5, &mut space).unwrap();
    let mut model: HashMap<u64, (u64, u64)> = HashMap::new();
    let mut rng = Rng(seed);
    for step in 0..steps {
        let live: Vec<u64> = {
            let mut v: Vec<u64> = model.keys().copied().collect();
            v.sort_unstable();
            v
        };
        let pick = |rng: &mut Rng| -> u64 {
            match rng.below(4) {
                0 => external(rng.below(5)),
                2 => 0,
                _ if !live.is_empty() => live[rng.below(live.len() as u64) as usize],
                _ => external(rng.below(3)),
            }
        };
        match rng.below(10) {
            0..=4 => {
                let (s, t) = (pick(&mut rng), pick(&mut rng));
                match links.create(s, t) {
                    Ok(index) => {
                        assert!(!model.values().any(|&v| v == (s, t)));
                        model.insert(index, (s, t));
                    }
                    Err(Error::Exists) => assert!(model.values().any(|&v| v == (s, t))),
                    Err(Error::InvalidArgument(_)) => assert_eq!((s, t), (0, 0)),
                    Err(error) => panic!("{error}"),
                }
            }
            5 => {
                let index = links.create_point().unwrap();
                model.insert(index, (index, index));
            }
            6 | 7 if !live.is_empty() => {
                let index = live[rng.below(live.len() as u64) as usize];
                let used = model
                    .iter()
                    .any(|(&i, &(s, t))| i != index && (s == index || t == index));
                match links.delete(index) {
                    Ok(()) => {
                        assert!(!used);
                        model.remove(&index);
                    }
                    Err(Error::Busy(_)) => assert!(used),
                    Err(error) => panic!("{error}"),
                }
            }
            8 | 9 if !live.is_empty() => {
                let index = live[rng.below(live.len() as u64) as usize];
                let (s, t) = (pick(&mut rng), pick(&mut rng));
                match links.update(index, s, t) {
                    Ok(()) => {
                        model.insert(index, (s, t));
                    }
                    Err(Error::Exists) => {
                        assert!(model.iter().any(|(&i, &v)| v == (s, t) && i != index));
                    }
                    Err(Error::InvalidArgument(_)) => assert_eq!((s, t), (0, 0)),
                    Err(error) => panic!("{error}"),
                }
            }
            _ => {}
        }
        if step % 50 == 0 {
            model_check(&links, &model);
        }
    }
    model_check(&links, &model);
    // Delete everything that can be deleted and see the store shrink.
    loop {
        let before = model.len();
        let mut keys: Vec<u64> = model.keys().copied().collect();
        keys.sort_unstable();
        for index in keys {
            if links.delete(index).is_ok() {
                model.remove(&index);
            }
        }
        if model.len() == before {
            break;
        }
    }
    model_check(&links, &model);
    if model.is_empty() {
        assert_eq!(links.header().allocated, 0);
        assert_eq!(links.header().first_free, 0);
    }
}

#[test]
fn random_operations_match_a_model() {
    for layout in layouts() {
        for (seed, mode) in [
            (7, IndexMode::Off),
            (11, IndexMode::Tree),
            (13, IndexMode::Bitstring),
            (17, IndexMode::Adaptive),
        ] {
            random_ops(layout, mode, seed, 1500);
        }
    }
}

#[test]
#[allow(clippy::many_single_char_names)]
fn united_free_list_follows_doublets() {
    let mut links = Links::new(Layout::United, 512);
    let a = links.create_point().unwrap();
    let b = links.create(a, external(1)).unwrap();
    let c = links.create(a, external(2)).unwrap();
    let d = links.create(a, external(3)).unwrap();
    assert_eq!((a, b, c, d), (1, 2, 3, 4));
    links.delete(b).unwrap();
    links.delete(c).unwrap();
    let h = links.header();
    assert_eq!(
        (h.allocated, h.free, h.first_free, h.last_free),
        (4, 2, 3, 2)
    );
    // previous = source, next = target, circular.
    assert_eq!((links.records.source(3), links.records.target(3)), (2, 2));
    assert_eq!((links.records.source(2), links.records.target(2)), (3, 3));
    // Deleting the top link pops trailing free slots too.
    links.delete(d).unwrap();
    let h = links.header();
    assert_eq!(
        (h.allocated, h.free, h.first_free, h.last_free),
        (1, 0, 0, 0)
    );
    assert_eq!(links.records.header(header::ALLOCATED), 1);
    // Reuse takes the first free slot.
    let e = links.create(a, external(9)).unwrap();
    assert_eq!(e, 2);
}

#[test]
fn references_parse_and_format() {
    assert_eq!(parse_ref("$0").unwrap(), EXTERNAL_ZERO);
    assert_eq!(format_ref(external(42)), "$42");
    assert_eq!(parse_ref("12").unwrap(), 12);
    assert!(parse_ref("x").is_err());
    assert_eq!(as_external(external(5)), Some(5));
    assert!(is_internal(5) && !is_internal(0) && !is_internal(external(1)));
}

#[test]
fn adaptive_index_switches_to_bitstrings() {
    let mut links = Links::new(Layout::DataOnly, 512);
    let mut space = Space::new(1 << 20);
    links
        .set_index(IndexMode::Adaptive, 0.05, 0.5, &mut space)
        .unwrap();
    let hub = links.create_point().unwrap();
    let mut made = Vec::new();
    for value in 0..40 {
        made.push(links.create(hub, external(value)).unwrap());
    }
    assert_eq!(
        links.usage.handle(hub, Role::Source).kind(),
        usage::KIND_BITS
    );
    let stats = links.usage.stats();
    assert!(stats.bitstrings >= 1);
    for index in made {
        links.delete(index).unwrap();
    }
    assert_eq!(links.check(), [] as [std::string::String; 0]);
    links
        .set_index(IndexMode::Tree, 0.05, 0.5, &mut space)
        .unwrap();
    assert_eq!(
        links.usage.handle(hub, Role::Source).kind(),
        usage::KIND_TREE
    );
    assert_eq!(links.check(), [] as [std::string::String; 0]);
}
