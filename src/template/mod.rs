//! Template engine: minijinja environment, filters, functions, preprocessing.
//!
//! - **Environment**: minijinja setup with template loader and configuration
//! - **Preprocessing**: fragment marker injection into block tags
//! - **Filters**: markdown, date, slugify, absolute, truncate, sort_by, group_by, json
//! - **Functions**: link_to, current_year, asset

mod environment;
pub mod errors;
pub mod filters;
pub mod functions;
pub mod preprocessing;

pub use environment::setup_environment;
