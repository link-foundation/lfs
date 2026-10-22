//! The `lfs` commands.

mod copy;
mod mount;

use crate::{Cli, Command, LinkAction};
use links_fs::config::{FormatOptions, Layout, NameEncoding, RUNTIME_KEYS};
use links_fs::demo::{Demo, DEMO_KEYS};
use links_fs::device::FileDevice;
use links_fs::error::Error;
use links_fs::fs::{Creator, FileKind, Fs, ROOT_INO};
use links_fs::links::packet::{self, Format};
use links_fs::links::{format_ref, parse_ref};
use std::fmt::{self, Write as _};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

/// A failed command.
#[derive(Debug)]
pub enum Failure {
    /// A file system error, with what was being done.
    Fs(String, Error),
    /// A host I/O error, with what was being done.
    Io(String, io::Error),
    /// Anything else.
    Message(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fs(context, error) => write!(f, "{context}: {error}"),
            Self::Io(context, error) => write!(f, "{context}: {error}"),
            Self::Message(message) => f.write_str(message),
        }
    }
}

/// Command result.
pub type Outcome<T> = std::result::Result<T, Failure>;

/// Adds context to errors.
pub trait Context<T> {
    /// Says what was being done when the error happened.
    fn context(self, what: impl FnOnce() -> String) -> Outcome<T>;
}

impl<T> Context<T> for links_fs::error::Result<T> {
    fn context(self, what: impl FnOnce() -> String) -> Outcome<T> {
        self.map_err(|error| Failure::Fs(what(), error))
    }
}

impl<T> Context<T> for io::Result<T> {
    fn context(self, what: impl FnOnce() -> String) -> Outcome<T> {
        self.map_err(|error| Failure::Io(what(), error))
    }
}

/// Whether the failure is a closed standard output (`lfs ls | head`).
pub fn is_broken_pipe(failure: &Failure) -> bool {
    match failure {
        Failure::Io(_, error) | Failure::Fs(_, Error::Io(error)) => {
            error.kind() == io::ErrorKind::BrokenPipe
        }
        _ => false,
    }
}

/// Parses `4096`, `64K`, `64M`, `2G`, `1T` (binary units).
pub fn parse_size(text: &str) -> std::result::Result<u64, String> {
    let text = text.trim();
    let (digits, shift) = match text.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&text[..text.len() - 1], 10),
        Some('M') => (&text[..text.len() - 1], 20),
        Some('G') => (&text[..text.len() - 1], 30),
        Some('T') => (&text[..text.len() - 1], 40),
        _ => (text, 0),
    };
    digits
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(1 << shift))
        .ok_or_else(|| format!("bad size {text:?} (try 64M or 2G)"))
}

/// Parses a link reference: `12` or `$7`.
pub fn parse_link_ref(text: &str) -> std::result::Result<u64, String> {
    parse_ref(text).map_err(|error| error.to_string())
}

/// Parses a blueprint depth: a number or `all`.
pub fn parse_depth(text: &str) -> std::result::Result<usize, String> {
    if text == "all" {
        Ok(usize::MAX)
    } else {
        text.parse()
            .map_err(|_| format!("bad depth {text:?} (a number or all)"))
    }
}

/// The demo keys, for `lfs demo --help`.
pub fn demo_help() -> String {
    let out = DEMO_KEYS
        .iter()
        .fold(String::from("Demo keys:\n"), |mut out, (key, help)| {
            let _ = writeln!(out, "  {key:<22} {help}");
            out
        });
    format!("{out}\nAny runtime key (see `lfs config --help`) is accepted too.")
}

