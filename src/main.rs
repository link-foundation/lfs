//! `lfs`: create, inspect, change and mount LFS volumes.

mod cli;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

/// LFS — a links file system with an embedded links database.
///
/// IMAGE is an image file or a raw device (`/dev/sdb1`, `\\.\E:`).
#[derive(Parser, Debug)]
#[command(name = "lfs", version, about, long_about = None)]
pub struct Cli {
    /// Runtime configuration for this run (`key=value`, repeatable); see
    /// `lfs config --help` for the keys.
    #[arg(long = "set", global = true, value_name = "KEY=VALUE")]
    pub set: Vec<String>,
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// Every `lfs` command.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Creates a volume.
    Mkfs {
        /// Image file or device.
        image: PathBuf,
        /// Create (or truncate) an image file of this size (`64M`, `2G`).
        #[arg(long, value_parser = cli::parse_size)]
        size: Option<u64>,
        /// Block size in bytes.
        #[arg(long, default_value_t = 4096)]
        block_size: u32,
        /// Link layout: united, split or data-only.
        #[arg(long, default_value = "united")]
        layout: String,
        /// Name encoding: chars (binary links notation strings) or packed.
        #[arg(long, default_value = "chars")]
        names: String,
        /// Journal size in blocks (0: no journal; default: picked from the size).
        #[arg(long)]
        journal_blocks: Option<u64>,
        /// Volume label.
        #[arg(long, default_value = "lfs")]
        label: String,
        /// Owner of the root directory (`UID:GID`; default: the owner of the
        /// image file, so whoever creates an image can write to its mount).
        #[arg(long, value_parser = cli::parse_owner)]
        root_owner: Option<(u32, u32)>,
        /// Overwrite an existing LFS volume.
        #[arg(long)]
        force: bool,
    },
    /// Shows volume statistics and the configuration.
    Info {
        /// Image file or device.
        image: PathBuf,
    },
    /// Writes the disk space map as JSON (for the web visualizer).
    Map {
        /// Image file or device.
        image: PathBuf,
        /// Output file (default: standard output).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Dissects the image byte by byte: regions, records, fields.
    ///
    /// Every node covers exactly the bytes of its children, down to one-byte
    /// fields, so the leaves partition the whole image.
    Blueprint {
        /// Image file or device.
        image: PathBuf,
        /// Levels to expand below the starting node (`all` for everything).
        #[arg(long, default_value = "2", value_parser = cli::parse_depth)]
        depth: usize,
        /// Start at this node: a path of child indices (`3/0/2`) or `@OFFSET`
        /// for the deepest node holding that byte (`@4096`, `@0x1000`).
        #[arg(long)]
        at: Option<String>,
        /// Write JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Include the bytes of nodes up to this size as hex (JSON only).
        #[arg(long, default_value_t = 0)]
        hex: u64,
    },
    /// Creates an image with a reproducible demo workload, for exploring
    /// any configuration (`lfs demo demo.img layout=split index=adaptive`).
    #[command(after_help = cli::demo_help())]
    Demo {
        /// Image file to create.
        image: PathBuf,
        /// `key=value` parameters: format options, workload and runtime keys.
        params: Vec<String>,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
    /// Checks every invariant of a volume (exit code 1 if anything is wrong).
    Fsck {
        /// Image file or device.
        image: PathBuf,
    },
    /// Lists a directory.
    Ls {
        /// Image file or device.
        image: PathBuf,
        /// Directory or file.
        #[arg(default_value = "/")]
        path: String,
        /// Long format: kind, permissions, links, size, blocks, name.
        #[arg(short, long)]
        long: bool,
        /// Recurse into subdirectories.
        #[arg(short = 'R', long)]
        recursive: bool,
    },
    /// Creates directories.
    Mkdir {
        /// Image file or device.
        image: PathBuf,
        /// Directories to create.
        #[arg(required = true)]
        paths: Vec<String>,
        /// Create missing parents and accept existing directories.
        #[arg(short, long)]
        parents: bool,
    },
    /// Copies host files or directory trees into the volume.
    Put {
        /// Image file or device.
        image: PathBuf,
        /// Host file or directory.
        source: PathBuf,
        /// Destination path in the volume (an existing directory receives
        /// the source under its own name).
        #[arg(default_value = "/")]
        dest: String,
    },
    /// Copies files or directory trees out of the volume.
    Get {
        /// Image file or device.
        image: PathBuf,
        /// Path in the volume.
        source: String,
        /// Host destination (an existing directory receives the source
        /// under its own name).
        #[arg(default_value = ".")]
        dest: PathBuf,
    },
    /// Writes files to standard output.
    Cat {
        /// Image file or device.
        image: PathBuf,
        /// Files.
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Writes standard input into a file (created if missing).
    Write {
        /// Image file or device.
        image: PathBuf,
        /// File.
        path: String,
    },
    /// Removes files, symbolic links and (with -r) directory trees.
    Rm {
        /// Image file or device.
        image: PathBuf,
        /// Paths.
        #[arg(required = true)]
        paths: Vec<String>,
        /// Remove directories and their contents.
        #[arg(short, long)]
        recursive: bool,
    },
    /// Renames or moves a file or directory.
    Mv {
        /// Image file or device.
        image: PathBuf,
        /// Existing path.
        from: String,
        /// New path (an existing directory receives the entry).
        to: String,
    },
    /// Creates a hard link (or with -s a symbolic link).
    Ln {
        /// Image file or device.
        image: PathBuf,
        /// Existing file (or the symbolic link text).
        target: String,
        /// New name.
        path: String,
        /// Create a symbolic link.
        #[arg(short, long)]
        symbolic: bool,
    },
    /// Shows the configuration, or changes keys (`key=value`) persistently.
    #[command(after_help = cli::config_help())]
    Config {
        /// Image file or device.
        image: PathBuf,
        /// `key=value` assignments.
        assignments: Vec<String>,
    },
    /// Lists, queries and changes links of the embedded database.
    Links {
        /// Image file or device.
        image: PathBuf,
        /// What to do (default: list matching links).
        #[command(subcommand)]
        action: Option<LinkAction>,
        /// List links with this source (`12` or `$7` for external 7).
        #[arg(long, value_parser = cli::parse_link_ref)]
        source: Option<u64>,
        /// List links with this target.
        #[arg(long, value_parser = cli::parse_link_ref)]
        target: Option<u64>,
    },
    /// Exports the whole links store (binary links notation, doublets, `LiNo`).
    ExportLinks {
        /// Image file or device.
        image: PathBuf,
        /// Output file (`-` for standard output).
        output: PathBuf,
        /// Format: binary, doublets or lino (default: from the extension,
        /// else binary).
        #[arg(long)]
        format: Option<String>,
    },
    /// Imports links (any format) into the volume's database.
    ImportLinks {
        /// Image file or device.
        image: PathBuf,
        /// Input file (`-` for standard input).
        input: PathBuf,
        /// Format: binary, doublets or lino (default: detected).
        #[arg(long)]
        format: Option<String>,
    },
    /// Converts links files between formats (no volume needed).
    Convert {
        /// Input file.
        input: PathBuf,
        /// Output file.
        output: PathBuf,
        /// Input format (default: detected).
        #[arg(long)]
        from: Option<String>,
        /// Output format (default: from the extension, else binary).
        #[arg(long)]
        to: Option<String>,
    },
    /// Defragments free space by moving files and link chunks together.
    Compact {
        /// Image file or device.
        image: PathBuf,
    },
    /// Grows or shrinks a volume (shrinking compacts it first if needed).
    Resize {
        /// Image file or device.
        image: PathBuf,
        /// New size (`64M`, `2G`); an image file is extended or truncated to
        /// it. Default: the whole device, after the image file or the
        /// partition was enlarged.
        #[arg(long, value_parser = cli::parse_size)]
        size: Option<u64>,
    },
    /// Mounts a volume (FUSE on Linux and macOS, Dokan on Windows, or NFS
    /// with `--nfs`).
    Mount {
        /// Image file or device.
        image: PathBuf,
        /// Mount point (a directory, or a drive letter like `M:` on Windows).
        mountpoint: PathBuf,
        /// Mount read-only.
        #[arg(long)]
        read_only: bool,
        /// Let other users access the mount (FUSE `allow_other`).
        #[arg(long)]
        allow_other: bool,
        /// Serve over NFS version 3 on localhost and mount with the system's
        /// NFS client (`nfs` feature): `mount_nfs` on macOS, which needs no
        /// kernel extension, or `mount -t nfs` as root on Linux.
        #[arg(long)]
        nfs: bool,
    },
    /// Serves a volume over NFS version 3 until Ctrl-C (`nfs` feature) and
    /// prints the command that mounts it.
    Serve {
        /// Image file or device.
        image: PathBuf,
        /// Address to listen on; port 0 picks a free port. NFS version 3
        /// trusts the uid a client sends, so listen beyond localhost only on
        /// a network you trust.
        #[arg(long, default_value = "127.0.0.1:0")]
        listen: std::net::SocketAddr,
        /// Serve read-only.
        #[arg(long)]
        read_only: bool,
    },
}

/// Link database actions.
#[derive(Subcommand, Debug)]
pub enum LinkAction {
    /// Creates `source -> target` and prints it.
    Create {
        /// Source (`12` or `$7`).
        #[arg(value_parser = cli::parse_link_ref)]
        source: u64,
        /// Target (`12` or `$7`).
        #[arg(value_parser = cli::parse_link_ref)]
        target: u64,
    },
    /// Creates a point (a link whose ends are itself) and prints it.
    Point,
    /// Changes both ends of a link.
    Update {
        /// Link index.
        index: u64,
        /// New source.
        #[arg(value_parser = cli::parse_link_ref)]
        source: u64,
        /// New target.
        #[arg(value_parser = cli::parse_link_ref)]
        target: u64,
    },
    /// Deletes a link no other link refers to.
    Delete {
        /// Link index.
        index: u64,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli::run(cli) {
        Ok(code) => code,
        Err(error) => {
            if !cli::is_broken_pipe(&error) {
                eprintln!("lfs: {error}");
            }
            ExitCode::FAILURE
        }
    }
}
