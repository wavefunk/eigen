//! Step 4.4: Custom minijinja functions.
//!
//! Registers the following global functions:
//!
//! - `link_to(path, target?, block?)` — generates HTMX-compatible link attributes
//! - `current_year()` — returns the current year as a string
//! - `asset(path)` — returns the path to a static asset (for future cache-busting)

use std::sync::Arc;

use minijinja::Environment;
use minijinja::Value;

use crate::build::content_hash::AssetManifest;
use crate::config::SiteConfig;

/// Register all custom functions on the given environment.
pub fn register_functions(
    env: &mut Environment<'_>,
    config: &SiteConfig,
    manifest: Option<Arc<AssetManifest>>,
) {
    let fragment_dir = config.build.fragment_dir.clone();
    let fragments_enabled = config.build.fragments;
    let content_block = config.build.content_block.clone();

    // link_to(path, target?, block?)
    env.add_function(
        "link_to",
        move |path: &str,
              target: Option<&str>,
              block: Option<&str>|
              -> String {
            let target = target.unwrap_or("#content");

            if !fragments_enabled {
                // Without fragments, just return a plain href.
                return format!(r#"href="{}""#, path);
            }

            let block_name = block.unwrap_or(&content_block);
            let fragment_path = compute_fragment_path(path, &fragment_dir, block_name);

            format!(
                r#"href="{path}" hx-get="{fragment_path}" hx-target="{target}" hx-push-url="{path}""#,
                path = path,
                fragment_path = fragment_path,
                target = target,
            )
        },
    );

    // current_year()
    env.add_function("current_year", || -> String {
        chrono::Local::now().format("%Y").to_string()
    });

    // asset(path)
    // Returns the content-hashed path when a manifest is available,
    // otherwise passes through unchanged.
    let manifest_clone = manifest;
    env.add_function("asset", move |path: &str| -> String {
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        match &manifest_clone {
            Some(m) => m.resolve(&normalized).to_string(),
            None => normalized,
        }
    });

    // site — expose the full [site] config as a global variable.
    env.add_global(
        "site",
        Value::from_serialize(&config.site),
    );
}

/// Compute the fragment path for a given page path and block name.
///
/// Examples:
/// - `("/about.html", "_fragments", "content")` → `"/_fragments/about.html"`
/// - `("/posts/my-post.html", "_fragments", "content")` → `"/_fragments/posts/my-post.html"`
/// - `("/about.html", "_fragments", "sidebar")` → `"/_fragments/about/sidebar.html"`
///
/// The default content block uses the page filename directly. Non-default blocks
/// get their own subdirectory.
fn compute_fragment_path(page_path: &str, fragment_dir: &str, block: &str) -> String {
    let clean_path = page_path.trim_start_matches('/');

    // Normalise directory-style URLs ("/about/" or "/about") to a stem ("about")
    // and derive the equivalent .html path ("about/index.html" or "about.html").
    let (stem, html_path) = if clean_path.ends_with('/') {
        // "/about/" → stem="about", html_path="about/index.html"
        let s = clean_path.trim_end_matches('/');
        (s, format!("{}/index.html", s))
    } else if clean_path.is_empty() {
        // "/" → root index
        ("index", "index.html".to_string())
    } else if let Some(s) = clean_path.strip_suffix(".html") {
        // "/about.html" → stem="about", html_path="about.html"
        (s, clean_path.to_string())
    } else {
        // "/about" (no trailing slash, no extension) — treat as directory
        let s = clean_path;
        (s, format!("{}/index.html", s))
    };

    // For the default content block, the fragment file mirrors the html path.
    // For additional blocks, nest under a directory named after the stem.
    if block == "content" {
        format!("/{}/{}", fragment_dir, html_path)
    } else {
        format!("/{}/{}/{}.html", fragment_dir, stem, block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BuildConfig, SiteSchemaConfig, SiteMeta, SiteSeoConfig};
    use minijinja::context;
    use std::collections::HashMap;

    fn test_config() -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test Site".into(),
                base_url: "https://example.com".into(),
                seo: SiteSeoConfig::default(),
                schema: SiteSchemaConfig::default(),
                extra: std::collections::HashMap::new(),
            },
            build: BuildConfig {
                fragments: true,
                fragment_dir: "_fragments".into(),
                content_block: "content".into(),
                ..Default::default()
            },
            sitemap: Default::default(),
            robots: Default::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            analytics: None,
            plugins: HashMap::new(),
            feed: HashMap::new(),
            robots: None,
            audit: None,
        }
    }

    fn test_config_no_fragments() -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test Site".into(),
                base_url: "https://example.com".into(),
                seo: SiteSeoConfig::default(),
                schema: SiteSchemaConfig::default(),
                extra: std::collections::HashMap::new(),
            },
            build: BuildConfig {
                fragments: false,
                ..Default::default()
            },
            sitemap: Default::default(),
            robots: Default::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            analytics: None,
            plugins: HashMap::new(),
            feed: HashMap::new(),
            robots: None,
            audit: None,
        }
    }

    // --- compute_fragment_path ---

    #[test]
    fn test_fragment_path_content_block() {
        let result = compute_fragment_path("/about.html", "_fragments", "content");
        assert_eq!(result, "/_fragments/about.html");
    }

    #[test]
    fn test_fragment_path_nested() {
        let result = compute_fragment_path("/posts/my-post.html", "_fragments", "content");
        assert_eq!(result, "/_fragments/posts/my-post.html");
    }

    #[test]
    fn test_fragment_path_non_content_block() {
        let result = compute_fragment_path("/about.html", "_fragments", "sidebar");
        assert_eq!(result, "/_fragments/about/sidebar.html");
    }

    #[test]
    fn test_fragment_path_no_leading_slash() {
        let result = compute_fragment_path("about.html", "_fragments", "content");
        assert_eq!(result, "/_fragments/about.html");
    }

    #[test]
    fn test_fragment_path_directory_trailing_slash() {
        let result = compute_fragment_path("/about/", "_fragments", "content");
        assert_eq!(result, "/_fragments/about/index.html");
    }

    #[test]
    fn test_fragment_path_directory_no_trailing_slash() {
        let result = compute_fragment_path("/about", "_fragments", "content");
        assert_eq!(result, "/_fragments/about/index.html");
    }

    #[test]
    fn test_fragment_path_directory_non_content_block() {
        let result = compute_fragment_path("/about/", "_fragments", "sidebar");
        assert_eq!(result, "/_fragments/about/sidebar.html");
    }

    #[test]
    fn test_fragment_path_nested_directory() {
        let result = compute_fragment_path("/case-study/flipkart/", "_fragments", "content");
        assert_eq!(result, "/_fragments/case-study/flipkart/index.html");
    }

    // --- link_to function ---

    #[test]
    fn test_link_to_default() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", r##"<a {{ link_to("/about.html") }}>About</a>"##)
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();

        assert!(result.contains(r##"href="/about.html""##));
        assert!(result.contains(r##"hx-get="/_fragments/about.html""##));
        assert!(result.contains(r##"hx-target="#content""##));
        assert!(result.contains(r##"hx-push-url="/about.html""##));
    }

    #[test]
    fn test_link_to_custom_target() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", r##"{{ link_to("/about.html", "#main") }}"##)
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();

        assert!(result.contains(r##"hx-target="#main""##));
    }

    #[test]
    fn test_link_to_custom_block() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", r##"{{ link_to("/about.html", "#sidebar", "sidebar") }}"##)
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();

        assert!(result.contains(r##"hx-get="/_fragments/about/sidebar.html""##));
    }

    #[test]
    fn test_link_to_no_fragments() {
        let mut env = Environment::new();
        let config = test_config_no_fragments();
        register_functions(&mut env, &config, None);

        env.add_template("test", r##"{{ link_to("/about.html") }}"##)
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();

        assert_eq!(result.trim(), r##"href="/about.html""##);
        assert!(!result.contains("hx-get"));
    }

    // --- current_year ---

    #[test]
    fn test_current_year() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", "{{ current_year() }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();

        let year: u32 = result.trim().parse().expect("should be a year number");
        assert!(year >= 2024);
    }

    // --- asset ---

    #[test]
    fn test_asset_with_leading_slash() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", "{{ asset('/css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.css");
    }

    #[test]
    fn test_asset_without_leading_slash() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", "{{ asset('css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.css");
    }

    // --- site global ---

    #[test]
    fn test_site_global() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", "{{ site.name }} - {{ site.base_url }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "Test Site - https://example.com");
    }

    // --- asset with manifest ---

    #[test]
    fn test_asset_with_manifest() {
        let mut env = Environment::new();
        let config = test_config();
        let mut manifest = AssetManifest::new();
        manifest.insert("/css/style.css".into(), "/css/style.abc123.css".into());
        let manifest = Arc::new(manifest);

        register_functions(&mut env, &config, Some(manifest));

        env.add_template("test", "{{ asset('/css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.abc123.css");
    }

    #[test]
    fn test_asset_without_manifest() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", "{{ asset('/css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.css");
    }

    #[test]
    fn test_asset_unknown_path_with_manifest() {
        let mut env = Environment::new();
        let config = test_config();
        let manifest = Arc::new(AssetManifest::new());

        register_functions(&mut env, &config, Some(manifest));

        env.add_template("test", "{{ asset('/unknown.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/unknown.css");
    }

    #[test]
    fn test_asset_normalizes_then_resolves() {
        let mut env = Environment::new();
        let config = test_config();
        let mut manifest = AssetManifest::new();
        manifest.insert("/css/style.css".into(), "/css/style.abc123.css".into());
        let manifest = Arc::new(manifest);

        register_functions(&mut env, &config, Some(manifest));

        // Path without leading slash should be normalized and then resolved.
        env.add_template("test", "{{ asset('css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.abc123.css");
    }
}
