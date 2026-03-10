//! Steps 5.3, 5.4, 5.6: Page rendering and build orchestrator.
//!
//! Renders static and dynamic pages, writes full HTML and fragment files,
//! and orchestrates the entire build process.

use eyre::{Result, WrapErr, bail};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::assets;
use crate::assets::cache::AssetCache;
use crate::config::SiteConfig;
use crate::data::{self, DataFetcher};
use crate::discovery::{self, PageDef, PageType};
use crate::plugins::registry::{self, PluginRegistry};
use crate::template;
use crate::template::errors::TemplateError;

use super::context::{self, PageMeta};
use super::fragments;
use super::output;
use super::sitemap;

/// A record of a rendered page, used for sitemap generation.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    /// URL path relative to site root, e.g. `/about.html`.
    pub url_path: String,
    /// Whether this is an index page (gets higher sitemap priority).
    pub is_index: bool,
    /// Whether this page was generated from a dynamic template.
    pub is_dynamic: bool,
}

/// Run the full build process.
///
/// This is the main entry point for `eigen build`.
pub fn build(project_root: &Path) -> Result<()> {
    let config = crate::config::load_config(project_root)?;
    tracing::info!("Loading config... ✓ ({})", config.site.name);

    // Initialize plugin registry.
    let plugin_registry = registry::build_registry(&config.plugins, project_root)?;
    if !plugin_registry.is_empty() {
        tracing::info!(
            "Plugins loaded: {}",
            plugin_registry.plugin_names().join(", ")
        );
    }

    let global_data = data::load_global_data(project_root)?;
    tracing::info!("Loading global data ({} files)... ✓", global_data.len());

    let pages = discovery::discover_pages(project_root, &config)?;
    let (static_count, dynamic_count) = count_page_types(&pages);
    tracing::info!(
        "Discovered {} static page(s), {} dynamic template(s)",
        static_count,
        dynamic_count,
    );

    // Set up output directory.
    output::setup_output_dir(
        project_root,
        config.build.fragments,
        &config.build.fragment_dir,
    )?;
    output::copy_static_assets(project_root)?;
    tracing::info!("Copying static assets... ✓");

    // Set up template engine (with plugin extensions).
    let env = template::setup_environment(project_root, &config, &pages, Some(&plugin_registry))?;
    tracing::debug!("Template engine configured.");

    // Data fetcher.
    let mut fetcher = DataFetcher::new(&config.sources, project_root);

    // Asset localization.
    let mut asset_cache = AssetCache::open(project_root)
        .wrap_err("Failed to open asset cache")?;
    let asset_client = reqwest::blocking::Client::new();
    if config.assets.localize {
        tracing::info!("Asset localization enabled.");
    }

    // Build timestamp.
    let build_time = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let dist_dir = project_root.join("dist");
    let mut rendered_pages: Vec<RenderedPage> = Vec::new();
    let mut output_paths: HashSet<String> = HashSet::new();
    let mut data_query_count = 0u32;

    for page in &pages {
        match &page.page_type {
            PageType::Static => {
                let result = render_static_page(
                    page,
                    &env,
                    &mut fetcher,
                    &global_data,
                    &config,
                    &dist_dir,
                    &build_time,
                    &mut output_paths,
                    &mut data_query_count,
                    &mut asset_cache,
                    &asset_client,
                    &plugin_registry,
                )?;
                rendered_pages.push(result);
            }
            PageType::Dynamic { param_name: _ } => {
                let results = render_dynamic_page(
                    page,
                    &env,
                    &mut fetcher,
                    &global_data,
                    &config,
                    &dist_dir,
                    &build_time,
                    &mut output_paths,
                    &mut data_query_count,
                    &mut asset_cache,
                    &asset_client,
                    &plugin_registry,
                )?;
                rendered_pages.extend(results);
            }
        }
    }

    // Generate sitemap.
    sitemap::generate_sitemap(&dist_dir, &rendered_pages, &config, &build_time)?;
    tracing::info!("Generating sitemap... ✓");

    // Run post-build hooks
    plugin_registry.post_build(&dist_dir, project_root)?;

    tracing::info!(
        "Rendering pages... ✓ ({} pages, {} data queries)",
        rendered_pages.len(),
        data_query_count,
    );

    eprintln!(
        "Built {} page(s) in dist/.",
        rendered_pages.len(),
    );
    Ok(())
}

