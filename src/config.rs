//! Format options (chosen at `mkfs`) and runtime configuration (changeable
//! at any time, without reformatting or redeploying the driver).
//!
//! Runtime configuration is written in links notation, for example
//!
//! ```text
//! (journal: full)
//! (index: adaptive)
//! (bitstring-threshold: auto)
//! ```
//!
//! It is persisted inside the file system (so an external drive carries its
//! settings), can be overridden per mount (`--config file.lino`,
//! `-o key=value`) and can be changed while mounted by writing to the
//! virtual file `/.lfs/config`.

use crate::error::{Error, Result};
use crate::space::Fit;
use links_notation::LiNo;
use std::fmt::Write as _;

/// How links are laid out on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// 64-byte records: the doublets `UnitedMemoryLinks` format, byte for
    /// byte (source, target and both embedded size-balanced trees).
    United,
    /// 16-byte data records plus a separate 48-byte embedded index array.
    Split,
    /// 16-byte data records only; indexing is optional (variable-size index
    /// or none).
    DataOnly,
}

impl Layout {
    /// Encoding in the superblock.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::United => 0,
            Self::Split => 1,
            Self::DataOnly => 2,
        }
    }

    /// Decodes a superblock value.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::United),
            1 => Some(Self::Split),
            2 => Some(Self::DataOnly),
            _ => None,
        }
    }

    /// Whether every link carries an embedded index.
    #[must_use]
    pub const fn has_embedded_index(self) -> bool {
        !matches!(self, Self::DataOnly)
    }

    /// Name used in configuration text.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::United => "united",
            Self::Split => "split",
            Self::DataOnly => "data-only",
        }
    }

    /// Parses a layout name.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "united" | "unified" => Ok(Self::United),
            "split" => Ok(Self::Split),
            "data-only" | "none" => Ok(Self::DataOnly),
            _ => Err(Error::invalid(format!(
                "unknown layout {text:?} (united, split, data-only)"
            ))),
        }
    }
}

/// How file names are stored as links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameEncoding {
    /// Exactly the binary links notation string: `(String chain)` with one
    /// external reference per Unicode code point.
    Chars,
    /// `(String chain)` with up to 7 UTF-8 bytes packed into each external
    /// reference (fewer links, LFS-specific).
    Packed,
}

impl NameEncoding {
    /// Name used in configuration text.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Chars => "chars",
            Self::Packed => "packed",
        }
    }

    /// Parses a name encoding.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "chars" | "lino" => Ok(Self::Chars),
            "packed" => Ok(Self::Packed),
            _ => Err(Error::invalid(format!(
                "unknown name encoding {text:?} (chars, packed)"
            ))),
        }
    }
}

/// Options fixed when the file system is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    /// Block size in bytes (a power of two, at least 1024).
    pub block_size: u32,
    /// Link layout.
    pub layout: Layout,
    /// Name encoding.
    pub name_encoding: NameEncoding,
    /// Journal size in blocks (`None` picks a size from the device size).
    pub journal_blocks: Option<u64>,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            block_size: 4096,
            layout: Layout::United,
            name_encoding: NameEncoding::Chars,
            journal_blocks: None,
        }
    }
}

/// What the journal protects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalMode {
    /// No journal: metadata is overwritten in place (fastest, not crash safe).
    Off,
    /// Metadata is journaled; file data is written in place before the
    /// metadata commit (ordered mode).
    Metadata,
    /// Metadata and overwrites of file data are journaled.
    Full,
}

/// Which optional (variable-size) index is maintained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexMode {
    /// None.
    Off,
    /// A size-balanced tree per link and role.
    Tree,
    /// A bit string per link and role (experimental).
    Bitstring,
    /// Tree or bit string per link, whichever the usage density favours.
    Adaptive,
}

/// Density of users above which a bit string replaces a tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Threshold {
    /// Break-even of the encodings: `1 / (8 * tree node bytes)` = 1/192.
    Auto,
    /// An explicit density in parts per million of all links.
    PartsPerMillion(u32),
}

impl Threshold {
    /// The threshold as a fraction.
    #[must_use]
    pub fn fraction(self) -> f64 {
        match self {
            Self::Auto => 1.0 / 192.0,
            Self::PartsPerMillion(ppm) => f64::from(ppm) / 1_000_000.0,
        }
    }
}

