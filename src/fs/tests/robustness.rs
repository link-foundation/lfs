//! Robustness: crashes after every device write, random operations against a
//! model, and corrupted images.

use super::*;

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
fn crashes_during_compaction_keep_every_file() {
    let intact =
        |fs: &mut Fs<MemDevice>| (0..6).all(|i| get(fs, &format!("/large{i}")) == pattern(9000, i));
    for journal in ["metadata", "full", "off"] {
        crash_everywhere(
            journal,
            |fs| {
                for i in 0..6 {
                    put(fs, &format!("/small{i}"), &pattern(500, i));
                    put(fs, &format!("/large{i}"), &pattern(9000, i));
                }
                for i in 0..6 {
                    fs.unlink(ROOT_INO, &format!("small{i}")).unwrap();
                }
            },
            |fs| {
                fs.compact().unwrap();
            },
            |fs| {
                // Without a journal the last commit leaves a few small gaps.
                let slack = if journal == "off" { 4 } else { 0 };
                fs.space().largest_free() + slack >= fs.statfs().free && intact(fs)
            },
            intact,
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
