//! Uses an LFS volume as a library: files, user links in the embedded
//! links database, runtime configuration, compaction and the space map.
//!
//! ```text
//! cargo run --example basic_usage
//! ```

use links_fs::config::FormatOptions;
use links_fs::device::MemDevice;
use links_fs::error::Result;
use links_fs::fs::{Creator, Fs, SetAttr, ROOT_INO};
use links_fs::links::format_ref;
use links_fs::links::packet::Format;

fn write_file(fs: &mut Fs<MemDevice>, name: &str, data: &[u8]) -> Result<u64> {
    let ino = fs.create(ROOT_INO, name, 0o644, Creator::default())?.ino;
    // Declaring the size first gives the file its one contiguous extent.
    fs.setattr(
        ino,
        SetAttr {
            size: Some(data.len() as u64),
            ..SetAttr::default()
        },
    )?;
    fs.write(ino, 0, data)?;
    Ok(ino)
}

fn main() -> Result<()> {
    // A 4 MiB volume in memory; `FileDevice` does the same for image files
    // and block devices.
    let mut fs = Fs::format(MemDevice::new(4 << 20), &FormatOptions::default(), "demo")?;

    // Runtime characteristics change without reformatting.
    fs.set_config_value("journal", "full")?;
    fs.set_config_value("index", "adaptive")?;

    let hello = write_file(&mut fs, "hello.txt", b"Hello, links!\n")?;
    let filler = write_file(&mut fs, "filler.bin", &vec![7; 300_000])?;
    write_file(&mut fs, "after.bin", &vec![9; 100_000])?;
    println!(
        "hello.txt: {:?}",
        String::from_utf8_lossy(&fs.read(hello, 0, 64)?)
    );
    println!("filler.bin is inode {filler}");

    // User links live in the same store and the same journal as the files.
    // A point is the link `p -> p`; this links two of them.
    let (a, b) = (fs.create_point()?, fs.create_point()?);
    let link = fs.create_link(a, b)?;
    println!("created points {a} and {b} and the link {link}");
    for found in fs.query_links(Some(a), None) {
        println!(
            "  {}: {} -> {}",
            found.index,
            format_ref(found.source),
            format_ref(found.target)
        );
    }
    fs.commit()?;

    // Deleting a file leaves a gap; compaction moves everything behind it
    // down, so the free space is one extent again.
    fs.unlink(ROOT_INO, "filler.bin")?;
    fs.commit()?;
    let before = fs.space().free_extents().count();
    let report = fs.compact()?;
    println!(
        "free extents: {before} -> {} (moved {} files and {} chunks)",
        fs.space().free_extents().count(),
        report.files,
        report.chunks
    );

    let stats = fs.statfs();
    println!(
        "{} of {} blocks free, largest free extent {}, {} links",
        stats.free, stats.blocks, stats.largest_free, stats.links
    );

    // The JSON the web visualizer draws (`lfs map` writes the same).
    let map = fs.map_json();
    println!("space map: {} bytes of JSON", map.len());

    // The whole links store in links notation.
    let text = fs.export_links(Format::Lino)?;
    println!("links notation export: {} bytes", text.len());
    fs.close()?;
    Ok(())
}
