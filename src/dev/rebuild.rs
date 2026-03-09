//! Step 7.6: Smart rebuild with cached data.
//!
//! Tracks state between rebuilds so we can take the most efficient path:
//! - **Full rebuild**: config changed, reload everything.
//! - **DataOnly**: `_data/` files changed — invalidate file cache, re-render all.
//! - **Templates**: only specific templates changed — rebuild those + any
//!   templates that depend on changed layouts/partials.
//! - **StaticOnly**: just re-copy `static/` → `dist/`.
//!
//! The live-reload script is injected into every full page during dev builds.

use eyre::{Result, WrapErr};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::assets;
use crate::assets::cache::AssetCache;
use crate::config::SiteConfig;
use crate::data::{self, DataFetcher};
use crate::discovery::{self, PageDef, PageType};
use crate::plugins::registry::{self, PluginRegistry};
use crate::template;
use crate::build::render::RenderedPage;

use super::inject;
use super::watcher::RebuildScope;

/// Mutable state preserved across dev rebuilds.
pub struct DevBuildState {
    /// Project root path.
    project_root: PathBuf,
    /// Current site config.
    config: SiteConfig,
    /// Previous frontmatter per template (raw YAML string for change detection).
    prev_frontmatter: HashMap<PathBuf, String>,
    /// Data fetcher with URL cache that persists across rebuilds.
    fetcher: DataFetcher,
    /// Plugin registry.
    plugin_registry: PluginRegistry,
    /// Asset cache for localized remote assets.
    asset_cache: AssetCache,
    /// HTTP client for asset downloads.
    asset_client: reqwest::blocking::Client,
}

impl DevBuildState {
    /// Create a new dev build state and perform the initial full build.
    pub fn new(project_root: &Path) -> Result<Self> {
        let config = crate::config::load_config(project_root)?;
        let fetcher = DataFetcher::new(&config.sources, project_root);
        let plugin_registry = registry::build_registry(&config.plugins, project_root)?;
        let asset_cache = AssetCache::open(project_root)
            .wrap_err("Failed to open asset cache")?;
        let asset_client = reqwest::blocking::Client::new();

        let mut state = Self {
            project_root: project_root.to_path_buf(),
            config,
            prev_frontmatter: HashMap::new(),
            fetcher,
            plugin_registry,
            asset_cache,
            asset_client,
        };

        state.full_build()?;
        Ok(state)
    }

    /// Handle a rebuild based on the detected scope.
    pub fn rebuild(&mut self, scope: RebuildScope) -> Result<()> {
        let start = Instant::now();

        match scope {
            RebuildScope::Full => {
                tracing::info!("Full rebuild (config changed)...");
                // Reload config and plugins.
                self.config = crate::config::load_config(&self.project_root)?;
                self.fetcher = DataFetcher::new(&self.config.sources, &self.project_root);
                self.plugin_registry = registry::build_registry(&self.config.plugins, &self.project_root)?;
                self.full_build()?;
            }
            RebuildScope::DataOnly => {
                tracing::info!("Rebuild (data changed)...");
                self.fetcher.clear_file_cache();
                self.full_build()?;
            }
            RebuildScope::Templates(changed) => {
                tracing::info!("Rebuild (templates changed: {:?})...", changed);
                // If any _-prefixed (layout/partial) file changed, do full rebuild
                // since we can't easily track which pages depend on which layouts.
                let has_layout_change = changed.iter().any(|p| {
                    p.components().any(|c| {
                        c.as_os_str()
                            .to_str()
                            .map(|s| s.starts_with('_'))
                            .unwrap_or(false)
                    })
                });

                if has_layout_change {
                    tracing::debug!("  Layout/partial changed — full rebuild.");
                    self.full_build()?;
                } else {
                    // Only page templates changed — still do a full rebuild
                    // for simplicity (templates may reference each other via
                    // includes etc.), but skip re-fetching cached URL data.
                    self.full_build()?;
                }
            }
            RebuildScope::StaticOnly => {
                tracing::info!("Re-copying static assets...");
                crate::build::output::copy_static_assets(&self.project_root)?;
                tracing::info!("Static assets copied.");
            }
        }

        let elapsed = start.elapsed();
        tracing::info!("Rebuild completed in {:.1?}", elapsed);
        Ok(())
    }

