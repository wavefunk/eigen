//! Build engine: output setup, context assembly, page rendering, fragment
//! extraction, and sitemap generation.

pub mod context;
pub mod fragments;
pub mod minify;
pub mod output;
pub mod render;
pub mod robots;
pub mod sitemap;

pub use render::build;