/// Settings that can change at any time.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct RuntimeConfig {
    /// Journal mode.
    pub journal: JournalMode,
    /// Commit automatically after this many operations (0: only on sync).
    pub commit_every: u64,
    /// Placement policy for file extents.
    pub alloc: Fit,
    /// Place link chunks in the smallest gaps between files.
    pub gap_fill: bool,
    /// Extra room reserved when a file grows, in percent of its size.
    pub growth_percent: u32,
    /// Optional variable-size index.
    pub index: IndexMode,
    /// Adaptive index threshold.
    pub bitstring_threshold: Threshold,
    /// A bit string falls back to a tree below this percentage of the threshold.
    pub hysteresis_percent: u32,
    /// Compact automatically when a file cannot grow contiguously.
    pub auto_compact: bool,
    /// Update access times on reads.
    pub atime: bool,
    /// Refuse all modifications.
    pub read_only: bool,
    /// Refuse raw link edits that would break the file system structure.
    pub protect_structure: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            journal: JournalMode::Metadata,
            commit_every: 64,
            alloc: Fit::Best,
            gap_fill: true,
            growth_percent: 25,
            index: IndexMode::Off,
            bitstring_threshold: Threshold::Auto,
            hysteresis_percent: 50,
            auto_compact: true,
            atime: false,
            read_only: false,
            protect_structure: true,
        }
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(Error::invalid(format!(
            "{key}: expected on/off, got {value:?}"
        ))),
    }
}

fn parse_number<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| Error::invalid(format!("{key}: expected a number, got {value:?}")))
}

const fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

/// Every runtime key, for help texts.
pub const RUNTIME_KEYS: &[(&str, &str)] = &[
    ("journal", "off | metadata | full"),
    (
        "commit-every",
        "operations between automatic commits (0 = only on sync)",
    ),
    ("alloc", "best-fit | first-fit (file extents)"),
    (
        "gap-fill",
        "on | off (put link chunks into the smallest gaps)",
    ),
    ("growth-percent", "extra room reserved when a file grows"),
    (
        "index",
        "off | tree | bitstring | adaptive (variable-size index)",
    ),
    ("bitstring-threshold", "auto | <percent of all links>"),
    (
        "hysteresis-percent",
        "fall back to a tree below this % of the threshold",
    ),
    ("auto-compact", "on | off"),
    ("atime", "on | off"),
    ("read-only", "on | off"),
    ("protect-structure", "on | off"),
];

