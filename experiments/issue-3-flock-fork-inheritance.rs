//! Reproduces the flaky "is in use by another process" failures of the
//! integration tests (issue #3): a `flock` lock belongs to the open file
//! description, and a child that another thread is spawning holds a copy of
//! every descriptor between fork and exec, close-on-exec or not. Closing the
//! locked file in the parent then does not release the lock until that child
//! execs, so an immediate reopen sees the volume as busy.
//!
//! An explicit `unlock()` before closing releases the lock at once, no matter
//! how many descriptors still refer to the open file description.
//!
//! Run: rustc --edition 2021 -O experiments/issue-3-flock-fork-inheritance.rs -o /tmp/flock-fork
//!      /tmp/flock-fork [rounds] [spawning-threads]
use std::fs::{File, OpenOptions, TryLockError};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

fn open(path: &str) -> File {
    OpenOptions::new().read(true).write(true).open(path).unwrap()
}

/// Locks, closes (unlocking first if asked) and immediately relocks `rounds`
/// times; returns how often the relock found the file still locked.
fn busy_reopens(path: &str, rounds: usize, unlock_first: bool) -> usize {
    let mut busy = 0;
    for _ in 0..rounds {
        let held = open(path);
        held.lock().unwrap();
        if unlock_first {
            held.unlock().unwrap();
        }
        drop(held);
        let again = open(path);
        match again.try_lock() {
            Ok(()) => again.unlock().unwrap(),
            Err(TryLockError::WouldBlock) => busy += 1,
            Err(TryLockError::Error(error)) => panic!("{error}"),
        }
    }
    busy
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rounds = args.next().map_or(20_000, |n| n.parse().unwrap());
    let spawners = args.next().map_or(4, |n| n.parse().unwrap());
    let path = std::env::temp_dir().join(format!("flock-fork-{}", std::process::id()));
    File::create(&path).unwrap();
    let path = path.to_str().unwrap().to_owned();

    let stop = Arc::new(AtomicBool::new(false));
    let threads: Vec<_> = (0..spawners)
        .map(|_| {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    Command::new("true").status().unwrap();
                }
            })
        })
        .collect();

    let closed = busy_reopens(&path, rounds, false);
    let unlocked = busy_reopens(&path, rounds, true);
    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        thread.join().unwrap();
    }
    let _ = std::fs::remove_file(&path);
    println!("{rounds} rounds with {spawners} threads spawning children:");
    println!("  close only:        {closed} reopens found the file still locked");
    println!("  unlock then close: {unlocked} reopens found the file still locked");
    if unlocked != 0 {
        std::process::exit(1);
    }
}
