//! Block allocator.
//!
//! The allocation state is not stored on disk: it is rebuilt at mount from
//! the owners of every block (superblock, journal, chunk tables, chunks and
//! file extents). This removes a whole class of bitmap/metadata mismatches,
//! and every block always has exactly one owner, which the disk map shows.
//!
//! Blocks freed since the last commit stay reserved until the commit lands
//! ("pending free"), so a crash can never expose a reused block to the old
//! committed metadata. Blocks allocated since the last commit are "fresh":
//! writing them needs no journaling because no committed structure refers to
//! them yet.

use std::collections::{BTreeMap, BTreeSet};

/// How to choose among free extents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The smallest free extent that fits (fills gaps first).
    Best,
    /// The lowest-addressed free extent that fits.
    First,
}

/// Free-space tracker over `total` blocks.
#[derive(Clone, Debug)]
pub struct Space {
    total: u64,
    used: Vec<u64>,
    fresh: Vec<u64>,
    fresh_list: Vec<(u64, u64)>,
    by_start: BTreeMap<u64, u64>,
    by_len: BTreeSet<(u64, u64)>,
    pending: Vec<(u64, u64)>,
    free_blocks: u64,
}

fn bit_get(bits: &[u64], index: u64) -> bool {
    bits[(index / 64) as usize] & (1 << (index % 64)) != 0
}

fn bit_set(bits: &mut [u64], index: u64, value: bool) {
    let word = &mut bits[(index / 64) as usize];
    if value {
        *word |= 1 << (index % 64);
    } else {
        *word &= !(1 << (index % 64));
    }
}

impl Space {
    /// A device of `total` blocks, all free.
    #[must_use]
    pub fn new(total: u64) -> Self {
        let words = total.div_ceil(64) as usize;
        let mut space = Self {
            total,
            used: vec![0; words],
            fresh: vec![0; words],
            fresh_list: Vec::new(),
            by_start: BTreeMap::new(),
            by_len: BTreeSet::new(),
            pending: Vec::new(),
            free_blocks: 0,
        };
        if total > 0 {
            space.insert_free(0, total);
        }
        space
    }

