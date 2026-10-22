//! Deterministic demo volumes: any configuration, a reproducible workload.
//!
//! The web visualizer (through the WebAssembly build), the `lfs demo`
//! command and the configuration-matrix tests all build volumes with
//! [`Demo`]: format options and runtime keys pick the configuration, the
//! workload keys say how many directories, files, symbolic links, hard links
//! and user links to create, how many files to delete afterwards (leaving
//! holes) and whether to compact. The same parameters always build the same
//! layout.
//!
//! Parameters are `key=value` lines (or `;`-separated), for example
//! `size=4M; block-size=1024; layout=split; index=adaptive; files=20`.

use crate::config::{FormatOptions, Layout, NameEncoding, RUNTIME_KEYS};
use crate::device::{BlockDevice, MemDevice};
use crate::error::{Error, Result};
use crate::fs::{Creator, Fs, SetAttr, ROOT_INO};

/// Workload and configuration of a demo volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demo {
    /// Image size in bytes.
    pub size: u64,
    /// Format options.
    pub options: FormatOptions,
    /// Volume label.
    pub label: String,
    /// Runtime keys applied (and persisted) before the workload.
    pub runtime: Vec<(String, String)>,
    /// Directories under the root.
    pub dirs: usize,
    /// Regular files, spread over the root and the directories.
    pub files: usize,
    /// Largest file size in bytes (sizes vary from 1 byte to this).
    pub file_size: u64,
    /// Symbolic links.
    pub symlinks: usize,
    /// Extra hard links to existing files.
    pub hardlinks: usize,
    /// User links of the embedded database (beyond the file system's own).
    pub user_links: usize,
    /// Percentage of the files deleted after the workload (leaves holes).
    pub delete_percent: u32,
    /// Whether to compact after the deletions.
    pub compact: bool,
    /// Seed of the pseudo-random sizes and choices.
    pub seed: u64,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            size: 4 << 20,
            options: FormatOptions::default(),
            label: "demo".into(),
            runtime: Vec::new(),
            dirs: 3,
            files: 12,
            file_size: 12_000,
            symlinks: 2,
            hardlinks: 1,
            user_links: 16,
            delete_percent: 25,
            compact: false,
            seed: 1,
        }
    }
}

/// The workload keys with their help, for documentation and the visualizer.
pub const DEMO_KEYS: &[(&str, &str)] = &[
    ("size", "image size in bytes (64K, 4M, ...)"),
    ("block-size", "block size: 1024 to 65536, a power of two"),
    ("layout", "united, split or data-only"),
    ("names", "name encoding: chars or packed"),
    (
        "journal-blocks",
        "journal size in blocks (0: none, auto: from the size)",
    ),
    ("label", "volume label"),
    ("dirs", "directories under the root"),
    ("files", "regular files"),
    ("file-size", "largest file size in bytes"),
    ("symlinks", "symbolic links"),
    ("hardlinks", "extra hard links"),
    ("user-links", "user links in the embedded database"),
    ("delete-percent", "percentage of files deleted afterwards"),
    ("compact", "compact after deleting (true/false)"),
    ("seed", "seed of the pseudo-random workload"),
];

fn number<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| Error::invalid(format!("{key}: {value:?} is not a number")))
}

/// Parses `4096`, `64K`, `4M`, `1G`.
pub fn parse_size(text: &str) -> Result<u64> {
    let text = text.trim();
    let (digits, shift) = match text.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&text[..text.len() - 1], 10),
        Some('M') => (&text[..text.len() - 1], 20),
        Some('G') => (&text[..text.len() - 1], 30),
        _ => (text, 0),
    };
    digits
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(1 << shift))
        .ok_or_else(|| Error::invalid(format!("bad size {text:?}")))
}

fn boolean(key: &str, value: &str) -> Result<bool> {
    match value {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(Error::invalid(format!(
            "{key}: {value:?} is not true or false"
        ))),
    }
}

impl Demo {
    /// Parses `key=value` lines (or `;`-separated assignments) over the
    /// defaults; runtime keys go to [`runtime`](Self::runtime).
    pub fn parse(text: &str) -> Result<Self> {
        let mut demo = Self::default();
        for assignment in text.split(['\n', ';']) {
            let assignment = assignment.trim();
            if assignment.is_empty() || assignment.starts_with('#') {
                continue;
            }
            let (key, value) = assignment
                .split_once('=')
                .ok_or_else(|| Error::invalid(format!("expected key=value, got {assignment:?}")))?;
            demo.set(key.trim(), value.trim())?;
        }
        Ok(demo)
    }

