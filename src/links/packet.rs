//! Import and export of the links store.
//!
//! * **Store archive** — the whole store in [binary links notation] version 1,
//!   exactly as link-cli writes it (`--export-binary`, spec §10): a links
//!   packet without external references and with packed widths, where every
//!   link keeps its address and unused addresses are section gaps, followed
//!   by a names packet (external references, packed widths) holding
//!   `address code-point…` per named link. The raw cells of external
//!   references are 64-bit Hybrid values (`2^64 - v`), so they travel as
//!   8-byte references and come back unchanged.
//! * **Doublets file** — the `.links` file of the doublets library
//!   (`UnitedMemoryLinks<u64>`): 64-byte records, the header in record 0.
//!   It is byte for byte the record array of the united layout.
//! * **`LiNo` text** — one `(index: source target)` link per line, with `$v`
//!   for the external value `v`.
//!
//! [`Links::place_all`] imports at the original addresses (into a store
//! where they are free); [`import_remapped`] gives the links new addresses
//! and reuses links that already exist, which is how a mounted volume
//! takes foreign links without touching its own.
//!
//! [binary links notation]: https://github.com/link-foundation/links-notation/blob/main/docs/protocol/binary-links-notation.md

use super::{format_ref, is_internal, parse_ref, Link, Links};
use crate::config::Layout;
use crate::error::{Error, Result};
use links_notation::binary::{DecodeLimits, LinksPacket, Reference};
use links_notation::LiNo;
use std::collections::HashMap;
use std::fmt::Write;

/// Size of one doublets record.
pub const DOUBLETS_RECORD: usize = 64;

/// The supported interchange formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Binary links notation store archive.
    Archive,
    /// Doublets `UnitedMemoryLinks<u64>` file.
    Doublets,
    /// Links notation text.
    Lino,
}

impl Format {
    /// Every format.
    pub const ALL: [Self; 3] = [Self::Archive, Self::Doublets, Self::Lino];

    /// The name used on the command line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Archive => "binary",
            Self::Doublets => "doublets",
            Self::Lino => "lino",
        }
    }

    /// Parses a format name.
    pub fn parse(text: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.name() == text)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "unknown links format {text:?} (binary, doublets or lino)"
                ))
            })
    }

    /// Guesses the format of `bytes`: UTF-8 text that starts like links
    /// notation, else a valid store archive, else a doublets file.
    #[must_use]
    pub fn detect(bytes: &[u8]) -> Self {
        let text = !bytes.contains(&0)
            && std::str::from_utf8(bytes).is_ok_and(|text| {
                text.trim_start()
                    .chars()
                    .next()
                    .is_some_and(|c| c == '(' || c == '#' || c.is_ascii_digit())
            });
        if text {
            Self::Lino
        } else if read_archive(bytes).is_ok() {
            Self::Archive
        } else {
            Self::Doublets
        }
    }

    /// The format a file name suggests (`.links`, `.lino`, anything else is
    /// an archive).
    #[must_use]
    pub fn of_path(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "links" => Some(Self::Doublets),
            "lino" | "txt" => Some(Self::Lino),
            "lnb" | "bin" => Some(Self::Archive),
            _ => None,
        }
    }
}

/// A decoded store archive.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Archive {
    /// The links in address order.
    pub links: Vec<Link>,
    /// Names of links, in address order.
    pub names: Vec<(u64, String)>,
}

fn binary_error(error: impl std::fmt::Display) -> Error {
    Error::invalid(format!("binary links notation: {error}"))
}

/// Writes a store archive of `links` with the given names (in address order).
pub fn write_archive(links: &[Link], names: &[(u64, String)]) -> Result<Vec<u8>> {
    let doublets: Vec<(u64, Vec<Reference>)> = links
        .iter()
        .map(|link| {
            (
                link.index,
                vec![
                    Reference::Internal(link.source),
                    Reference::Internal(link.target),
                ],
            )
        })
        .collect();
    let labels: Vec<(u64, Vec<Reference>)> = names
        .iter()
        .enumerate()
        .map(|(index, (address, name))| {
            let refs = std::iter::once(*address)
                .chain(name.chars().map(u64::from))
                .map(Reference::External)
                .collect();
            (index as u64 + 1, refs)
        })
        .collect();
    let mut out = LinksPacket::pack(false, &doublets, true)
        .and_then(|packet| packet.to_bytes())
        .map_err(binary_error)?;
    out.extend(
        LinksPacket::pack(true, &labels, true)
            .and_then(|packet| packet.to_bytes())
            .map_err(binary_error)?,
    );
    Ok(out)
}

