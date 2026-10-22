//! LFS — a links file system.

// Compiles and runs the Rust examples of the README.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod chunked;
pub mod codec;
pub mod config;
pub mod demo;
pub mod device;
pub mod driver;
pub mod error;
pub mod fs;
pub mod journal;
pub mod links;
pub mod sbt;
pub mod space;
pub mod superblock;
pub mod varstore;