    /// Sets one parameter.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "size" => self.size = parse_size(value)?,
            "block-size" => self.options.block_size = number(key, value)?,
            "layout" => self.options.layout = Layout::parse(value)?,
            "names" => self.options.name_encoding = NameEncoding::parse(value)?,
            "journal-blocks" => {
                self.options.journal_blocks = if value == "auto" {
                    None
                } else {
                    Some(number(key, value)?)
                };
            }
            "label" => self.label = value.into(),
            "dirs" => self.dirs = number(key, value)?,
            "files" => self.files = number(key, value)?,
            "file-size" => self.file_size = parse_size(value)?,
            "symlinks" => self.symlinks = number(key, value)?,
            "hardlinks" => self.hardlinks = number(key, value)?,
            "user-links" => self.user_links = number(key, value)?,
            "delete-percent" => self.delete_percent = number::<u32>(key, value)?.min(100),
            "compact" => self.compact = boolean(key, value)?,
            "seed" => self.seed = number(key, value)?,
            _ if RUNTIME_KEYS.iter().any(|(name, _)| *name == key) => {
                self.runtime.retain(|(name, _)| name != key);
                self.runtime.push((key.into(), value.into()));
            }
            _ => return Err(Error::invalid(format!("unknown demo key {key:?}"))),
        }
        Ok(())
    }

    /// Formats an in-memory volume and runs the workload on it.
    pub fn build(&self) -> Result<Fs<MemDevice>> {
        let mut fs = Fs::format(MemDevice::new(self.size), &self.options, &self.label)?;
        self.run(&mut fs)?;
        Ok(fs)
    }

    /// Runs the configuration and the workload on a formatted volume, then
    /// commits.
    pub fn run<D: BlockDevice>(&self, fs: &mut Fs<D>) -> Result<()> {
        for (key, value) in &self.runtime {
            fs.set_config_value(key, value)?;
        }
        let mut rng = Rng(self.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let creator = Creator::default();
        let mut parents = vec![(ROOT_INO, String::new())];
        for index in 0..self.dirs {
            let name = format!("dir{index}");
            let attr = fs.mkdir(ROOT_INO, &name, 0o755, creator)?;
            parents.push((attr.ino, format!("/{name}")));
        }
        let mut files = Vec::new();
        for index in 0..self.files {
            let (parent, _) = parents[rng.below(parents.len() as u64) as usize].clone();
            let name = format!("file{index}.bin");
            let attr = fs.create(parent, &name, 0o644, creator)?;
            let len = 1 + rng.below(self.file_size.max(1));
            let data: Vec<u8> = (0..len)
                .map(|at| (at as u8).wrapping_mul(31).wrapping_add(index as u8))
                .collect();
            fs.write(attr.ino, 0, &data)?;
            fs.release(attr.ino)?;
            files.push((parent, name, attr.ino));
        }
        for index in 0..self.symlinks {
            let (parent, _) = parents[rng.below(parents.len() as u64) as usize].clone();
            let target = files
                .get(index % files.len().max(1))
                .map_or_else(|| "nowhere".to_string(), |(_, name, _)| name.clone());
            fs.symlink(parent, &format!("link{index}"), &target, creator)?;
        }
        for (index, (_, _, ino)) in files.iter().take(self.hardlinks).enumerate() {
            fs.link(*ino, ROOT_INO, &format!("hard{index}"))?;
        }
        let mut previous = 0;
        for index in 0..self.user_links {
            previous = if index % 4 == 0 || previous == 0 {
                fs.create_point()?
            } else {
                let source = previous;
                let target = crate::links::external(rng.below(1000));
                fs.create_link(source, target)?
            };
        }
        let deletions = files.len() * self.delete_percent as usize / 100;
        for _ in 0..deletions {
            let (parent, name, _) = files.remove(rng.below(files.len() as u64) as usize);
            fs.unlink(parent, &name)?;
        }
        if let Some((_, _, ino)) = files.first() {
            // A sparse grow leaves room after the data: the visualizer shows
            // the slack.
            let size = fs.getattr(*ino)?.size;
            fs.setattr(
                *ino,
                SetAttr {
                    size: Some(size + 1),
                    ..SetAttr::default()
                },
            )?;
        }
        if self.compact {
            fs.compact()?;
        }
        fs.commit()
    }

    /// The parameters as `key=value` lines (runtime keys last).
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut lines = vec![
            format!("size={}", self.size),
            format!("block-size={}", self.options.block_size),
            format!("layout={}", self.options.layout.name()),
            format!("names={}", self.options.name_encoding.name()),
            format!(
                "journal-blocks={}",
                self.options
                    .journal_blocks
                    .map_or_else(|| "auto".into(), |blocks| blocks.to_string())
            ),
            format!("label={}", self.label),
            format!("dirs={}", self.dirs),
            format!("files={}", self.files),
            format!("file-size={}", self.file_size),
            format!("symlinks={}", self.symlinks),
            format!("hardlinks={}", self.hardlinks),
            format!("user-links={}", self.user_links),
            format!("delete-percent={}", self.delete_percent),
            format!("compact={}", self.compact),
            format!("seed={}", self.seed),
        ];
        lines.extend(
            self.runtime
                .iter()
                .map(|(key, value)| format!("{key}={value}")),
        );
        lines.join("\n")
    }
}

/// xorshift64*: small, deterministic and good enough for workloads.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    const fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next() % bound
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_round_trip_and_reject_unknown_keys() {
        let demo =
            Demo::parse("size=2M; block-size=1024\nlayout=split\nindex=adaptive\nfiles=5").unwrap();
        assert_eq!(demo.size, 2 << 20);
        assert_eq!(demo.options.block_size, 1024);
        assert_eq!(demo.options.layout, Layout::Split);
        assert_eq!(demo.files, 5);
        assert_eq!(demo.runtime, vec![("index".into(), "adaptive".into())]);
        assert_eq!(Demo::parse(&demo.to_text()).unwrap(), demo);
        assert!(Demo::parse("colour=blue").is_err());
        assert!(Demo::parse("files").is_err());
    }

    #[test]
    fn the_same_parameters_build_the_same_layout() {
        let demo = Demo::parse("size=1M; block-size=1024; files=8; user-links=9").unwrap();
        let a = demo.build().unwrap();
        let b = demo.build().unwrap();
        assert_eq!(a.space_map(), b.space_map());
        assert!(a.check().is_empty(), "{:?}", a.check());
        assert_eq!(a.statfs().files, b.statfs().files);
    }
}