/// Parses `--at`: child indices (`3/0/2`) or `@OFFSET`.
fn blueprint_path<D: links_fs::device::BlockDevice>(
    fs: &mut Fs<D>,
    at: Option<&str>,
) -> Outcome<Vec<usize>> {
    let Some(at) = at else {
        return Ok(Vec::new());
    };
    if let Some(offset) = at.strip_prefix('@') {
        let offset = offset
            .strip_prefix("0x")
            .map_or_else(|| offset.parse(), |hex| u64::from_str_radix(hex, 16))
            .map_err(|_| Failure::Message(format!("bad offset {offset:?}")))?;
        return fs
            .blueprint_locate(offset)
            .context(|| format!("locating byte {offset}"));
    }
    at.split('/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse()
                .map_err(|_| Failure::Message(format!("bad blueprint path {at:?}")))
        })
        .collect()
}

/// The runtime keys, for `lfs config --help`.
pub fn config_help() -> String {
    RUNTIME_KEYS
        .iter()
        .fold(String::from("Runtime keys:\n"), |mut out, (key, help)| {
            let _ = writeln!(out, "  {key:<22} {help}");
            out
        })
}

fn overrides(cli_set: &[String]) -> Outcome<Vec<(String, String)>> {
    cli_set
        .iter()
        .map(|assignment| {
            assignment
                .split_once('=')
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .ok_or_else(|| {
                    Failure::Message(format!("--set expects key=value, got {assignment:?}"))
                })
        })
        .collect()
}

/// An open volume.
pub type Volume = Fs<FileDevice>;

fn open(image: &Path, writable: bool, set: &[(String, String)]) -> Outcome<Volume> {
    let device =
        FileDevice::open(image, writable).context(|| format!("opening {}", image.display()))?;
    Fs::open(device, writable, set).context(|| format!("mounting {}", image.display()))
}

fn close(fs: Volume) -> Outcome<()> {
    fs.close().map(drop).context(|| "committing".into())
}

fn read_input(path: &Path) -> Outcome<Vec<u8>> {
    if path == Path::new("-") {
        let mut bytes = Vec::new();
        io::stdin()
            .read_to_end(&mut bytes)
            .context(|| "reading standard input".into())?;
        Ok(bytes)
    } else {
        std::fs::read(path).context(|| format!("reading {}", path.display()))
    }
}

fn write_output(path: &Path, bytes: &[u8]) -> Outcome<()> {
    if path == Path::new("-") {
        let mut out = io::stdout().lock();
        out.write_all(bytes)
            .and_then(|()| out.flush())
            .context(|| "writing standard output".into())
    } else {
        std::fs::write(path, bytes).context(|| format!("writing {}", path.display()))
    }
}

fn format_arg(name: Option<&str>, path: &Path, bytes: Option<&[u8]>) -> Outcome<Format> {
    if let Some(name) = name {
        return Format::parse(name).context(|| "--format".into());
    }
    Ok(bytes.map_or_else(
        || Format::of_path(path).unwrap_or(Format::Archive),
        Format::detect,
    ))
}

const fn kind_char(kind: FileKind) -> char {
    match kind {
        FileKind::File => '-',
        FileKind::Dir => 'd',
        FileKind::Symlink => 'l',
    }
}

fn perm_text(perm: u16) -> String {
    (0..9)
        .map(|bit| {
            if perm & (0o400 >> bit) == 0 {
                '-'
            } else {
                ['r', 'w', 'x'][bit % 3]
            }
        })
        .collect()
}

fn print_lines(lines: impl IntoIterator<Item = String>) -> Outcome<()> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in lines {
        writeln!(out, "{line}").context(|| "writing standard output".into())?;
    }
    out.flush().context(|| "writing standard output".into())
}

