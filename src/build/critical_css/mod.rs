//! Critical CSS inlining: extract per-page used CSS and inline it to
//! eliminate render-blocking stylesheets.
//!
//! This module provides `inline_critical_css`, the main entry point called
//! from the build pipeline. It:
//! 1. Parses HTML to find `<link rel="stylesheet">` tags.
//! 2. Reads each referenced local stylesheet from dist_dir.
//! 3. Parses the CSS and matches selectors against the HTML DOM.
//! 4. Inlines matched CSS as a `<style>` block in `<head>`.
//! 5. Rewrites `<link>` tags to load asynchronously or removes them.

pub mod extract;
pub mod rewrite;

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::config::CriticalCssConfig;

/// Matches `@import` directives in CSS (both `url()` and string forms).
static IMPORT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"@import\s+(?:url\(\s*['"]?([^'")]+)['"]?\s*\)|['"]([^'"]+)['"]);?"#
    ).expect("import regex is valid")
});

/// Cache for parsed stylesheets, keyed by href.
///
/// Avoids re-reading and re-resolving the same CSS file for every page.
/// Created once per build and passed through the pipeline.
pub struct StylesheetCache {
    cache: HashMap<String, String>,
}

impl StylesheetCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Get or load a stylesheet's CSS content.
    ///
    /// Returns `Some(&str)` if the stylesheet is cached or can be loaded,
    /// `None` if loading fails.
    fn get_or_load(
        &mut self,
        href: &str,
        dist_dir: &Path,
        manifest: Option<&crate::build::content_hash::AssetManifest>,
    ) -> Option<&str> {
        if !self.cache.contains_key(href) {
            match load_stylesheet(href, dist_dir, manifest) {
                Ok(css) => {
                    self.cache.insert(href.to_string(), css);
                }
                Err(e) => {
                    tracing::warn!("Failed to load stylesheet '{}': {}", href, e);
                    return None;
                }
            }
        }
        self.cache.get(href).map(|s| s.as_str())
    }
}

/// Extract critical CSS and inline it into the HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
    css_cache: &mut StylesheetCache,
    manifest: Option<&crate::build::content_hash::AssetManifest>,
) -> String {
    if !config.enabled {
        return html.to_string();
    }

    // Step 1: Find local stylesheet links.
    let hrefs = rewrite::extract_stylesheet_hrefs(html, &config.exclude);
    if hrefs.is_empty() {
        return html.to_string();
    }

    // Step 2: Parse the HTML into a DOM for selector matching.
    let document = scraper::Html::parse_document(html);

    // Step 3: Load and extract critical CSS from each stylesheet.
    let mut combined_critical_css = String::new();
    let mut processed_hrefs: Vec<String> = Vec::new();

    for href in &hrefs {
        let css_content = match css_cache.get_or_load(href, dist_dir, manifest) {
            Some(css) => css.to_string(),
            None => continue, // Warning already logged by get_or_load.
        };

        match extract::extract_critical_css(&css_content, &document) {
            Ok(critical) => {
                if !critical.is_empty() {
                    if !combined_critical_css.is_empty() {
                        combined_critical_css.push('\n');
                    }
                    combined_critical_css.push_str(&critical);
                    processed_hrefs.push(href.clone());
                }
            }
            Err(e) => {
                tracing::warn!("Failed to extract critical CSS from '{}': {}", href, e);
            }
        }
    }

    if combined_critical_css.is_empty() {
        return html.to_string();
    }

    // Step 4: Check size limit.
    if combined_critical_css.len() > config.max_inline_size {
        tracing::info!(
            "Critical CSS ({} bytes) exceeds max_inline_size ({} bytes), skipping inlining",
            combined_critical_css.len(),
            config.max_inline_size,
        );
        return html.to_string();
    }

    // Step 5: Rewrite HTML.
    match rewrite::rewrite_html(
        html,
        &combined_critical_css,
        &processed_hrefs,
        config.preload_full,
    ) {
        Ok(rewritten) => rewritten,
        Err(e) => {
            tracing::warn!("Failed to rewrite HTML for critical CSS: {}", e);
            html.to_string()
        }
    }
}

