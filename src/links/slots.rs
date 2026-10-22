//! Slot allocation: the free list of the united layout, the free set of the
//! split layouts, and placing links at exact addresses.

use super::{is_internal, Link, Links, ROLES};
use crate::config::Layout;
use crate::error::{Error, Result};
use std::collections::{BTreeSet, HashSet};

impl Links {
    pub(super) fn allocate_slot(&mut self) -> u64 {
        let index = match self.records.layout {
            Layout::United if self.header.first_free != 0 => {
                let index = self.header.first_free;
                self.free_list_detach(index);
                index
            }
            Layout::Split | Layout::DataOnly if !self.free_set.is_empty() => {
                let index = self.free_set.pop_first().unwrap_or_default();
                self.header.free -= 1;
                index
            }
            _ => {
                self.header.allocated += 1;
                self.header.allocated
            }
        };
        self.reset_slot(index);
        index
    }

    /// Clears the ends and the tree cells of a slot (not the roots of its
    /// per-link trees, which belong to its users).
    pub(super) fn reset_slot(&mut self, index: u64) {
        self.records.ensure(index);
        self.records.set_link(index, 0, 0);
        if self.records.layout.has_embedded_index() {
            for role in ROLES {
                for which in 0..3 {
                    self.records.set_tree_cell(index, role, which, 0);
                }
            }
        }
    }

    /// Creates links at exact addresses, as importing a store does.
    ///
    /// Every address must be free, every `(source, target)` pair new, and
    /// every internal reference must name a live link or one of `links`, so
    /// references may point forward. Nothing changes if a check fails.
    /// Addresses between the old top and the new links become free slots.
    pub fn place_all(&mut self, links: &[Link]) -> Result<()> {
        let mut addresses = BTreeSet::new();
        let mut pairs = HashSet::with_capacity(links.len());
        for link in links {
            if !is_internal(link.index) {
                return Err(Error::invalid(format!("bad link address {}", link.index)));
            }
            if self.exists(link.index) || !addresses.insert(link.index) {
                return Err(Error::invalid(format!("address {} is taken", link.index)));
            }
            if link.source == 0 && link.target == 0 {
                return Err(Error::invalid("a link needs a non-null end"));
            }
            if !pairs.insert((link.source, link.target))
                || self.search(link.source, link.target).is_some()
            {
                return Err(Error::Exists);
            }
        }
        for link in links {
            for end in [link.source, link.target] {
                if is_internal(end) && !self.exists(end) && !addresses.contains(&end) {
                    return Err(Error::invalid(format!(
                        "link {} refers to missing link {end}",
                        link.index
                    )));
                }
            }
        }
        let Some(&top) = addresses.last() else {
            return Ok(());
        };
        while self.header.allocated < top {
            self.header.allocated += 1;
            let index = self.header.allocated;
            self.reset_slot(index);
            if self.records.layout == Layout::United {
                self.free_list_attach_first(index);
            } else {
                self.free_set.insert(index);
                self.header.free += 1;
            }
        }
        for link in links {
            if self.records.layout == Layout::United {
                self.free_list_detach(link.index);
            } else {
                self.free_set.remove(&link.index);
                self.header.free -= 1;
            }
            self.reset_slot(link.index);
            self.attach(link.index, link.source, link.target)?;
        }
        self.sync_header();
        Ok(())
    }

    pub(super) fn free_slot(&mut self, index: u64) {
        if index < self.header.allocated {
            if self.records.layout == Layout::United {
                self.free_list_attach_first(index);
            } else {
                self.free_set.insert(index);
                self.header.free += 1;
            }
            return;
        }
        self.header.allocated -= 1;
        loop {
            let top = self.header.allocated;
            if top == 0 {
                break;
            }
            if self.records.layout == Layout::United {
                if !self.united_unused(top) {
                    break;
                }
                self.free_list_detach(top);
            } else {
                if !self.free_set.remove(&top) {
                    break;
                }
                self.header.free -= 1;
            }
            self.header.allocated -= 1;
        }
    }

    // The doublets unused-links list: an absolute circular doubly linked
    // list with `previous = source` and `next = target`.
    pub(super) fn free_list_attach_first(&mut self, element: u64) {
        let first = self.header.first_free;
        if first == 0 {
            self.header.first_free = element;
            self.header.last_free = element;
            self.records.set_link(element, element, element);
        } else {
            let previous = self.records.source(first);
            self.records.set_link(element, previous, first);
            let previous_source = self.records.source(previous);
            self.records.set_link(previous, previous_source, element);
            let first_target = self.records.target(first);
            self.records.set_link(first, element, first_target);
            self.header.first_free = element;
        }
        self.header.free += 1;
    }

    pub(super) fn free_list_detach(&mut self, element: u64) {
        let previous = self.records.source(element);
        let next = self.records.target(element);
        if next == element {
            self.header.first_free = 0;
            self.header.last_free = 0;
        } else {
            let previous_source = self.records.source(previous);
            self.records.set_link(previous, previous_source, next);
            let next_target = self.records.target(next);
            self.records.set_link(next, previous, next_target);
            if element == self.header.first_free {
                self.header.first_free = next;
            }
            if element == self.header.last_free {
                self.header.last_free = previous;
            }
        }
        self.records.set_link(element, 0, 0);
        self.header.free -= 1;
    }
}