fn list(fs: &Volume, path: &str, long: bool, recursive: bool) -> Outcome<Vec<String>> {
    let ino = fs.resolve(path).context(|| path.to_string())?;
    let attr = fs.getattr(ino).context(|| path.to_string())?;
    let describe = |name: &str, ino: u64| -> Outcome<String> {
        if !long {
            return Ok(name.to_string());
        }
        let attr = fs.getattr(ino).context(|| name.to_string())?;
        let mut line = format!(
            "{}{} {:>3} {:>12} {:>8} {name}",
            kind_char(attr.kind),
            perm_text(attr.perm),
            attr.nlink,
            attr.size,
            attr.allocated / fs.block_size() as u64,
        );
        if attr.kind == FileKind::Symlink {
            line.push_str(" -> ");
            line.push_str(&fs.readlink(ino).context(|| name.to_string())?);
        }
        Ok(line)
    };
    if attr.kind != FileKind::Dir {
        return Ok(vec![describe(path, ino)?]);
    }
    let mut lines = Vec::new();
    let mut pending = vec![(path.trim_end_matches('/').to_string(), ino)];
    while let Some((dir, dir_ino)) = pending.pop() {
        let mut entries = fs.readdir(dir_ino).context(|| dir.clone())?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        if recursive {
            lines.push(format!("{}:", if dir.is_empty() { "/" } else { &dir }));
        }
        let mut subdirs = Vec::new();
        for entry in entries {
            lines.push(describe(&entry.name, entry.ino)?);
            if recursive
                && entry.kind == FileKind::Dir
                && entry.ino != links_fs::fs::CONTROL_DIR_INO
            {
                subdirs.push((format!("{dir}/{}", entry.name), entry.ino));
            }
        }
        pending.extend(subdirs.into_iter().rev());
    }
    Ok(lines)
}

/// Creates `path`; with `parents`, missing parents too and existing
/// directories are fine.
pub fn mkdir(fs: &mut Volume, path: &str, parents: bool) -> Outcome<u64> {
    if !parents {
        let (parent, name) = fs.resolve_parent(path).context(|| path.to_string())?;
        return Ok(fs
            .mkdir(parent, name, 0o755, Creator::default())
            .context(|| path.to_string())?
            .ino);
    }
    let mut ino = ROOT_INO;
    for part in path.split('/').filter(|part| !part.is_empty()) {
        ino = match fs.lookup(ino, part) {
            Ok(attr) if attr.kind == FileKind::Dir => attr.ino,
            Ok(_) => return Err(Failure::Fs(path.to_string(), Error::NotDir)),
            Err(Error::NotFound) => {
                fs.mkdir(ino, part, 0o755, Creator::default())
                    .context(|| path.to_string())?
                    .ino
            }
            Err(error) => return Err(Failure::Fs(path.to_string(), error)),
        };
    }
    Ok(ino)
}

fn remove(fs: &mut Volume, path: &str, recursive: bool) -> Outcome<()> {
    let (parent, name) = fs.resolve_parent(path).context(|| path.to_string())?;
    let attr = fs.lookup(parent, name).context(|| path.to_string())?;
    if attr.kind != FileKind::Dir {
        return fs.unlink(parent, name).context(|| path.to_string());
    }
    if !recursive {
        return Err(Failure::Fs(path.to_string(), Error::IsDir));
    }
    for entry in fs.readdir(attr.ino).context(|| path.to_string())? {
        remove(
            fs,
            &format!("{}/{}", path.trim_end_matches('/'), entry.name),
            true,
        )?;
    }
    fs.rmdir(parent, name).context(|| path.to_string())
}

fn mkfs(
    image: &Path,
    size: Option<u64>,
    mut options: FormatOptions,
    root_owner: Option<(u32, u32)>,
    label: &str,
    force: bool,
) -> Outcome<()> {
    if !force && image.exists() {
        if let Ok(device) = FileDevice::open(image, false) {
            if Fs::open(device, false, &[]).is_ok() {
                return Err(Failure::Message(format!(
                    "{} already holds an LFS volume (use --force to overwrite it)",
                    image.display()
                )));
            }
        }
    }
    let device = size
        .map_or_else(
            || FileDevice::open(image, true),
            |size| FileDevice::create(image, size),
        )
        .context(|| format!("opening {}", image.display()))?;
    options.root_owner = root_owner.unwrap_or_else(|| image_owner(image));
    let fs = Fs::format(device, &options, label).context(|| "formatting".into())?;
    let stats = fs.statfs();
    println!(
        "created {}: {} blocks of {} bytes, {} free, layout {}, names {}",
        image.display(),
        stats.blocks,
        stats.block_size,
        stats.free,
        options.layout.name(),
        options.name_encoding.name()
    );
    close(fs)
}

