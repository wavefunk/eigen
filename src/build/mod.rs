//! Build engine: output setup, context assembly, page rendering, fragment
//! extraction, and sitemap generation.

pub mod audit;
pub mod bundling;
pub mod content_hash;
pub mod context;
pub mod critical_css;
pub mod feed;
pub mod fragments;
pub mod hints;
pub mod json_ld;
pub mod minify;
pub mod not_found;
pub mod output;
pub mod render;
pub mod robots;
pub mod seo;
pub mod sitemap;
pub mod view_transitions;

pub use render::build;
