//! File names as links-notation strings.
//!
//! A name is the binary links notation string `(STRING chain)`, where the
//! chain is a nil-terminated cons list `(c1 (c2 (... (cn 0))))` of external
//! references. With [`NameEncoding::Chars`] every element is one Unicode
//! code point, exactly as binary links notation stores strings; with
//! [`NameEncoding::Packed`] every element packs up to seven UTF-8 bytes
//! (`length << 56 | bytes`), so a 21-byte name takes 3 links instead of 21.
//!
//! Chains are built from the end, so names share suffix links and every
//! name is stored once, however many directories use it.

use crate::config::NameEncoding;
use crate::error::{Error, Result};
use crate::links::{as_external, external, Links};
use links_notation::binary::packet::STRING;

/// Longest file name in bytes.
pub const NAME_MAX: usize = 255;
/// Longest symbolic link target in bytes.
pub const TARGET_MAX: usize = 4096;
const CHAIN_LIMIT: usize = TARGET_MAX + 1;

/// Encodes text into chain element values.
#[must_use]
pub fn encode(text: &str, encoding: NameEncoding) -> Vec<u64> {
    match encoding {
        NameEncoding::Chars => text.chars().map(u64::from).collect(),
        NameEncoding::Packed => text
            .as_bytes()
            .chunks(7)
            .map(|chunk| {
                let mut value = (chunk.len() as u64) << 56;
                for (index, byte) in chunk.iter().enumerate() {
                    value |= u64::from(*byte) << (index * 8);
                }
                value
            })
            .collect(),
    }
}

/// Decodes chain element values.
pub fn decode(values: &[u64], encoding: NameEncoding) -> Result<String> {
    match encoding {
        NameEncoding::Chars => values
            .iter()
            .map(|&value| {
                u32::try_from(value)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| Error::corrupt(format!("bad code point {value}")))
            })
            .collect(),
        NameEncoding::Packed => {
            let mut bytes = Vec::with_capacity(values.len() * 7);
            for &value in values {
                let len = (value >> 56) as usize;
                if len == 0 || len > 7 {
                    return Err(Error::corrupt(format!("bad packed name cell {value:#x}")));
                }
                for index in 0..len {
                    bytes.push((value >> (index * 8)) as u8);
                }
            }
            String::from_utf8(bytes).map_err(|_| Error::corrupt("name is not UTF-8"))
        }
    }
}

/// Checks a file name.
pub fn validate(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(Error::invalid(format!("bad file name {name:?}")));
    }
    if name.len() > NAME_MAX {
        return Err(Error::NameTooLong);
    }
    Ok(())
}

/// Finds or creates the string link of `text`.
pub fn intern(links: &mut Links, text: &str, encoding: NameEncoding) -> Result<u64> {
    let mut rest = 0;
    for value in encode(text, encoding).into_iter().rev() {
        rest = links.get_or_create(external(value), rest)?;
    }
    links.get_or_create(STRING, rest)
}

/// Finds the string link of `text` without creating anything.
#[must_use]
pub fn find(links: &Links, text: &str, encoding: NameEncoding) -> Option<u64> {
    let mut rest = 0;
    for value in encode(text, encoding).into_iter().rev() {
        rest = links.search(external(value), rest)?;
    }
    links.search(STRING, rest)
}

/// Whether `link` is a string link.
#[must_use]
pub fn is_string(links: &Links, link: u64) -> bool {
    links.exists(link) && links.source(link) == STRING
}

/// Reads the chain element values of a string link.
pub fn values(links: &Links, link: u64) -> Result<Vec<u64>> {
    if !is_string(links, link) {
        return Err(Error::corrupt(format!("link {link} is not a string")));
    }
    let mut out = Vec::new();
    let mut cell = links.target(link);
    while cell != 0 {
        if out.len() > CHAIN_LIMIT || !links.exists(cell) {
            return Err(Error::corrupt(format!("string {link} has a broken chain")));
        }
        let value = as_external(links.source(cell))
            .ok_or_else(|| Error::corrupt(format!("string {link} has a non-external element")))?;
        out.push(value);
        cell = links.target(cell);
    }
    Ok(out)
}

/// Reads the text of a string link.
pub fn read(links: &Links, link: u64, encoding: NameEncoding) -> Result<String> {
    decode(&values(links, link)?, encoding)
}

/// Deletes a string link and its chain cells if nothing uses them any more.
pub fn release(links: &mut Links, link: u64) -> Result<()> {
    if !links.exists(link) || links.references(link) > 0 {
        return Ok(());
    }
    let mut cell = links.target(link);
    links.delete(link)?;
    while cell != 0 && links.exists(cell) && links.references(cell) == 0 {
        let next = links.target(cell);
        links.delete(cell)?;
        cell = next;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Layout;

    #[test]
    fn encodings_round_trip() {
        for encoding in [NameEncoding::Chars, NameEncoding::Packed] {
            for text in ["a", "hello.txt", "Привет, мир ✓", "0123456789abcdef0123"] {
                assert_eq!(decode(&encode(text, encoding), encoding).unwrap(), text);
            }
        }
        assert_eq!(encode("abcdefgh", NameEncoding::Packed).len(), 2);
    }

    #[test]
    fn names_share_suffixes_and_are_released() {
        for encoding in [NameEncoding::Chars, NameEncoding::Packed] {
            let mut links = Links::new(Layout::United, 1024);
            for _ in 0..5 {
                links.create_point().unwrap();
            }
            let base = links.count();
            let a = intern(&mut links, "readme", encoding).unwrap();
            let b = intern(&mut links, "readme", encoding).unwrap();
            assert_eq!(a, b);
            assert_eq!(read(&links, a, encoding).unwrap(), "readme");
            assert_eq!(find(&links, "readme", encoding), Some(a));
            assert_eq!(find(&links, "readm", encoding), None);
            let c = intern(&mut links, "xreadme", encoding).unwrap();
            assert_ne!(a, c);
            release(&mut links, c).unwrap();
            assert_eq!(read(&links, a, encoding).unwrap(), "readme");
            release(&mut links, a).unwrap();
            assert_eq!(links.count(), base);
        }
    }
}