/// Read a CSS file from dist_dir, resolving the href to a filesystem path.
///
/// Resolves `@import` directives by inlining imported files. Uses a simple
/// recursive approach rather than lightningcss's bundler API (which requires
/// a `SourceProvider` trait implementation).
///
/// External `@import` URLs (http/https) are left as-is.
fn load_stylesheet(
    href: &str,
    dist_dir: &Path,
    manifest: Option<&crate::build::content_hash::AssetManifest>,
) -> Result<String, String> {
    let relative = href.trim_start_matches('/');
    let css_path = dist_dir.join(relative);

    if !css_path.exists() {
        // Try resolving through manifest (file may have been renamed by content hashing).
        if let Some(m) = manifest {
            let url_path = if href.starts_with('/') {
                href.to_string()
            } else {
                format!("/{}", href)
            };
            let resolved = m.resolve(&url_path);
            if resolved != url_path {
                let resolved_relative = resolved.trim_start_matches('/');
                let resolved_path = dist_dir.join(resolved_relative);
                if resolved_path.exists() {
                    let css = std::fs::read_to_string(&resolved_path)
                        .map_err(|e| format!("Failed to read {}: {e}", resolved_path.display()))?;
                    return resolve_imports(&css, &resolved_path, dist_dir, 0);
                }
            }
        }
        return Err(format!("File not found: {}", css_path.display()));
    }

    let css = std::fs::read_to_string(&css_path)
        .map_err(|e| format!("Failed to read {}: {e}", css_path.display()))?;

    // Resolve @import directives.
    resolve_imports(&css, &css_path, dist_dir, 0)
}