/// Reads a store archive.
///
/// Rejects anything the spec rejects: a short archive or trailing bytes, a
/// link that is not a doublet of internal references, and a name holding
/// an internal reference or a non-character.
pub fn read_archive(bytes: &[u8]) -> Result<Archive> {
    let mut reader = bytes;
    let limits = DecodeLimits::unlimited();
    let mut next = || -> Result<LinksPacket> {
        LinksPacket::read_from(&mut reader, &limits)
            .map_err(binary_error)?
            .ok_or_else(|| Error::invalid("the store archive is cut short"))
    };
    let links_packet = next()?;
    let names_packet = next()?;
    if !reader.is_empty() {
        return Err(Error::invalid("bytes after the store archive"));
    }
    let mut archive = Archive::default();
    for (index, refs) in links_packet.links() {
        let [Reference::Internal(source), Reference::Internal(target)] = refs else {
            return Err(Error::invalid(format!(
                "archive link {index} is not a doublet of internal references"
            )));
        };
        archive.links.push(Link {
            index,
            source: *source,
            target: *target,
        });
    }
    for (_, refs) in names_packet.links() {
        let mut values = refs.iter().map(|reference| match reference {
            Reference::External(value) => Ok(*value),
            Reference::Internal(_) => Err(Error::invalid("a name holds an internal reference")),
        });
        let address = values.next().transpose()?.unwrap_or_default();
        let name = values
            .map(|value| {
                value.and_then(|value| {
                    u32::try_from(value)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| Error::invalid(format!("bad code point {value} in a name")))
                })
            })
            .collect::<Result<String>>()?;
        archive.names.push((address, name));
    }
    Ok(archive)
}

/// Writes the links as a doublets `UnitedMemoryLinks<u64>` file. The store
/// may use any layout; holes become doublets free slots.
pub fn write_doublets(links: &[Link]) -> Result<Vec<u8>> {
    let mut united = Links::new(Layout::United, 4096);
    united.place_all(links)?;
    let top = united.allocated();
    let mut out = Vec::with_capacity((top as usize + 1) * DOUBLETS_RECORD);
    for index in 0..=top {
        out.extend_from_slice(united.records.data.record(index));
    }
    // `reserved` is the capacity of the file in links, as doublets keeps it.
    out[8..16].copy_from_slice(&top.to_le_bytes());
    Ok(out)
}

/// Reads the live links of a doublets `UnitedMemoryLinks<u64>` file.
pub fn read_doublets(bytes: &[u8]) -> Result<Vec<Link>> {
    if bytes.len() < DOUBLETS_RECORD || !bytes.len().is_multiple_of(DOUBLETS_RECORD) {
        return Err(Error::invalid(
            "a doublets file is a whole number of 64-byte records",
        ));
    }
    let cell = |record: u64, index: usize| {
        let offset = record as usize * DOUBLETS_RECORD + index * 8;
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap_or_default())
    };
    let allocated = cell(0, 0);
    let first_free = cell(0, 3);
    let records = (bytes.len() / DOUBLETS_RECORD) as u64;
    if allocated >= records {
        return Err(Error::invalid(format!(
            "the doublets header claims {allocated} links but the file holds {}",
            records - 1
        )));
    }
    // The doublets `is_unused` rule: the first free slot, and slots with an
    // empty sources tree and a non-null source (the free list's previous
    // pointer), are unused.
    Ok((1..=allocated)
        .filter(|&index| index != first_free && (cell(index, 4) != 0 || cell(index, 0) == 0))
        .map(|index| Link {
            index,
            source: cell(index, 0),
            target: cell(index, 1),
        })
        .collect())
}

/// Every link as `(index: source target)`, one per line.
#[must_use]
pub fn write_lino(links: &[Link]) -> String {
    let mut out = String::new();
    for link in links {
        let _ = writeln!(
            out,
            "({}: {} {})",
            link.index,
            format_ref(link.source),
            format_ref(link.target)
        );
    }
    out
}

fn lino_ref(value: &LiNo<String>) -> Result<u64> {
    match value {
        LiNo::Ref(text) => parse_ref(text),
        LiNo::Link { .. } => Err(Error::invalid("nested links are not doublets")),
    }
}

/// Reads `(index: source target)` links; `index` is required.
pub fn read_lino(text: &str) -> Result<Vec<Link>> {
    let document = links_notation::parse_lino_to_links(text)
        .map_err(|error| Error::invalid(format!("links notation: {error}")))?;
    let mut links = Vec::with_capacity(document.len());
    for item in &document {
        let LiNo::Link {
            id: Some(id),
            values,
        } = item
        else {
            return Err(Error::invalid(format!(
                "expected (index: source target), found {item:?}"
            )));
        };
        let [source, target] = values.as_slice() else {
            return Err(Error::invalid(format!("link {id} is not a doublet")));
        };
        let index = parse_ref(id)?;
        if !is_internal(index) {
            return Err(Error::invalid(format!("bad link index {id}")));
        }
        links.push(Link {
            index,
            source: lino_ref(source)?,
            target: lino_ref(target)?,
        });
    }
    links.sort_unstable_by_key(|link| link.index);
    if links.windows(2).any(|pair| pair[0].index == pair[1].index) {
        return Err(Error::invalid("a link index appears twice"));
    }
    Ok(links)
}