/// The owner of a regular image file (its creator), or root for devices.
#[cfg(unix)]
fn image_owner(image: &Path) -> (u32, u32) {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(image)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map_or((0, 0), |metadata| (metadata.uid(), metadata.gid()))
}

/// Root: the volume has no host owner to inherit here.
#[cfg(not(unix))]
const fn image_owner(_image: &Path) -> (u32, u32) {
    (0, 0)
}

/// Parses `UID:GID`.
pub fn parse_owner(text: &str) -> std::result::Result<(u32, u32), String> {
    let parse = |part: &str| {
        part.parse::<u32>()
            .map_err(|_| format!("expected UID:GID with numeric ids, got {text:?}"))
    };
    let (uid, gid) = text
        .split_once(':')
        .ok_or_else(|| format!("expected UID:GID, got {text:?}"))?;
    Ok((parse(uid)?, parse(gid)?))
}

fn links_command(
    fs: &mut Volume,
    action: Option<&LinkAction>,
    source: Option<u64>,
    target: Option<u64>,
) -> Outcome<()> {
    let show = |fs: &Volume, index: u64| -> Outcome<String> {
        let link = fs.links().get(index).context(|| format!("link {index}"))?;
        Ok(format!(
            "({}: {} {})",
            link.index,
            format_ref(link.source),
            format_ref(link.target)
        ))
    };
    let line = match action {
        None => {
            let found = fs.query_links(source, target);
            return print_lines(packet::write_lino(&found).lines().map(str::to_string));
        }
        Some(&LinkAction::Create { source, target }) => {
            let index = fs
                .create_link(source, target)
                .context(|| format!("creating ({} {})", format_ref(source), format_ref(target)))?;
            show(fs, index)?
        }
        Some(LinkAction::Point) => {
            let index = fs.create_point().context(|| "creating a point".into())?;
            show(fs, index)?
        }
        Some(&LinkAction::Update {
            index,
            source,
            target,
        }) => {
            fs.update_link(index, source, target)
                .context(|| format!("updating link {index}"))?;
            show(fs, index)?
        }
        Some(&LinkAction::Delete { index }) => {
            let line = show(fs, index)?;
            fs.delete_link(index)
                .context(|| format!("deleting link {index}"))?;
            line
        }
    };
    print_lines([line])
}

