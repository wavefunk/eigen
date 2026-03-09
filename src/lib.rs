//! Eigen — a static site generator with HTMX support.
//!
//! This library crate exposes Eigen's modules for use in integration tests
//! and as a library.

pub mod assets;
pub mod build;
pub mod config;
pub mod data;
pub mod dev;
pub mod discovery;
pub mod frontmatter;
pub mod init;
pub mod plugins;
pub mod template;

/// Re-export the live-reload injection function for integration testing.
pub mod dev_inject {
    pub use crate::dev::inject::inject_reload_script;
}