impl RuntimeConfig {
    /// Sets one key.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let value = value.trim();
        match key.trim() {
            "journal" => {
                self.journal = match value {
                    "off" | "none" => JournalMode::Off,
                    "metadata" | "ordered" => JournalMode::Metadata,
                    "full" | "data" => JournalMode::Full,
                    _ => return Err(Error::invalid(format!("journal: unknown mode {value:?}"))),
                };
            }
            "commit-every" => self.commit_every = parse_number(key, value)?,
            "alloc" => {
                self.alloc = match value {
                    "best-fit" | "best" => Fit::Best,
                    "first-fit" | "first" => Fit::First,
                    _ => return Err(Error::invalid(format!("alloc: unknown policy {value:?}"))),
                };
            }
            "gap-fill" => self.gap_fill = parse_bool(key, value)?,
            "growth-percent" => self.growth_percent = parse_number(key, value)?,
            "index" => {
                self.index = match value {
                    "off" | "none" => IndexMode::Off,
                    "tree" | "trees" => IndexMode::Tree,
                    "bitstring" | "bitstrings" => IndexMode::Bitstring,
                    "adaptive" => IndexMode::Adaptive,
                    _ => return Err(Error::invalid(format!("index: unknown mode {value:?}"))),
                };
            }
            "bitstring-threshold" => {
                self.bitstring_threshold = if value == "auto" {
                    Threshold::Auto
                } else {
                    let percent: f64 = parse_number(key, value.trim_end_matches('%'))?;
                    if !(0.0..=100.0).contains(&percent) {
                        return Err(Error::invalid("bitstring-threshold must be within 0..=100"));
                    }
                    Threshold::PartsPerMillion((percent * 10_000.0).round() as u32)
                };
            }
            "hysteresis-percent" => {
                let percent: u32 = parse_number(key, value)?;
                if percent > 100 {
                    return Err(Error::invalid("hysteresis-percent must be within 0..=100"));
                }
                self.hysteresis_percent = percent;
            }
            "auto-compact" => self.auto_compact = parse_bool(key, value)?,
            "atime" => self.atime = parse_bool(key, value)?,
            "read-only" => self.read_only = parse_bool(key, value)?,
            "protect-structure" => self.protect_structure = parse_bool(key, value)?,
            other => {
                return Err(Error::invalid(format!(
                    "unknown configuration key {other:?}"
                )))
            }
        }
        Ok(())
    }

    /// Applies `key=value` or `key:value`.
    pub fn set_assignment(&mut self, assignment: &str) -> Result<()> {
        let (key, value) = assignment
            .split_once('=')
            .or_else(|| assignment.split_once(':'))
            .ok_or_else(|| Error::invalid(format!("expected key=value, got {assignment:?}")))?;
        self.set(key, value)
    }

    /// Applies every setting found in links notation text.
    ///
    /// Accepted shapes: `(key: value)`, `(key value)`, `key: value` lines and
    /// any of them nested inside a wrapper link such as `(lfs ...)`.
    pub fn apply_lino(&mut self, text: &str) -> Result<()> {
        if text.trim().is_empty() {
            return Ok(());
        }
        let links = links_notation::parse_lino_to_links(text)
            .map_err(|error| Error::invalid(format!("configuration: {error}")))?;
        for link in &links {
            self.apply_link(link)?;
        }
        Ok(())
    }

    fn apply_link(&mut self, link: &LiNo<String>) -> Result<()> {
        let LiNo::Link { id, values } = link else {
            return Ok(());
        };
        let refs: Vec<&str> = values
            .iter()
            .filter_map(|value| match value {
                LiNo::Ref(text) => Some(text.as_str()),
                LiNo::Link { .. } => None,
            })
            .collect();
        match (id, refs.as_slice()) {
            (Some(key), [value]) if values.len() == 1 => return self.set(key, value),
            (None, [key, value]) if values.len() == 2 => return self.set(key, value),
            _ => {}
        }
        for value in values {
            self.apply_link(value)?;
        }
        Ok(())
    }

    /// All settings as `(key, value)` pairs.
    #[must_use]
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        let journal = match self.journal {
            JournalMode::Off => "off",
            JournalMode::Metadata => "metadata",
            JournalMode::Full => "full",
        };
        let index = match self.index {
            IndexMode::Off => "off",
            IndexMode::Tree => "tree",
            IndexMode::Bitstring => "bitstring",
            IndexMode::Adaptive => "adaptive",
        };
        let threshold = match self.bitstring_threshold {
            Threshold::Auto => "auto".to_string(),
            Threshold::PartsPerMillion(ppm) => format!("{}", f64::from(ppm) / 10_000.0),
        };
        vec![
            ("journal", journal.to_string()),
            ("commit-every", self.commit_every.to_string()),
            (
                "alloc",
                match self.alloc {
                    Fit::Best => "best-fit",
                    Fit::First => "first-fit",
                }
                .to_string(),
            ),
            ("gap-fill", on_off(self.gap_fill).to_string()),
            ("growth-percent", self.growth_percent.to_string()),
            ("index", index.to_string()),
            ("bitstring-threshold", threshold),
            ("hysteresis-percent", self.hysteresis_percent.to_string()),
            ("auto-compact", on_off(self.auto_compact).to_string()),
            ("atime", on_off(self.atime).to_string()),
            ("read-only", on_off(self.read_only).to_string()),
            (
                "protect-structure",
                on_off(self.protect_structure).to_string(),
            ),
        ]
    }

    /// The configuration as links notation, one `(key: value)` per line.
    #[must_use]
    pub fn to_lino(&self) -> String {
        self.pairs()
            .into_iter()
            .fold(String::new(), |mut out, (key, value)| {
                let _ = writeln!(out, "({key}: {value})");
                out
            })
    }

    /// Placement policy for link chunks.
    #[must_use]
    pub const fn chunk_fit(&self) -> Fit {
        if self.gap_fill {
            Fit::Best
        } else {
            Fit::First
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lino_round_trip() {
        let mut config = RuntimeConfig {
            journal: JournalMode::Full,
            index: IndexMode::Adaptive,
            bitstring_threshold: Threshold::PartsPerMillion(12_500),
            ..RuntimeConfig::default()
        };
        config.atime = true;
        let text = config.to_lino();
        let mut parsed = RuntimeConfig::default();
        parsed.apply_lino(&text).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn accepts_several_shapes() {
        let mut config = RuntimeConfig::default();
        config
            .apply_lino("(lfs (journal full) (index: tree))\n(gap-fill: off)")
            .unwrap();
        assert_eq!(config.journal, JournalMode::Full);
        assert_eq!(config.index, IndexMode::Tree);
        assert!(!config.gap_fill);
        config.set_assignment("bitstring-threshold=2.5%").unwrap();
        assert_eq!(
            config.bitstring_threshold,
            Threshold::PartsPerMillion(25_000)
        );
        assert!(config.set("journal", "sometimes").is_err());
        assert!(config.set("colour", "blue").is_err());
    }

    #[test]
    fn auto_threshold_is_the_break_even_density() {
        assert!((Threshold::Auto.fraction() - 1.0 / 192.0).abs() < f64::EPSILON);
    }
}
