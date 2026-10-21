//! `lfs mount`: serves a volume through FUSE or Dokan until it is unmounted
//! or the user presses Ctrl-C.

use super::{Context, Failure, Outcome};
use links_fs::driver::{spawn, MountOptions};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

pub fn mount(
    image: &Path,
    mountpoint: &Path,
    read_only: bool,
    allow_other: bool,
    set: &[(String, String)],
) -> Outcome<()> {
    let fs = super::open(image, !read_only, set)?;
    let options = MountOptions {
        read_only,
        allow_other,
        name: image
            .file_name()
            .map_or_else(|| "lfs".into(), |name| name.to_string_lossy().into_owned()),
    };
    let (stop, stopped) = mpsc::channel();
    ctrlc::set_handler(move || {
        // Ignored once the main thread has stopped listening.
        let _ = stop.send(());
    })
    .map_err(|error| Failure::Message(format!("watching for Ctrl-C: {error}")))?;
    let mounted = spawn(fs, mountpoint, &options)
        .context(|| format!("mounting at {}", mountpoint.display()))?;
    eprintln!(
        "mounted {} at {} (Ctrl-C or unmount to finish)",
        image.display(),
        mountpoint.display()
    );
    let mut interrupted = false;
    while mounted.is_mounted() {
        if stopped.recv_timeout(Duration::from_millis(200)).is_ok() {
            interrupted = true;
            break;
        }
    }
    let result = if interrupted && mounted.is_mounted() {
        mounted.unmount()
    } else {
        mounted.wait()
    };
    result.context(|| format!("unmounting {}", mountpoint.display()))?;
    eprintln!("unmounted {}", mountpoint.display());
    Ok(())
}
