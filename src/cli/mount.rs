//! `lfs mount` and `lfs serve`: serve a volume through FUSE, Dokan or NFS
//! until it is unmounted or the user presses Ctrl-C.

use super::{Context, Failure, Outcome};
use links_fs::driver::{spawn, MountOptions};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// How `lfs mount` mounts.
pub struct How {
    pub read_only: bool,
    pub allow_other: bool,
    pub nfs: bool,
}

/// A channel that receives a message on each Ctrl-C.
fn interrupts() -> Outcome<mpsc::Receiver<()>> {
    let (stop, stopped) = mpsc::channel();
    ctrlc::set_handler(move || {
        // Ignored once the main thread has stopped listening.
        let _ = stop.send(());
    })
    .map_err(|error| Failure::Message(format!("watching for Ctrl-C: {error}")))?;
    Ok(stopped)
}

/// Waits while `running` holds; returns whether Ctrl-C ended the wait.
fn wait(stopped: &mpsc::Receiver<()>, running: impl Fn() -> bool) -> bool {
    while running() {
        if stopped.recv_timeout(Duration::from_millis(200)).is_ok() {
            return true;
        }
    }
    false
}

fn options(image: &Path, read_only: bool, allow_other: bool) -> MountOptions {
    MountOptions {
        read_only,
        allow_other,
        name: image
            .file_name()
            .map_or_else(|| "lfs".into(), |name| name.to_string_lossy().into_owned()),
    }
}

pub fn mount(image: &Path, mountpoint: &Path, how: &How, set: &[(String, String)]) -> Outcome<()> {
    if how.nfs {
        return nfs::mount(image, mountpoint, how.read_only, set);
    }
    let fs = super::open(image, !how.read_only, set)?;
    let options = options(image, how.read_only, how.allow_other);
    let stopped = interrupts()?;
    let mounted = spawn(fs, mountpoint, &options)
        .context(|| format!("mounting at {}", mountpoint.display()))?;
    eprintln!(
        "mounted {} at {} (Ctrl-C or unmount to finish)",
        image.display(),
        mountpoint.display()
    );
    let interrupted = wait(&stopped, || mounted.is_mounted());
    let result = if interrupted && mounted.is_mounted() {
        mounted.unmount()
    } else {
        mounted.wait()
    };
    result.context(|| format!("unmounting {}", mountpoint.display()))?;
    eprintln!("unmounted {}", mountpoint.display());
    Ok(())
}

pub fn serve(
    image: &Path,
    listen: SocketAddr,
    read_only: bool,
    set: &[(String, String)],
) -> Outcome<()> {
    nfs::serve(image, listen, read_only, set)
}

#[cfg(feature = "nfs")]
mod nfs {
    use super::{interrupts, options, wait, Context, Outcome};
    use links_fs::driver::nfs;
    use std::net::SocketAddr;
    use std::path::Path;

    pub fn mount(
        image: &Path,
        mountpoint: &Path,
        read_only: bool,
        set: &[(String, String)],
    ) -> Outcome<()> {
        let fs = crate::cli::open(image, !read_only, set)?;
        let stopped = interrupts()?;
        let mounted = nfs::spawn(fs, mountpoint, &options(image, read_only, false))
            .context(|| format!("mounting at {} over NFS", mountpoint.display()))?;
        eprintln!(
            "mounted {} at {} over NFS on port {} (Ctrl-C or unmount to finish)",
            image.display(),
            mountpoint.display(),
            mounted.served().port()
        );
        wait(&stopped, || mounted.is_mounted());
        mounted
            .unmount()
            .context(|| format!("unmounting {}", mountpoint.display()))?;
        eprintln!("unmounted {}", mountpoint.display());
        Ok(())
    }

    pub fn serve(
        image: &Path,
        listen: SocketAddr,
        read_only: bool,
        set: &[(String, String)],
    ) -> Outcome<()> {
        let fs = crate::cli::open(image, !read_only, set)?;
        let stopped = interrupts()?;
        let served = nfs::serve(fs, listen, &options(image, read_only, false))
            .context(|| format!("serving on {listen}"))?;
        println!("serving {} on {}", image.display(), served.address());
        let command = nfs::mount_command(served.port(), Path::new("MOUNTPOINT"));
        let words: Vec<String> = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|word| word.to_string_lossy().into_owned())
            .collect();
        println!("mount it with: {}", words.join(" "));
        eprintln!("Ctrl-C to stop");
        wait(&stopped, || served.is_serving());
        served.stop().context(|| "stopping the server".into())?;
        eprintln!("stopped");
        Ok(())
    }
}

#[cfg(not(feature = "nfs"))]
mod nfs {
    use super::{Failure, Outcome};
    use std::net::SocketAddr;
    use std::path::Path;

    fn missing() -> Failure {
        Failure::Message("this lfs was built without NFS; rebuild it with `--features nfs`".into())
    }

    pub fn mount(_: &Path, _: &Path, _: bool, _: &[(String, String)]) -> Outcome<()> {
        Err(missing())
    }

    pub fn serve(_: &Path, _: SocketAddr, _: bool, _: &[(String, String)]) -> Outcome<()> {
        Err(missing())
    }
}
