//! Copying between the host and a volume.

use super::{Context, Failure, Outcome, Volume};
use links_fs::error::Error;
use links_fs::fs::{Creator, FileKind, SetAttr, CONTROL_DIR_INO};
use std::fmt;
use std::fs as host;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Copy buffer size.
const CHUNK: usize = 1 << 20;

/// What a copy did.
#[derive(Debug, Default)]
pub struct Report {
    files: u64,
    dirs: u64,
    symlinks: u64,
    bytes: u64,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} files ({} bytes), {} directories, {} symbolic links",
            self.files, self.bytes, self.dirs, self.symlinks
        )
    }
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

fn host_name(path: &Path) -> Outcome<String> {
    let name = path
        .canonicalize()
        .ok()
        .and_then(|path| path.file_name().map(std::ffi::OsStr::to_os_string))
        .or_else(|| path.file_name().map(std::ffi::OsStr::to_os_string))
        .ok_or_else(|| Failure::Message(format!("{} has no name", path.display())))?;
    name.into_string()
        .map_err(|name| Failure::Message(format!("{} is not valid UTF-8", name.to_string_lossy())))
}

/// Creates or truncates the volume file `path` and fills it from `source`
/// (`len` bytes, reserved up front so the file stays contiguous).
fn fill(fs: &mut Volume, path: &str, perm: u16, len: u64, source: &mut dyn Read) -> Outcome<u64> {
    let (parent, name) = fs.resolve_parent(path).context(|| path.to_string())?;
    let ino = match fs.lookup(parent, name) {
        Ok(attr) if attr.kind == FileKind::File => attr.ino,
        Ok(attr) if attr.kind == FileKind::Dir => {
            return Err(Failure::Fs(path.to_string(), Error::IsDir))
        }
        Ok(_) => {
            fs.unlink(parent, name).context(|| path.to_string())?;
            fs.create(parent, name, perm, Creator::default())
                .context(|| path.to_string())?
                .ino
        }
        Err(Error::NotFound) => {
            fs.create(parent, name, perm, Creator::default())
                .context(|| path.to_string())?
                .ino
        }
        Err(error) => return Err(Failure::Fs(path.to_string(), error)),
    };
    fs.open_file(ino).context(|| path.to_string())?;
    let result = (|| {
        fs.setattr(
            ino,
            SetAttr {
                size: Some(0),
                ..SetAttr::default()
            },
        )
        .context(|| path.to_string())?;
        if len > 0 {
            fs.setattr(
                ino,
                SetAttr {
                    size: Some(len),
                    ..SetAttr::default()
                },
            )
            .context(|| format!("reserving {len} bytes for {path}"))?;
        }
        let mut buf = vec![0; CHUNK];
        let mut offset = 0u64;
        loop {
            let read = match source.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(Failure::Io(format!("reading for {path}"), error)),
            };
            fs.write(ino, offset, &buf[..read])
                .context(|| path.to_string())?;
            offset += read as u64;
        }
        if offset != len {
            fs.setattr(
                ino,
                SetAttr {
                    size: Some(offset),
                    ..SetAttr::default()
                },
            )
            .context(|| path.to_string())?;
        }
        Ok(offset)
    })();
    fs.release(ino).context(|| path.to_string())?;
    result
}

/// Replaces the content of the volume file `path` with `data`.
pub fn write_file(fs: &mut Volume, path: &str, data: &[u8], perm: u16) -> Outcome<()> {
    fill(fs, path, perm, data.len() as u64, &mut &data[..]).map(drop)
}

#[cfg(unix)]
fn host_perm(meta: &host::Metadata) -> u16 {
    use std::os::unix::fs::PermissionsExt;
    (meta.permissions().mode() & 0o7777) as u16
}