    /// Perform a full build with live-reload injection.
    fn full_build(&mut self) -> Result<()> {
        let config = &self.config;
        let project_root = &self.project_root;

        let global_data = data::load_global_data(project_root)?;
        let pages = discovery::discover_pages(project_root, config)?;

        // Set up output directory.
        crate::build::output::setup_output_dir(
            project_root,
            config.build.fragments,
            &config.build.fragment_dir,
        )?;
        crate::build::output::copy_static_assets(project_root)?;

        // Set up template engine (with plugin extensions).
        let env = template::setup_environment(project_root, config, &pages, Some(&self.plugin_registry))?;

        // Build timestamp.
        let build_time =
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let dist_dir = project_root.join("dist");
        let mut rendered_pages: Vec<RenderedPage> = Vec::new();

        // Store frontmatter for change detection.
        let mut new_frontmatter: HashMap<PathBuf, String> = HashMap::new();

        for page in &pages {
            // Store raw frontmatter body for change detection.
            new_frontmatter.insert(
                page.template_path.clone(),
                format!("{:?}", page.frontmatter),
            );

            match &page.page_type {
                PageType::Static => {
                    let result = render_static_page_dev(
                        page,
                        &env,
                        &mut self.fetcher,
                        &global_data,
                        config,
                        &dist_dir,
                        &build_time,
                        &mut self.asset_cache,
                        &self.asset_client,
                        &self.plugin_registry,
                    )?;
                    rendered_pages.push(result);
                }
                PageType::Dynamic { param_name: _ } => {
                    let results = render_dynamic_page_dev(
                        page,
                        &env,
                        &mut self.fetcher,
                        &global_data,
                        config,
                        &dist_dir,
                        &build_time,
                        &mut self.asset_cache,
                        &self.asset_client,
                        &self.plugin_registry,
                    )?;
                    rendered_pages.extend(results);
                }
            }
        }

        // Generate sitemap.
        crate::build::sitemap::generate_sitemap(
            &dist_dir,
            &rendered_pages,
            config,
            &build_time,
        )?;

        self.prev_frontmatter = new_frontmatter;

        // Run post-build plugin hooks.
        self.plugin_registry.post_build(&dist_dir, project_root)?;

        tracing::info!("Dev build: {} page(s).", rendered_pages.len());
        Ok(())
    }
}

/// Render a static page with live-reload script injection.
fn render_static_page_dev(
    page: &PageDef,
    env: &minijinja::Environment<'_>,
    fetcher: &mut DataFetcher,
    global_data: &HashMap<String, serde_json::Value>,
    config: &SiteConfig,
    dist_dir: &Path,
    build_time: &str,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    plugin_registry: &PluginRegistry,
) -> Result<RenderedPage> {
    use crate::build::context::{self, PageMeta};
    use crate::build::fragments;

    let tmpl_name = page.template_path.to_string_lossy().to_string();

    // Resolve data queries.
    let page_data = data::resolve_page_data(&page.frontmatter, fetcher, Some(plugin_registry))
        .wrap_err_with(|| format!("Failed to resolve data for {}", tmpl_name))?;

    // Compute output path.
    let output_path = page
        .output_dir
        .join(page.template_path.file_name().unwrap_or_default());
    let url_path = format!("/{}", output_path.to_string_lossy().replace('\\', "/"));

    let is_index = output_path
        .file_name()
        .and_then(|f| f.to_str())
        .map(|f| f == "index.html")
        .unwrap_or(false);

    let meta = PageMeta {
        current_url: url_path.clone(),
        current_path: output_path.to_string_lossy().to_string(),
        base_url: config.site.base_url.clone(),
        build_time: build_time.to_string(),
    };

    let ctx = context::build_page_context(config, global_data, &page_data, meta, None);

    let tmpl = env
        .get_template(&tmpl_name)
        .wrap_err_with(|| format!("Template '{}' not found", tmpl_name))?;

    let rendered = tmpl
        .render(&ctx)
        .wrap_err_with(|| format!("Failed to render '{}'", tmpl_name))?;

    // Strip markers, localize assets, run plugins, and inject reload script.
    let full_html = fragments::strip_fragment_markers(&rendered);
    let full_html = assets::localize_assets(
        &full_html,
        &config.assets,
        asset_cache,
        asset_client,
        dist_dir,
    ).wrap_err_with(|| format!("Failed to localize assets for '{}'", tmpl_name))?;
    let full_html = plugin_registry.post_render_html(
        full_html,
        &url_path,
        dist_dir,
    )?;
    let full_html = inject::inject_reload_script(&full_html);

    let full_path = dist_dir.join(&output_path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full_path, &full_html)?;

    // Write fragments (also localize assets).
    if config.build.fragments {
        let frags = fragments::extract_fragments(&rendered);
        let frags: Vec<_> = match &page.frontmatter.fragment_blocks {
            Some(blocks) => frags
                .into_iter()
                .filter(|f| blocks.contains(&f.block_name))
                .collect(),
            None => frags,
        };
        if !frags.is_empty() {
            let localized_frags = localize_fragments_dev(
                &frags,
                &config.assets,
                asset_cache,
                asset_client,
                dist_dir,
            )?;
            fragments::write_fragments(
                dist_dir,
                &output_path,
                &localized_frags,
                &config.build.content_block,
                &config.build.fragment_dir,
            )?;
        }
    }

    Ok(RenderedPage {
        url_path,
        is_index,
        is_dynamic: false,
    })
}