    /// Number of blocks on the device.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.total
    }

    /// Blocks that can be allocated right now.
    #[must_use]
    pub const fn free_blocks(&self) -> u64 {
        self.free_blocks
    }

    /// Blocks waiting for the next commit before they become free.
    #[must_use]
    pub fn pending_blocks(&self) -> u64 {
        self.pending.iter().map(|&(_, len)| len).sum()
    }

    /// Length of the largest free extent.
    #[must_use]
    pub fn largest_free(&self) -> u64 {
        self.by_len.iter().next_back().map_or(0, |&(len, _)| len)
    }

    /// Free extents as `(start, len)` in address order.
    pub fn free_extents(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.by_start.iter().map(|(&start, &len)| (start, len))
    }

    /// Whether `block` is allocated (or pending free).
    #[must_use]
    pub fn is_used(&self, block: u64) -> bool {
        block < self.total && bit_get(&self.used, block)
    }

    /// Whether `block` was allocated after the last commit.
    #[must_use]
    pub fn is_fresh(&self, block: u64) -> bool {
        block < self.total && bit_get(&self.fresh, block)
    }

    /// Whether every block of the range is free.
    #[must_use]
    pub fn is_free(&self, start: u64, len: u64) -> bool {
        if len == 0 {
            return true;
        }
        if start.checked_add(len).is_none_or(|end| end > self.total) {
            return false;
        }
        self.by_start
            .range(..=start)
            .next_back()
            .is_some_and(|(&free_start, &free_len)| free_start + free_len >= start + len)
    }

    fn insert_free(&mut self, start: u64, len: u64) {
        self.by_start.insert(start, len);
        self.by_len.insert((len, start));
        self.free_blocks += len;
    }

    fn remove_free(&mut self, start: u64) -> u64 {
        let len = self.by_start.remove(&start).expect("free extent exists");
        self.by_len.remove(&(len, start));
        self.free_blocks -= len;
        len
    }

    /// Marks a range as used while building the state at mount.
    ///
    /// Returns `false` if part of the range is already used (two owners).
    pub fn mark_used(&mut self, start: u64, len: u64) -> bool {
        if !self.is_free(start, len) {
            return false;
        }
        self.take(start, len);
        true
    }

    fn take(&mut self, start: u64, len: u64) {
        if len == 0 {
            return;
        }
        let (&free_start, _) = self
            .by_start
            .range(..=start)
            .next_back()
            .expect("range is free");
        let free_len = self.remove_free(free_start);
        if free_start < start {
            self.insert_free(free_start, start - free_start);
        }
        let end = start + len;
        let free_end = free_start + free_len;
        if end < free_end {
            self.insert_free(end, free_end - end);
        }
        for block in start..end {
            bit_set(&mut self.used, block, true);
        }
    }

    fn take_fresh(&mut self, start: u64, len: u64) {
        self.take(start, len);
        for block in start..start + len {
            bit_set(&mut self.fresh, block, true);
        }
        self.fresh_list.push((start, len));
    }

    /// Allocates `len` contiguous blocks.
    pub fn allocate(&mut self, len: u64, fit: Fit) -> Option<u64> {
        self.allocate_avoiding(len, fit, None)
    }

    /// Allocates `len` contiguous blocks outside `avoid` (`(start, len)`).
    pub fn allocate_avoiding(&mut self, len: u64, fit: Fit, avoid: Option<(u64, u64)>) -> Option<u64> {
        if len == 0 {
            return Some(0);
        }
        let fits = |start: u64, free_len: u64| -> Option<u64> {
            let Some((avoid_start, avoid_len)) = avoid else {
                return (free_len >= len).then_some(start);
            };
            let avoid_end = avoid_start + avoid_len;
            let end = start + free_len;
            // Candidate before the avoided range.
            if start < avoid_start && avoid_start.min(end) - start >= len {
                return Some(start);
            }
            // Candidate after the avoided range.
            let after = start.max(avoid_end);
            (after < end && end - after >= len).then_some(after)
        };
        let found = match fit {
            Fit::Best => self
                .by_len
                .range((len, 0)..)
                .find_map(|&(free_len, start)| fits(start, free_len)),
            Fit::First => self
                .by_start
                .iter()
                .find_map(|(&start, &free_len)| fits(start, free_len)),
        }?;
        self.take_fresh(found, len);
        Some(found)
    }

    /// Allocates exactly `[start, start + len)` if it is free.
    pub fn allocate_at(&mut self, start: u64, len: u64) -> bool {
        if !self.is_free(start, len) {
            return false;
        }
        self.take_fresh(start, len);
        true
    }

    /// Releases a range. Fresh blocks are free immediately; blocks that the
    /// committed state still references become free at the next commit.
    pub fn free(&mut self, start: u64, len: u64) {
        let mut run_start = start;
        while run_start < start + len {
            let fresh = self.is_fresh(run_start);
            let mut run_end = run_start + 1;
            while run_end < start + len && self.is_fresh(run_end) == fresh {
                run_end += 1;
            }
            if fresh {
                self.release(run_start, run_end - run_start);
            } else {
                self.pending.push((run_start, run_end - run_start));
            }
            run_start = run_end;
        }
    }

    fn release(&mut self, start: u64, len: u64) {
        for block in start..start + len {
            debug_assert!(bit_get(&self.used, block), "double free of block {block}");
            bit_set(&mut self.used, block, false);
            bit_set(&mut self.fresh, block, false);
        }
        let mut new_start = start;
        let mut new_len = len;
        if let Some((&prev_start, &prev_len)) = self.by_start.range(..start).next_back() {
            if prev_start + prev_len == start {
                self.remove_free(prev_start);
                new_start = prev_start;
                new_len += prev_len;
            }
        }
        if let Some(&next_len) = self.by_start.get(&(start + len)) {
            self.remove_free(start + len);
            new_len += next_len;
        }
        self.insert_free(new_start, new_len);
    }

    /// Called after a commit is durable: pending blocks become free and
    /// fresh blocks become part of the committed state.
    pub fn commit(&mut self) {
        for (start, len) in std::mem::take(&mut self.fresh_list) {
            for block in start..start + len {
                bit_set(&mut self.fresh, block, false);
            }
        }
        for (start, len) in std::mem::take(&mut self.pending) {
            self.release(start, len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_fit_fills_the_smallest_gap() {
        let mut space = Space::new(100);
        assert!(space.mark_used(0, 10));
        assert!(space.mark_used(12, 20));
        assert!(space.mark_used(35, 5));
        // Gaps: [10,12) len 2, [32,35) len 3, [40,100) len 60.
        assert_eq!(space.allocate(1, Fit::Best), Some(10));
        assert_eq!(space.allocate(3, Fit::Best), Some(32));
        assert_eq!(space.allocate(3, Fit::First), Some(40));
        assert_eq!(space.free_blocks(), 100 - 10 - 20 - 5 - 1 - 3 - 3);
    }

    #[test]
    fn freeing_committed_blocks_waits_for_commit() {
        let mut space = Space::new(10);
        assert!(space.mark_used(0, 4));
        space.free(0, 4);
        assert_eq!(space.free_blocks(), 6);
        assert_eq!(space.pending_blocks(), 4);
        space.commit();
        assert_eq!(space.free_blocks(), 10);
        assert_eq!(space.largest_free(), 10);
    }

    #[test]
    fn freeing_fresh_blocks_is_immediate() {
        let mut space = Space::new(10);
        let start = space.allocate(5, Fit::First).unwrap();
        assert!(space.is_fresh(start));
        space.free(start, 5);
        assert_eq!(space.free_blocks(), 10);
        let start = space.allocate(5, Fit::First).unwrap();
        space.commit();
        assert!(!space.is_fresh(start));
        assert!(space.is_used(start));
    }

    #[test]
    fn allocate_at_and_avoid() {
        let mut space = Space::new(20);
        assert!(space.allocate_at(5, 5));
        assert!(!space.allocate_at(8, 5));
        assert!(!space.is_free(4, 2));
        assert_eq!(space.allocate_avoiding(3, Fit::First, Some((0, 4))), Some(10));
        assert_eq!(space.allocate_avoiding(4, Fit::First, Some((13, 2))), Some(0));
    }
}