#[cfg(not(unix))]
fn host_perm(meta: &host::Metadata) -> u16 {
    if meta.is_dir() {
        0o755
    } else if meta.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

fn put_tree(fs: &mut Volume, source: &Path, dest: &str, report: &mut Report) -> Outcome<()> {
    let meta = host::symlink_metadata(source).context(|| source.display().to_string())?;
    if meta.file_type().is_symlink() {
        let target = host::read_link(source).context(|| source.display().to_string())?;
        let target = target.to_string_lossy().replace('\\', "/");
        let (parent, name) = fs.resolve_parent(dest).context(|| dest.to_string())?;
        fs.symlink(parent, name, &target, Creator::default())
            .context(|| dest.to_string())?;
        report.symlinks += 1;
    } else if meta.is_dir() {
        let (parent, name) = fs.resolve_parent(dest).context(|| dest.to_string())?;
        match fs.lookup(parent, name) {
            Ok(attr) if attr.kind == FileKind::Dir => {}
            Ok(_) => return Err(Failure::Fs(dest.to_string(), Error::Exists)),
            Err(Error::NotFound) => {
                fs.mkdir(parent, name, host_perm(&meta), Creator::default())
                    .context(|| dest.to_string())?;
                report.dirs += 1;
            }
            Err(error) => return Err(Failure::Fs(dest.to_string(), error)),
        }
        let mut entries: Vec<_> = host::read_dir(source)
            .context(|| source.display().to_string())?
            .collect::<std::io::Result<_>>()
            .context(|| source.display().to_string())?;
        entries.sort_by_key(host::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().into_string().map_err(|name| {
                Failure::Message(format!(
                    "{} in {} is not UTF-8",
                    name.to_string_lossy(),
                    source.display()
                ))
            })?;
            put_tree(fs, &entry.path(), &join(dest, &name), report)?;
        }
    } else {
        let mut file = host::File::open(source).context(|| source.display().to_string())?;
        report.bytes += fill(fs, dest, host_perm(&meta), meta.len(), &mut file)?;
        report.files += 1;
    }
    Ok(())
}

/// Copies the host file or tree `source` to `dest` (an existing directory
/// receives it under its own name).
pub fn put(fs: &mut Volume, source: &Path, dest: &str) -> Outcome<Report> {
    let dest = match fs.resolve(dest) {
        Ok(ino) if fs.getattr(ino).is_ok_and(|attr| attr.kind == FileKind::Dir) => {
            join(dest, &host_name(source)?)
        }
        _ => dest.to_string(),
    };
    let mut report = Report::default();
    put_tree(fs, source, &dest, &mut report)?;
    Ok(report)
}

/// Streams the volume file `path` into `out`.
pub fn cat(fs: &mut Volume, path: &str, out: &mut dyn Write) -> Outcome<u64> {
    let ino = fs.resolve(path).context(|| path.to_string())?;
    let mut offset = 0u64;
    loop {
        let data = fs.read(ino, offset, CHUNK).context(|| path.to_string())?;
        if data.is_empty() {
            break;
        }
        out.write_all(&data)
            .context(|| "writing standard output".into())?;
        offset += data.len() as u64;
    }
    out.flush().context(|| "writing standard output".into())?;
    Ok(offset)
}

#[cfg(unix)]
fn host_symlink(target: &str, path: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(windows)]
fn host_symlink(target: &str, path: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, path)
}

#[cfg(not(any(unix, windows)))]
fn host_symlink(_target: &str, _path: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

fn get_tree(
    fs: &mut Volume,
    ino: u64,
    source: &str,
    dest: &Path,
    report: &mut Report,
) -> Outcome<()> {
    let attr = fs.getattr(ino).context(|| source.to_string())?;
    match attr.kind {
        FileKind::Symlink => {
            let target = fs.readlink(ino).context(|| source.to_string())?;
            host_symlink(&target, dest).context(|| dest.display().to_string())?;
            report.symlinks += 1;
        }
        FileKind::Dir => {
            if !dest.is_dir() {
                host::create_dir(dest).context(|| dest.display().to_string())?;
                report.dirs += 1;
            }
            for entry in fs.readdir(ino).context(|| source.to_string())? {
                if entry.ino == CONTROL_DIR_INO {
                    continue;
                }
                get_tree(
                    fs,
                    entry.ino,
                    &join(source, &entry.name),
                    &dest.join(&entry.name),
                    report,
                )?;
            }
        }
        FileKind::File => {
            let mut file = std::io::BufWriter::new(
                host::File::create(dest).context(|| dest.display().to_string())?,
            );
            report.bytes += cat(fs, source, &mut file)?;
            report.files += 1;
        }
    }
    Ok(())
}

/// Copies the volume file or tree `source` to the host path `dest` (an
/// existing directory receives it under its own name).
pub fn get(fs: &mut Volume, source: &str, dest: &Path) -> Outcome<Report> {
    let ino = fs.resolve(source).context(|| source.to_string())?;
    let dest: PathBuf = if dest.is_dir() {
        let name = source
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("");
        if name.is_empty() {
            dest.to_path_buf()
        } else {
            dest.join(name)
        }
    } else {
        dest.to_path_buf()
    };
    let mut report = Report::default();
    get_tree(fs, ino, source, &dest, &mut report)?;
    Ok(report)
}