/// Count static vs dynamic pages.
fn count_page_types(pages: &[PageDef]) -> (usize, usize) {
    let mut static_count = 0;
    let mut dynamic_count = 0;
    for page in pages {
        match page.page_type {
            PageType::Static => static_count += 1,
            PageType::Dynamic { .. } => dynamic_count += 1,
        }
    }
    (static_count, dynamic_count)
}

/// Check for output path collision and register the path.
fn register_output_path(
    url_path: &str,
    template_name: &str,
    output_paths: &mut HashSet<String>,
) -> Result<()> {
    if !output_paths.insert(url_path.to_string()) {
        bail!(
            "Output path collision: '{}' is produced by template '{}' \
             but another page already outputs to this path. \
             Check for conflicting static and dynamic page definitions.",
            url_path,
            template_name,
        );
    }
    Ok(())
}

/// Render a single static page.
///
/// 1. Resolve data queries from frontmatter.
/// 2. Build template context.
/// 3. Render template → full HTML string.
/// 4. Write to dist/{output_path}.
/// 5. Extract and write fragments if enabled.
fn render_static_page(
    page: &PageDef,
    env: &minijinja::Environment<'_>,
    fetcher: &mut DataFetcher,
    global_data: &HashMap<String, serde_json::Value>,
    config: &SiteConfig,
    dist_dir: &Path,
    build_time: &str,
    output_paths: &mut HashSet<String>,
    data_query_count: &mut u32,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    plugin_registry: &PluginRegistry,
) -> Result<RenderedPage> {
    let tmpl_name = page.template_path.to_string_lossy().to_string();

    // 1. Resolve data queries.
    *data_query_count += page.frontmatter.data.len() as u32;
    let page_data = data::resolve_page_data(&page.frontmatter, fetcher, Some(plugin_registry))
        .wrap_err_with(|| format!("Failed to resolve data for template '{}'", tmpl_name))?;

    // Compute output path.
    let output_path = page.output_dir.join(
        page.template_path
            .file_name()
            .unwrap_or_default(),
    );
    let url_path = format!("/{}", output_path.to_string_lossy().replace('\\', "/"));

    // Check for output path collision.
    register_output_path(&url_path, &tmpl_name, output_paths)?;

    // 2. Build context.
    let is_index = output_path.file_name()
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

    // 3. Render template.
    let tmpl = env.get_template(&tmpl_name)
        .wrap_err_with(|| format!("Template '{}' not found in environment", tmpl_name))?;

    let rendered = match tmpl.render(&ctx) {
        Ok(html) => html,
        Err(err) => {
            let te = TemplateError::from_minijinja(&err, &tmpl_name, None);
            eprintln!("{}", te.format_console(&tmpl_name, None));
            return Err(eyre::eyre!(
                "Failed to render template '{}': {}",
                tmpl_name, te.short_msg
            ));
        }
    };

    // 4. Write full page (with markers stripped, assets localized, plugins applied).
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
    ).wrap_err_with(|| format!("Plugin post_render_html failed for '{}'", tmpl_name))?;

    let full_path = dist_dir.join(&output_path);

    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("Failed to create output dir {}", parent.display()))?;
    }

    std::fs::write(&full_path, &full_html)
        .wrap_err_with(|| format!("Failed to write {}", full_path.display()))?;

    tracing::debug!("  Static: {} → {} ({} bytes)", tmpl_name, output_path.display(), full_html.len());

    // 5. Extract and write fragments (also localize assets in fragments).
    if config.build.fragments {
        let frags = extract_page_fragments(&rendered, page, &config.build.content_block);
        if !frags.is_empty() {
            let localized_frags = localize_fragments(
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

/// Render all pages for a dynamic template.
///
/// 1. Fetch collection from frontmatter query.
/// 2. For each item: extract slug, resolve nested data, build context, render.
/// 3. Write full pages and fragments.
fn render_dynamic_page(
    page: &PageDef,
    env: &minijinja::Environment<'_>,
    fetcher: &mut DataFetcher,
    global_data: &HashMap<String, serde_json::Value>,
    config: &SiteConfig,
    dist_dir: &Path,
    build_time: &str,
    output_paths: &mut HashSet<String>,
    data_query_count: &mut u32,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    plugin_registry: &PluginRegistry,
) -> Result<Vec<RenderedPage>> {
    let tmpl_name = page.template_path.to_string_lossy().to_string();
    let item_as = &page.frontmatter.item_as;
    let slug_field = &page.frontmatter.slug_field;

    // 1. Fetch collection.
    *data_query_count += 1;
    let items = data::resolve_dynamic_page_data(&page.frontmatter, fetcher, Some(plugin_registry))
        .wrap_err_with(|| format!("Failed to fetch collection for template '{}'", tmpl_name))?;

    if items.is_empty() {
        tracing::debug!("  Dynamic: {} → collection is empty, skipping.", tmpl_name);
        return Ok(Vec::new());
    }

    tracing::debug!(
        "  Dynamic: {} ({} items in collection)",
        tmpl_name,
        items.len(),
    );

    let tmpl = env.get_template(&tmpl_name)
        .wrap_err_with(|| format!("Template '{}' not found in environment", tmpl_name))?;

    let mut rendered_pages = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();

    for (idx, item) in items.iter().enumerate() {
        // Extract slug.
        let slug = match item.get(slug_field) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(_) => {
                tracing::warn!(
                    "Item {} in '{}' has non-string/number slug field '{}', skipping.",
                    idx, tmpl_name, slug_field,
                );
                continue;
            }
            None => {
                tracing::warn!(
                    "Item {} in '{}' is missing slug field '{}', skipping.",
                    idx, tmpl_name, slug_field,
                );
                continue;
            }
        };

        // Sanitize slug: replace problematic chars.
        let slug = slug::slugify(&slug);
        if slug.is_empty() {
            tracing::warn!(
                "Item {} in '{}' has empty slug after sanitization, skipping.",
                idx, tmpl_name,
            );
            continue;
        }

        // Check for duplicate slugs within this dynamic template.
        if !seen_slugs.insert(slug.clone()) {
            bail!(
                "Duplicate slug '{}' in dynamic template '{}'. \
                 Multiple items produced the same slug. \
                 Consider using a different slug_field.",
                slug,
                tmpl_name,
            );
        }

        // Resolve nested data queries for this item.
        *data_query_count += page.frontmatter.data.len() as u32;
        let item_data = data::resolve_dynamic_page_data_for_item(
            &page.frontmatter,
            item,
            fetcher,
            Some(plugin_registry),
        )
        .wrap_err_with(|| {
            format!(
                "Failed to resolve data for item '{}' in template '{}'",
                slug, tmpl_name,
            )
        })?;

        // Compute output path: {output_dir}/{slug}.html
        let output_path = page.output_dir.join(format!("{}.html", slug));
        let url_path = format!("/{}", output_path.to_string_lossy().replace('\\', "/"));

        // Check for output path collision with other templates.
        register_output_path(&url_path, &tmpl_name, output_paths)?;

        // Build context.
        let meta = PageMeta {
            current_url: url_path.clone(),
            current_path: output_path.to_string_lossy().to_string(),
            base_url: config.site.base_url.clone(),
            build_time: build_time.to_string(),
        };

        let ctx = context::build_page_context(
            config,
            global_data,
            &item_data,
            meta,
            Some((item_as, item)),
        );

        // Render.
        let rendered = match tmpl.render(&ctx) {
            Ok(html) => html,
            Err(err) => {
                let te = TemplateError::from_minijinja(&err, &tmpl_name, Some(&slug));
                eprintln!("{}", te.format_console(&tmpl_name, Some(&slug)));
                return Err(eyre::eyre!(
                    "Failed to render template '{}' for item with slug '{}': {}",
                    tmpl_name, slug, te.short_msg
                ));
            }
        };

        // Write full page (with assets localized, plugins applied).
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
        ).wrap_err_with(|| {
            format!("Plugin post_render_html failed for '{}' slug '{}'", tmpl_name, slug)
        })?;

        let full_path = dist_dir.join(&output_path);

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .wrap_err_with(|| format!("Failed to create output dir {}", parent.display()))?;
        }

        std::fs::write(&full_path, &full_html)
            .wrap_err_with(|| format!("Failed to write {}", full_path.display()))?;

        // Write fragments (also localize assets in fragments).
        if config.build.fragments {
            let frags = extract_page_fragments(&rendered, page, &config.build.content_block);
            if !frags.is_empty() {
                let localized_frags = localize_fragments(
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

    tracing::debug!(
        "    → rendered {} page(s) from {}",
        rendered_pages.len(),
        tmpl_name,
    );

    Ok(rendered_pages)
}

/// Localize assets in fragment HTML.
///
/// Since the full page has already been through localization (and all assets
/// are cached), this should be fast — no new downloads.
fn localize_fragments(
    frags: &[fragments::Fragment],
    assets_config: &crate::config::AssetsConfig,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    dist_dir: &Path,
) -> Result<Vec<fragments::Fragment>> {
    let mut result = Vec::with_capacity(frags.len());
    for frag in frags {
        let localized_html = assets::localize_assets(
            &frag.html,
            assets_config,
            asset_cache,
            asset_client,
            dist_dir,
        )?;
        result.push(fragments::Fragment {
            block_name: frag.block_name.clone(),
            html: localized_html,
        });
    }
    Ok(result)
}

/// Extract fragments from rendered HTML according to the page's configuration.
///
/// If `fragment_blocks` is specified in frontmatter, only those blocks are
/// extracted. Otherwise, all blocks found in the rendered HTML are extracted.
fn extract_page_fragments(
    rendered_html: &str,
    page: &PageDef,
    _content_block: &str,
) -> Vec<fragments::Fragment> {
    let all_frags = fragments::extract_fragments(rendered_html);

    match &page.frontmatter.fragment_blocks {
        Some(blocks) => {
            // Only keep fragments whose block name is in the whitelist.
            all_frags
                .into_iter()
                .filter(|f| blocks.contains(&f.block_name))
                .collect()
        }
        None => {
            // Extract all found fragments.
            all_frags
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to write a file, creating parent dirs.
    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Set up a minimal project that can be built.
    fn setup_minimal_project(root: &Path) {
        write(
            root,
            "site.toml",
            r#"
[site]
name = "Test Site"
base_url = "https://test.com"

[build]
fragments = true
"#,
        );

        write(
            root,
            "templates/_base.html",
            "<!DOCTYPE html><html><body>{% block content %}{% endblock %}</body></html>",
        );

        write(
            root,
            "templates/index.html",
            r#"{% extends "_base.html" %}
{% block content %}<h1>Home</h1>{% endblock %}"#,
        );
    }

    #[test]
    fn test_build_minimal_site() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        setup_minimal_project(root);

        build(root).unwrap();

        // Check dist/ exists and has the output.
        assert!(root.join("dist/index.html").exists());

        let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
        assert!(html.contains("<h1>Home</h1>"));
        assert!(html.contains("<!DOCTYPE html>"));

        // Check fragments.
        assert!(root.join("dist/_fragments/index.html").exists());
        let frag = fs::read_to_string(root.join("dist/_fragments/index.html")).unwrap();
        assert!(frag.contains("<h1>Home</h1>"));
        // Fragment should NOT contain the DOCTYPE wrapper.
        assert!(!frag.contains("<!DOCTYPE html>"));

        // Check sitemap.
        assert!(root.join("dist/sitemap.xml").exists());
        let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
        assert!(sitemap.contains("https://test.com/index.html"));
    }

    #[test]
    fn test_build_static_pages_with_data() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Data Test"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(
            root,
            "templates/_base.html",
            "<html><body>{% block content %}{% endblock %}</body></html>",
        );

        write(
            root,
            "templates/index.html",
            r#"---
data:
  nav:
    file: "nav.yaml"
---
{% extends "_base.html" %}
{% block content %}
<nav>{% for item in nav %}<a href="{{ item.url }}">{{ item.label }}</a>{% endfor %}</nav>
{% endblock %}"#,
        );

        write(
            root,
            "_data/nav.yaml",
            "- label: Home\n  url: /\n- label: About\n  url: /about\n",
        );

        build(root).unwrap();

        let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
        assert!(html.contains(r#"<a href="/">Home</a>"#));
        assert!(html.contains(r#"<a href="/about">About</a>"#));
    }

    #[test]
    fn test_build_dynamic_pages() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Dynamic Test"
base_url = "https://test.com"

[build]
fragments = true
"#,
        );

        write(
            root,
            "templates/_base.html",
            "<html><body>{% block content %}{% endblock %}</body></html>",
        );

        write(
            root,
            "templates/posts/[post].html",
            r#"---
collection:
  file: "posts.json"
slug_field: slug
item_as: post
---
{% extends "_base.html" %}
{% block content %}<h1>{{ post.title }}</h1>{% endblock %}"#,
        );

        write(
            root,
            "_data/posts.json",
            r#"[
                {"slug": "hello-world", "title": "Hello World"},
                {"slug": "second-post", "title": "Second Post"}
            ]"#,
        );

        build(root).unwrap();

        // Check generated pages.
        assert!(root.join("dist/posts/hello-world.html").exists());
        assert!(root.join("dist/posts/second-post.html").exists());

        let hello = fs::read_to_string(root.join("dist/posts/hello-world.html")).unwrap();
        assert!(hello.contains("Hello World"));

        let second = fs::read_to_string(root.join("dist/posts/second-post.html")).unwrap();
        assert!(second.contains("Second Post"));

        // Check fragments.
        assert!(root.join("dist/_fragments/posts/hello-world.html").exists());
        assert!(root.join("dist/_fragments/posts/second-post.html").exists());

        // Check sitemap includes both.
        let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
        assert!(sitemap.contains("/posts/hello-world.html"));
        assert!(sitemap.contains("/posts/second-post.html"));
    }

    #[test]
    fn test_build_dynamic_empty_collection() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Empty Collection"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(root, "templates/_base.html", "<html>{% block content %}{% endblock %}</html>");

        write(
            root,
            "templates/[item].html",
            r#"---
collection:
  file: "items.json"
---
{% extends "_base.html" %}
{% block content %}<p>{{ item.name }}</p>{% endblock %}"#,
        );

        write(root, "_data/items.json", "[]");

        build(root).unwrap();

        // No pages should be generated for empty collection.
        let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
        assert!(!sitemap.contains("<url>"));
    }

    #[test]
    fn test_build_static_assets_copied() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        setup_minimal_project(root);
        write(root, "static/css/style.css", "body { color: red; }");
        write(root, "static/favicon.ico", "icon");

        build(root).unwrap();

        assert!(root.join("dist/css/style.css").exists());
        assert!(root.join("dist/favicon.ico").exists());

        let css = fs::read_to_string(root.join("dist/css/style.css")).unwrap();
        assert_eq!(css, "body { color: red; }");
    }

    #[test]
    fn test_build_no_fragments_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "No Frags"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(
            root,
            "templates/_base.html",
            "<html>{% block content %}{% endblock %}</html>",
        );

        write(
            root,
            "templates/index.html",
            r#"{% extends "_base.html" %}
{% block content %}<h1>Home</h1>{% endblock %}"#,
        );

        build(root).unwrap();

        assert!(root.join("dist/index.html").exists());
        assert!(!root.join("dist/_fragments").exists());

        // Full page should not contain fragment markers.
        let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
        assert!(!html.contains("<!--FRAG:"));
    }

    #[test]
    fn test_build_cleans_previous_output() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        setup_minimal_project(root);

        // Create stale file in dist/.
        write(root, "dist/stale.html", "old content");

        build(root).unwrap();

        // Stale file should be gone.
        assert!(!root.join("dist/stale.html").exists());
        // New content should be there.
        assert!(root.join("dist/index.html").exists());
    }

    #[test]
    fn test_build_multiple_static_pages() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Multi"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(root, "templates/_base.html", "<html>{% block content %}{% endblock %}</html>");
        write(
            root,
            "templates/index.html",
            r#"{% extends "_base.html" %}{% block content %}Home{% endblock %}"#,
        );
        write(
            root,
            "templates/about.html",
            r#"{% extends "_base.html" %}{% block content %}About{% endblock %}"#,
        );
        write(
            root,
            "templates/docs/guide.html",
            r#"{% extends "_base.html" %}{% block content %}Guide{% endblock %}"#,
        );

        build(root).unwrap();

        assert!(root.join("dist/index.html").exists());
        assert!(root.join("dist/about.html").exists());
        assert!(root.join("dist/docs/guide.html").exists());

        let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
        assert!(sitemap.contains("/index.html"));
        assert!(sitemap.contains("/about.html"));
        assert!(sitemap.contains("/docs/guide.html"));
    }

    #[test]
    fn test_build_dynamic_numeric_slug() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Numeric Slug"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(root, "templates/_base.html", "<html>{% block content %}{% endblock %}</html>");

        write(
            root,
            "templates/[item].html",
            r#"---
collection:
  file: "items.json"
slug_field: id
---
{% extends "_base.html" %}
{% block content %}<p>{{ item.title }}</p>{% endblock %}"#,
        );

        write(
            root,
            "_data/items.json",
            r#"[{"id": 1, "title": "First"}, {"id": 2, "title": "Second"}]"#,
        );

        build(root).unwrap();

        assert!(root.join("dist/1.html").exists());
        assert!(root.join("dist/2.html").exists());
    }

    #[test]
    fn test_build_page_context_available() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Context Test"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(root, "templates/_base.html", "<html>{% block content %}{% endblock %}</html>");

        write(
            root,
            "templates/about.html",
            r#"{% extends "_base.html" %}
{% block content %}URL:{{ page.current_url }} PATH:{{ page.current_path }}{% endblock %}"#,
        );

        build(root).unwrap();

        let html = fs::read_to_string(root.join("dist/about.html")).unwrap();
        assert!(html.contains("URL:/about.html"));
        assert!(html.contains("PATH:about.html"));
    }

    #[test]
    fn test_build_dynamic_duplicate_slugs_error() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Dup Slug"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(root, "templates/_base.html", "<html>{% block content %}{% endblock %}</html>");

        write(
            root,
            "templates/[item].html",
            r#"---
collection:
  file: "items.json"
slug_field: slug
---
{% extends "_base.html" %}
{% block content %}{{ item.title }}{% endblock %}"#,
        );

        write(
            root,
            "_data/items.json",
            r#"[
                {"slug": "same-slug", "title": "First"},
                {"slug": "same-slug", "title": "Second"}
            ]"#,
        );

        let result = build(root);
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("Duplicate slug"));
        assert!(err.contains("same-slug"));
    }

    #[test]
    fn test_build_dynamic_slug_special_chars_sanitized() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Slug Sanitize"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(root, "templates/_base.html", "<html>{% block content %}{% endblock %}</html>");

        write(
            root,
            "templates/[item].html",
            r#"---
collection:
  file: "items.json"
slug_field: slug
---
{% extends "_base.html" %}
{% block content %}{{ item.title }}{% endblock %}"#,
        );

        write(
            root,
            "_data/items.json",
            r#"[{"slug": "Hello World / Special!", "title": "Test"}]"#,
        );

        build(root).unwrap();

        // The slug should be sanitized to something safe.
        assert!(root.join("dist/hello-world-special.html").exists());
    }

    #[test]
    fn test_build_missing_layout_error() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Missing Layout"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        // No _base.html layout file!
        write(
            root,
            "templates/index.html",
            r#"{% extends "_missing_layout.html" %}
{% block content %}<h1>Home</h1>{% endblock %}"#,
        );

        let result = build(root);
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        // Should mention the template and the missing layout.
        assert!(err.contains("index.html") || err.contains("_missing_layout.html"));
    }

    #[test]
    fn test_build_undefined_variable_error() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Undef Var"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        write(root, "templates/index.html", "<h1>{{ undefined_var }}</h1>");

        let result = build(root);
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("undefined") || err.contains("unknown variable"));
    }

    #[test]
    fn test_build_empty_templates_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Empty Templates"
base_url = "https://test.com"

[build]
fragments = false
"#,
        );

        // Create empty templates directory.
        std::fs::create_dir_all(root.join("templates")).unwrap();

        // Should succeed, producing empty dist with just static assets.
        build(root).unwrap();

        assert!(root.join("dist").is_dir());
        let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
        assert!(!sitemap.contains("<url>"));
    }

    #[test]
    fn test_build_missing_site_toml() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let result = build(root);
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("site.toml"));
        assert!(err.contains("eigen init"));
    }
}