/// Runs a parsed command line.
pub fn run(cli: Cli) -> Outcome<ExitCode> {
    let set = overrides(&cli.set)?;
    match cli.command {
        Command::Mkfs {
            image,
            size,
            block_size,
            layout,
            names,
            journal_blocks,
            label,
            root_owner,
            force,
        } => {
            let options = FormatOptions {
                block_size,
                layout: Layout::parse(&layout).context(|| "--layout".into())?,
                name_encoding: NameEncoding::parse(&names).context(|| "--names".into())?,
                journal_blocks,
                root_owner: (0, 0),
            };
            mkfs(&image, size, options, root_owner, &label, force)?;
            if !set.is_empty() {
                let mut fs = open(&image, true, &[])?;
                for (key, value) in &set {
                    fs.set_config_value(key, value).context(|| key.clone())?;
                }
                close(fs)?;
            }
        }
        Command::Info { image } => {
            let fs = open(&image, false, &set)?;
            let mut text = fs.stats_text();
            text.push_str(&fs.config().to_lino());
            print_lines(text.lines().map(str::to_string))?;
        }
        Command::Map { image, output } => {
            let fs = open(&image, false, &set)?;
            write_output(
                output.as_deref().unwrap_or_else(|| Path::new("-")),
                fs.map_json().as_bytes(),
            )?;
        }
        Command::Blueprint {
            image,
            depth,
            at,
            json,
            hex,
        } => {
            let mut fs = open(&image, false, &set)?;
            let path = blueprint_path(&mut fs, at.as_deref())?;
            let mut node = fs
                .blueprint_at(&path, depth)
                .context(|| "dissecting".into())?;
            if json {
                if hex > 0 {
                    fs.blueprint_hex(&mut node, hex)
                        .context(|| "reading".into())?;
                }
                let mut text = node.to_json();
                text.push('\n');
                write_output(Path::new("-"), text.as_bytes())?;
            } else {
                if !path.is_empty() {
                    let path: Vec<String> = path.iter().map(ToString::to_string).collect();
                    print_lines([format!("at {}", path.join("/"))])?;
                }
                print_lines(node.to_text().lines().map(str::to_string))?;
            }
        }
        Command::Demo {
            image,
            params,
            force,
        } => {
            if !force && image.exists() {
                return Err(Failure::Message(format!(
                    "{} exists (use --force to overwrite it)",
                    image.display()
                )));
            }
            let mut demo = Demo::parse(&params.join("\n")).context(|| "parameters".into())?;
            for (key, value) in &set {
                demo.set(key, value).context(|| key.clone())?;
            }
            let fs = demo.build().context(|| "building the demo".into())?;
            let stats = fs.statfs();
            let bytes = fs.close().context(|| "committing".into())?.into_bytes();
            std::fs::write(&image, bytes).context(|| format!("writing {}", image.display()))?;
            print_lines([format!(
                "created {}: {} blocks of {} bytes, {} free, {} files, {} links",
                image.display(),
                stats.blocks,
                stats.block_size,
                stats.free,
                stats.files,
                stats.links
            )])?;
        }
        Command::Fsck { image } => {
            let fs = open(&image, false, &set)?;
            let problems = fs.check();
            if problems.is_empty() {
                print_lines([format!("{}: clean", image.display())])?;
            } else {
                print_lines(problems)?;
                return Ok(ExitCode::FAILURE);
            }
        }
        Command::Ls {
            image,
            path,
            long,
            recursive,
        } => {
            let fs = open(&image, false, &set)?;
            print_lines(list(&fs, &path, long, recursive)?)?;
        }
        Command::Mkdir {
            image,
            paths,
            parents,
        } => {
            let mut fs = open(&image, true, &set)?;
            for path in &paths {
                mkdir(&mut fs, path, parents)?;
            }
            close(fs)?;
        }
        Command::Put {
            image,
            source,
            dest,
        } => {
            let mut fs = open(&image, true, &set)?;
            let report = copy::put(&mut fs, &source, &dest)?;
            close(fs)?;
            eprintln!("copied {report}");
        }
        Command::Get {
            image,
            source,
            dest,
        } => {
            let mut fs = open(&image, false, &set)?;
            let report = copy::get(&mut fs, &source, &dest)?;
            eprintln!("copied {report}");
        }
        Command::Cat { image, paths } => {
            let mut fs = open(&image, false, &set)?;
            let mut out = io::stdout().lock();
            for path in &paths {
                copy::cat(&mut fs, path, &mut out)?;
            }
        }
        Command::Write { image, path } => {
            let data = read_input(Path::new("-"))?;
            let mut fs = open(&image, true, &set)?;
            copy::write_file(&mut fs, &path, &data, 0o644)?;
            close(fs)?;
        }
        Command::Rm {
            image,
            paths,
            recursive,
        } => {
            let mut fs = open(&image, true, &set)?;
            for path in &paths {
                remove(&mut fs, path, recursive)?;
            }
            close(fs)?;
        }
        Command::Mv { image, from, to } => {
            let mut fs = open(&image, true, &set)?;
            let (parent, name) = fs.resolve_parent(&from).context(|| from.clone())?;
            let (new_parent, new_name) = match fs.resolve(&to) {
                Ok(dir) if fs.getattr(dir).is_ok_and(|attr| attr.kind == FileKind::Dir) => {
                    (dir, name)
                }
                _ => fs.resolve_parent(&to).context(|| to.clone())?,
            };
            fs.rename(parent, name, new_parent, new_name, false)
                .context(|| format!("{from} -> {to}"))?;
            close(fs)?;
        }
        Command::Ln {
            image,
            target,
            path,
            symbolic,
        } => {
            let mut fs = open(&image, true, &set)?;
            let (parent, name) = fs.resolve_parent(&path).context(|| path.clone())?;
            if symbolic {
                fs.symlink(parent, name, &target, Creator::default())
                    .context(|| path.clone())?;
            } else {
                let ino = fs.resolve(&target).context(|| target.clone())?;
                fs.link(ino, parent, name).context(|| path.clone())?;
            }
            close(fs)?;
        }
        Command::Config { image, assignments } => {
            if assignments.is_empty() {
                let fs = open(&image, false, &set)?;
                print_lines(fs.config().to_lino().lines().map(str::to_string))?;
            } else {
                let mut fs = open(&image, true, &set)?;
                for assignment in &assignments {
                    let (key, value) = assignment
                        .split_once('=')
                        .or_else(|| assignment.split_once(':'))
                        .ok_or_else(|| {
                            Failure::Message(format!("expected key=value, got {assignment:?}"))
                        })?;
                    fs.set_config_value(key.trim(), value.trim())
                        .context(|| assignment.clone())?;
                }
                print_lines(fs.config().to_lino().lines().map(str::to_string))?;
                close(fs)?;
            }
        }
        Command::Links {
            image,
            action,
            source,
            target,
        } => {
            let writable = action.is_some();
            let mut fs = open(&image, writable, &set)?;
            links_command(&mut fs, action.as_ref(), source, target)?;
            if writable {
                close(fs)?;
            }
        }
        Command::ExportLinks {
            image,
            output,
            format,
        } => {
            let fs = open(&image, false, &set)?;
            let format = format_arg(format.as_deref(), &output, None)?;
            let bytes = fs.export_links(format).context(|| "exporting".into())?;
            write_output(&output, &bytes)?;
            eprintln!("exported {} links as {}", fs.links().count(), format.name());
        }
        Command::ImportLinks {
            image,
            input,
            format,
        } => {
            let bytes = read_input(&input)?;
            let format = format_arg(format.as_deref(), &input, Some(&bytes))?;
            let mut fs = open(&image, true, &set)?;
            let added = fs
                .import_links(&bytes, format)
                .context(|| format!("importing {}", input.display()))?;
            close(fs)?;
            eprintln!("imported {added} new links ({})", format.name());
        }
        Command::Convert {
            input,
            output,
            from,
            to,
        } => convert(&input, &output, from.as_deref(), to.as_deref())?,
        Command::Compact { image } => {
            let mut fs = open(&image, true, &set)?;
            let report = fs.compact().context(|| "compacting".into())?;
            close(fs)?;
            println!(
                "moved {} files and {} chunks in {} passes; largest free extent {} -> {} blocks",
                report.files,
                report.chunks,
                report.passes,
                report.largest_before,
                report.largest_after
            );
        }
        Command::Mount {
            image,
            mountpoint,
            read_only,
            allow_other,
            nfs,
        } => {
            let how = mount::How {
                read_only,
                allow_other,
                nfs,
            };
            mount::mount(&image, &mountpoint, &how, &set)?;
        }
        Command::Serve {
            image,
            listen,
            read_only,
        } => mount::serve(&image, listen, read_only, &set)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn convert(input: &Path, output: &Path, from: Option<&str>, to: Option<&str>) -> Outcome<()> {
    let bytes = read_input(input)?;
    let from = format_arg(from, input, Some(&bytes))?;
    let to = format_arg(to, output, None)?;
    let (links, names) = if from == Format::Archive {
        let archive = packet::read_archive(&bytes).context(|| input.display().to_string())?;
        (archive.links, archive.names)
    } else {
        (
            packet::read(from, &bytes).context(|| input.display().to_string())?,
            Vec::new(),
        )
    };
    let out = packet::write(to, &links, &names).context(|| "converting".into())?;
    write_output(output, &out)?;
    eprintln!(
        "converted {} links: {} -> {}",
        links.len(),
        from.name(),
        to.name()
    );
    Ok(())
}