/// Recursively resolve `@import` directives in CSS content.
/// `depth` prevents infinite recursion from circular imports.
fn resolve_imports(
    css: &str,
    css_path: &Path,
    dist_dir: &Path,
    depth: usize,
) -> Result<String, String> {
    const MAX_IMPORT_DEPTH: usize = 10;

    if depth > MAX_IMPORT_DEPTH {
        tracing::warn!(
            "Circular or deeply nested @import detected in {}",
            css_path.display()
        );
        return Ok(css.to_string());
    }

    let css_dir = css_path.parent().unwrap_or(dist_dir);
    let mut result = css.to_string();

    // Collect matches first to avoid borrow issues with iterative replacement.
    let captures: Vec<_> = IMPORT_RE.captures_iter(css).collect();

    for cap in captures {
        let import_path_str = cap.get(1).or(cap.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");

        // Skip external imports.
        if import_path_str.starts_with("http://") || import_path_str.starts_with("https://") {
            continue;
        }

        if import_path_str.is_empty() {
            continue;
        }

        // Resolve relative to the importing file's directory.
        let import_path = if import_path_str.starts_with('/') {
            dist_dir.join(import_path_str.trim_start_matches('/'))
        } else {
            css_dir.join(import_path_str)
        };

        if import_path.exists() {
            match std::fs::read_to_string(&import_path) {
                Ok(imported_css) => {
                    let resolved = resolve_imports(
                        &imported_css, &import_path, dist_dir, depth + 1
                    )?;
                    if let Some(full_match) = cap.get(0) {
                        result = result.replace(full_match.as_str(), &resolved);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to read @import '{}': {e}",
                        import_path.display(),
                    );
                }
            }
        } else {
            tracing::warn!(
                "File not found for @import '{}' in {}",
                import_path_str, css_path.display()
            );
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to write a file, creating parent dirs.
    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_inline_critical_css_disabled() {
        let config = CriticalCssConfig::default(); // enabled: false
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let result = inline_critical_css(html, &config, Path::new("/tmp"), &mut cache, None);
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_no_links() {
        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = "<html><head><title>No CSS</title></head><body><p>Hi</p></body></html>";
        let result = inline_critical_css(html, &config, Path::new("/tmp"), &mut cache, None);
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_file_not_found() {
        let tmp = TempDir::new().unwrap();
        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/missing.css"></head></html>"#;
        let result = inline_critical_css(html, &config, tmp.path(), &mut cache, None);
        // Should return original HTML unchanged.
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_basic() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", r#"
            .hero { color: red; }
            .unused { color: green; }
        "#);

        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache, None);

        assert!(result.contains("<style>"));
        assert!(result.contains(".hero"));
        assert!(!result.contains(".unused"));
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains("<noscript>"));
    }

    #[test]
    fn test_inline_critical_css_exceeds_max_size() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        let large_css = ".exists { color: red; padding: 0; margin: 0; border: none; }";
        write_file(dist, "css/style.css", large_css);

        let config = CriticalCssConfig {
            enabled: true,
            max_inline_size: 10, // Very small limit.
            ..Default::default()
        };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="exists">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache, None);

        // Should return original HTML unchanged (no inlining).
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_excluded_pattern() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/vendor/bootstrap.css", ".btn { color: red; }");

        let config = CriticalCssConfig {
            enabled: true,
            exclude: vec!["**/vendor/**".to_string()],
            ..Default::default()
        };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/vendor/bootstrap.css"></head><body><div class="btn">Click</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache, None);

        // Excluded stylesheet should not be processed.
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_preload_false() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", ".hero { color: red; }");

        let config = CriticalCssConfig {
            enabled: true,
            preload_full: false,
            ..Default::default()
        };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache, None);

        assert!(result.contains("<style>"));
        assert!(result.contains(".hero"));
        // No preload or noscript.
        assert!(!result.contains("preload"));
        assert!(!result.contains("<noscript>"));
    }

    #[test]
    fn test_inline_critical_css_with_import() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/base.css", ".base { margin: 0; }");
        write_file(dist, "css/style.css", r#"
            @import "base.css";
            .hero { color: red; }
            .unused { color: green; }
        "#);

        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero base">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache, None);

        assert!(result.contains("<style>"));
        assert!(result.contains(".hero"));
        assert!(result.contains(".base"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_stylesheet_cache_reuse() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", ".hero { color: red; }");

        let mut cache = StylesheetCache::new();

        // First load.
        let css1 = cache.get_or_load("/css/style.css", dist, None)
            .map(|s| s.to_string());
        assert!(css1.is_some());

        // Second load should hit cache (even if file is deleted).
        fs::remove_file(dist.join("css/style.css")).unwrap();
        let css2 = cache.get_or_load("/css/style.css", dist, None)
            .map(|s| s.to_string());
        assert!(css2.is_some());
        assert_eq!(css1, css2);
    }

    #[test]
    fn test_get_or_load_with_manifest_fallback() {
        use crate::build::content_hash::AssetManifest;

        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        // File exists at hashed path, not at original.
        let css = "body { color: red; }";
        let hashed_dir = dist.join("css");
        fs::create_dir_all(&hashed_dir).unwrap();
        fs::write(hashed_dir.join("style.abc123.css"), css).unwrap();

        let mut manifest = AssetManifest::new();
        manifest.insert(
            "/css/style.css".into(),
            "/css/style.abc123.css".into(),
        );

        let mut cache = StylesheetCache::new();
        let result = cache.get_or_load("/css/style.css", dist, Some(&manifest));
        assert!(result.is_some(), "should resolve via manifest fallback");
        assert_eq!(result.unwrap(), css);
    }

    #[test]
    fn test_get_or_load_without_manifest() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        // File exists at original path.
        let css = "body { color: blue; }";
        let css_dir = dist.join("css");
        fs::create_dir_all(&css_dir).unwrap();
        fs::write(css_dir.join("style.css"), css).unwrap();

        let mut cache = StylesheetCache::new();
        let result = cache.get_or_load("/css/style.css", dist, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), css);
    }
}
