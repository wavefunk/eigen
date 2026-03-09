//! Plugin system for Eigen.
//!
//! Plugins hook into the build pipeline at well-defined points to extend
//! Eigen's functionality without modifying core code.  Each plugin
//! implements the [`Plugin`] trait and is registered in the
//! [`PluginRegistry`].
//!
//! # Hook points
//!
//! The build pipeline calls plugins at these stages (in order):
//!
//! 1. **`on_config_loaded`** — After `site.toml` is parsed. Plugins can
//!    read their own config from `[plugins.<name>]`.
//!
//! 2. **`transform_data`** — After a data query is fetched and the `root`
//!    is extracted, but before filter/sort/limit transforms.  Plugins can
//!    reshape API responses (e.g., flatten Strapi's `attributes` nesting).
//!
//! 3. **`register_template_extensions`** — During template environment
//!    setup. Plugins can add custom filters, functions, and global variables.
//!
//! 4. **`post_render_html`** — After a page is rendered to HTML but before
//!    it is written to disk.  Plugins can process CSS (Tailwind), bundle JS,
//!    minify HTML, etc.
//!
//! 5. **`post_build`** — After all pages are written.  Plugins can run
//!    final processing steps (e.g., generating a search index).
//!
//! 6. **`dev_server_routes`** — During dev server setup.  Plugins can add
//!    Axum routes (e.g., a Tailwind CSS watch endpoint).
//!
//! # Configuration
//!
//! Plugins are declared in `site.toml`:
//!
//! ```toml
//! [plugins.strapi]
//! # plugin-specific settings
//! flatten = "attributes"
//! media_base_url = "http://localhost:1337"
//!
//! ```

pub mod registry;
pub mod strapi;

use eyre::Result;
use std::path::Path;

/// The trait all Eigen plugins must implement.
///
/// All methods have default no-op implementations so plugins only need to
/// override the hooks they care about.
pub trait Plugin: std::fmt::Debug + Send + Sync {
    /// The unique name of this plugin (e.g., `"strapi"`).
    fn name(&self) -> &str;

    /// Called after `site.toml` is loaded.  `plugin_config` is the raw TOML
    /// table from `[plugins.<name>]`, or `None` if the section is absent.
    fn on_config_loaded(
        &mut self,
        _plugin_config: Option<&toml::Value>,
        _project_root: &Path,
    ) -> Result<()> {
        Ok(())
    }

    /// Transform data after fetch + root extraction, before filter/sort/limit.
    ///
    /// `source_name` is `Some("strapi")` for remote sources, or `None` for
    /// local files.  Plugins should check the source name and return the
    /// value unmodified if they don't handle that source.
    ///
    /// This is called for EVERY data fetch, so plugins must be fast for
    /// sources they don't care about.
    fn transform_data(
        &self,
        value: serde_json::Value,
        _source_name: Option<&str>,
        _query_path: Option<&str>,
    ) -> Result<serde_json::Value> {
        Ok(value)
    }

    /// Register custom filters, functions, or globals on the template engine.
    fn register_template_extensions(
        &self,
        _env: &mut minijinja::Environment<'_>,
    ) -> Result<()> {
        Ok(())
    }

    /// Process rendered HTML before it is written to disk.
    ///
    /// `output_path` is relative to `dist/`, e.g. `"posts/hello.html"`.
    fn post_render_html(
        &self,
        html: String,
        _output_path: &str,
        _dist_dir: &Path,
    ) -> Result<String> {
        Ok(html)
    }

    /// Called once after all pages have been rendered and written.
    fn post_build(
        &self,
        _dist_dir: &Path,
        _project_root: &Path,
    ) -> Result<()> {
        Ok(())
    }
}