/// Render all pages for a dynamic template with live-reload script injection.
fn render_dynamic_page_dev(
    page: &PageDef,
    env: &minijinja::Environment<'_>,
    fetcher: &mut DataFetcher,
    global_data: &HashMap<String, serde_json::Value>,
    config: &SiteConfig,
    dist_dir: &Path,
    build_time: &str,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    plugin_registry: &PluginRegistry,
) -> Result<Vec<RenderedPage>> {
    use crate::build::context::{self, PageMeta};
    use crate::build::fragments;
    use eyre::bail;

    let tmpl_name = page.template_path.to_string_lossy().to_string();
    let item_as = &page.frontmatter.item_as;
    let slug_field = &page.frontmatter.slug_field;

    // Fetch collection.
    let items = data::resolve_dynamic_page_data(&page.frontmatter, fetcher, Some(plugin_registry))
        .wrap_err_with(|| format!("Failed to fetch collection for {}", tmpl_name))?;

    if items.is_empty() {
        return Ok(Vec::new());
    }

    let tmpl = env
        .get_template(&tmpl_name)
        .wrap_err_with(|| format!("Template '{}' not found", tmpl_name))?;

    let mut rendered_pages = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        // Extract slug.
        let slug = match item.get(slug_field) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(_) => {
                eprintln!("  Warning: item {} has non-string slug, skipping.", idx);
                continue;
            }
            None => {
                eprintln!(
                    "  Warning: item {} missing slug field '{}', skipping.",
                    idx, slug_field
                );
                continue;
            }
        };

        let slug = slug::slugify(&slug);
        if slug.is_empty() {
            continue;
        }

        // Resolve nested data.
        let item_data =
            data::resolve_dynamic_page_data_for_item(&page.frontmatter, item, fetcher, Some(plugin_registry))?;

        let output_path = page.output_dir.join(format!("{}.html", slug));
        let url_path = format!("/{}", output_path.to_string_lossy().replace('\\', "/"));

        if rendered_pages.iter().any(|rp: &RenderedPage| rp.url_path == url_path) {
            bail!("Duplicate output path '{}' in '{}'", url_path, tmpl_name);
        }

        let meta = PageMeta {
            current_url: url_path.clone(),
            current_path: output_path.to_string_lossy().to_string(),
            base_url: config.site.base_url.clone(),
            build_time: build_time.to_string(),
        };

        let ctx =
            context::build_page_context(config, global_data, &item_data, meta, Some((item_as, item)));

        let rendered = tmpl.render(&ctx).wrap_err_with(|| {
            format!("Failed to render '{}' for slug '{}'", tmpl_name, slug)
        })?;

        // Strip markers, localize assets, run plugins, and inject reload script.
        let full_html = fragments::strip_fragment_markers(&rendered);
        let full_html = assets::localize_assets(
            &full_html,
            &config.assets,
            asset_cache,
            asset_client,
            dist_dir,
        ).wrap_err_with(|| {
            format!("Failed to localize assets for '{}' slug '{}'", tmpl_name, slug)
        })?;
        let full_html = plugin_registry.post_render_html(
            full_html,
            &url_path,
            dist_dir,
        )?;
        let full_html = inject::inject_reload_script(&full_html);

        let full_path = dist_dir.join(&output_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full_path, &full_html)?;

        // Write fragments (also localize assets).
        if config.build.fragments {
            let frags = fragments::extract_fragments(&rendered);
            let frags: Vec<_> = match &page.frontmatter.fragment_blocks {
                Some(blocks) => frags
                    .into_iter()
                    .filter(|f| blocks.contains(&f.block_name))
                    .collect(),
                None => frags,
            };
            if !frags.is_empty() {
                let localized_frags = localize_fragments_dev(
                    &frags,
                    &config.assets,
                    asset_cache,
                    asset_client,
                    dist_dir,
                )?;
                fragments::write_fragments(
                    dist_dir,
                    &output_path,
                    &localized_frags,
                    &config.build.content_block,
                    &config.build.fragment_dir,
                )?;
            }
        }

        rendered_pages.push(RenderedPage {
            url_path,
            is_index: false,
            is_dynamic: true,
        });
    }

    Ok(rendered_pages)
}

/// Localize assets in fragment HTML for dev builds.
fn localize_fragments_dev(
    frags: &[crate::build::fragments::Fragment],
    assets_config: &crate::config::AssetsConfig,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    dist_dir: &Path,
) -> Result<Vec<crate::build::fragments::Fragment>> {
    let mut result = Vec::with_capacity(frags.len());
    for frag in frags {
        let localized_html = assets::localize_assets(
            &frag.html,
            assets_config,
            asset_cache,
            asset_client,
            dist_dir,
        )?;
        result.push(crate::build::fragments::Fragment {
            block_name: frag.block_name.clone(),
            html: localized_html,
        });
    }
    Ok(result)
}