/// Reads links in any format.
pub fn read(format: Format, bytes: &[u8]) -> Result<Vec<Link>> {
    match format {
        Format::Archive => read_archive(bytes).map(|archive| archive.links),
        Format::Doublets => read_doublets(bytes),
        Format::Lino => read_lino(
            std::str::from_utf8(bytes).map_err(|_| Error::invalid("links text is not UTF-8"))?,
        ),
    }
}

/// Writes links in any format (`names` only go into archives).
pub fn write(format: Format, links: &[Link], names: &[(u64, String)]) -> Result<Vec<u8>> {
    match format {
        Format::Archive => write_archive(links, names),
        Format::Doublets => write_doublets(links),
        Format::Lino => Ok(write_lino(links).into_bytes()),
    }
}

/// Strongly connected components of the reference graph, dependencies
/// first (iterative Tarjan, so long chains do not exhaust the stack).
fn components(links: &[Link], position: &HashMap<u64, usize>) -> Vec<Vec<usize>> {
    const NONE: usize = usize::MAX;
    let deps = |node: usize| {
        let link = links[node];
        [link.source, link.target]
            .into_iter()
            .filter_map(|end| position.get(&end).copied())
    };
    let count = links.len();
    let (mut index, mut low) = (vec![NONE; count], vec![0; count]);
    let mut on_stack = vec![false; count];
    let (mut stack, mut out, mut next) = (Vec::new(), Vec::new(), 0);
    for root in 0..count {
        if index[root] != NONE {
            continue;
        }
        let mut work = vec![(root, 0)];
        while let Some((node, edge)) = work.pop() {
            if edge == 0 {
                index[node] = next;
                low[node] = next;
                next += 1;
                stack.push(node);
                on_stack[node] = true;
            }
            if let Some(dep) = deps(node).nth(edge) {
                work.push((node, edge + 1));
                if index[dep] == NONE {
                    work.push((dep, 0));
                } else if on_stack[dep] {
                    low[node] = low[node].min(index[dep]);
                }
                continue;
            }
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                out.push(component);
            }
        }
    }
    out
}

/// Imports links under new addresses and returns the map from the old
/// addresses to the new ones.
///
/// Every internal reference must name one of `links`. A link that does not
/// refer to itself, directly or through a cycle, is looked up first and only
/// created if missing, so importing the same links twice adds nothing.
/// Points and cycles are created anew at fresh addresses. `fixed` maps
/// imported links onto existing ones instead (the marker points of an
/// archive's names, say); they are not created. Nothing changes if a check
/// fails.
pub fn import_remapped(
    store: &mut Links,
    links: &[Link],
    fixed: &HashMap<u64, u64, impl std::hash::BuildHasher>,
) -> Result<HashMap<u64, u64>> {
    let position: HashMap<u64, usize> = links
        .iter()
        .enumerate()
        .map(|(position, link)| (link.index, position))
        .collect();
    if position.len() != links.len() {
        return Err(Error::invalid("a link index appears twice"));
    }
    let mut pairs = std::collections::HashSet::with_capacity(links.len());
    for link in links {
        if link.source == 0 && link.target == 0 {
            return Err(Error::invalid("a link needs a non-null end"));
        }
        if !pairs.insert((link.source, link.target)) {
            return Err(Error::invalid(format!(
                "the pair ({} {}) appears twice",
                format_ref(link.source),
                format_ref(link.target)
            )));
        }
        for end in [link.source, link.target] {
            if is_internal(end) && !position.contains_key(&end) {
                return Err(Error::invalid(format!(
                    "link {} refers to link {end}, which is not imported",
                    link.index
                )));
            }
        }
    }
    for (&from, &to) in fixed {
        if !position.contains_key(&from) || !store.exists(to) {
            return Err(Error::invalid(format!(
                "cannot map link {from} onto link {to}"
            )));
        }
    }
    let mut map: HashMap<u64, u64> = HashMap::with_capacity(links.len());
    map.extend(fixed.iter().map(|(&from, &to)| (from, to)));
    let translate = |map: &HashMap<u64, u64>, end: u64| {
        if is_internal(end) {
            map.get(&end).copied().unwrap_or(end)
        } else {
            end
        }
    };
    for mut component in components(links, &position) {
        component.retain(|&member| !fixed.contains_key(&links[member].index));
        let Some(&first) = component.first() else {
            continue;
        };
        let link = links[first];
        let cyclic = component.len() > 1 || link.source == link.index || link.target == link.index;
        if !cyclic {
            let (source, target) = (translate(&map, link.source), translate(&map, link.target));
            map.insert(link.index, store.get_or_create(source, target)?);
            continue;
        }
        let mut next = store.allocated();
        for &member in &component {
            next += 1;
            map.insert(links[member].index, next);
        }
        let mut placed: Vec<Link> = component
            .iter()
            .map(|&member| {
                let link = links[member];
                Link {
                    index: map[&link.index],
                    source: translate(&map, link.source),
                    target: translate(&map, link.target),
                }
            })
            .collect();
        placed.sort_unstable_by_key(|link| link.index);
        store.place_all(&placed)?;
    }
    Ok(map)
}

#[cfg(test)]
mod tests;
