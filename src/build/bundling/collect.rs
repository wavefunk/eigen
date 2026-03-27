//! HTML scanning for CSS and JS references.
//!
//! Scans all HTML files in dist/ and extracts `<link rel="stylesheet">`
//! hrefs and `<script src>` srcs, deduplicating in first-encounter order.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eyre::{Result, WrapErr};
use walkdir::WalkDir;

/// Collected references from all rendered HTML files.
pub struct CollectedRefs {
    /// Ordered list of unique local CSS hrefs found across all HTML files,
    /// in first-encounter order.
    pub css_hrefs: Vec<String>,

    /// Ordered list of unique local JS srcs found across all HTML files,
    /// in first-encounter order.
    pub js_srcs: Vec<String>,

    /// Paths to all HTML files in dist/ (full pages and fragments).
    /// Used for rewriting and for tree-shaking DOM parsing.
    pub html_files: Vec<PathBuf>,
}

/// Scan all HTML files in dist/ and collect `<link rel="stylesheet">`
/// hrefs and `<script src="...">` srcs.
///
/// Returns deduplicated lists in first-encounter order across all pages.
/// Also returns the list of HTML file paths for rewriting.
///
/// Skips:
/// - External URLs (http://, https://)
/// - `<link>` tags with a `media` attribute (e.g. print stylesheets)
/// - `<script>` tags with `defer`, `async`, or `type="module"`
/// - `<script>` tags without a `src` attribute (inline scripts)
/// - Paths matching the `exclude` glob patterns
///
/// HTML files are sorted by path before processing to ensure
/// deterministic merge order.
pub fn collect_references(
    dist_dir: &Path,
    exclude: &[String],
) -> Result<CollectedRefs> {
    // Collect and sort all HTML file paths for deterministic ordering.
    let mut html_files: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(dist_dir)
        .sort_by_file_name()
        .into_iter()
    {
        let entry = entry.wrap_err("Failed to read directory entry during reference collection")?;

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("html") {
            html_files.push(path.to_path_buf());
        }
    }

    if html_files.is_empty() {
        return Ok(CollectedRefs {
            css_hrefs: Vec::new(),
            js_srcs: Vec::new(),
            html_files: Vec::new(),
        });
    }

    let mut css_hrefs: Vec<String> = Vec::new();
    let mut css_seen: HashSet<String> = HashSet::new();
    let mut js_srcs: Vec<String> = Vec::new();
    let mut js_seen: HashSet<String> = HashSet::new();

    // Compile exclude patterns once.
    let exclude_patterns: Vec<glob::Pattern> = exclude
        .iter()
        .filter_map(|p| match glob::Pattern::new(p) {
            Ok(pat) => Some(pat),
            Err(e) => {
                tracing::warn!("Invalid exclude pattern '{}': {}", p, e);
                None
            }
        })
        .collect();

    for html_path in &html_files {
        let html_content = match std::fs::read_to_string(html_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to read HTML file '{}': {}",
                    html_path.display(), e
                );
                continue;
            }
        };

        let page_css: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let page_js: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let page_css_clone = page_css.clone();
        let page_js_clone = page_js.clone();
        let exclude_patterns_css = exclude_patterns.clone();
        let exclude_patterns_js = exclude_patterns.clone();

        let _ = lol_html::rewrite_str(
            &html_content,
            lol_html::RewriteStrSettings {
                element_content_handlers: vec![
                    // Collect CSS hrefs.
                    lol_html::element!("link[rel='stylesheet']", move |el| {
                        // Skip links with media attribute.
                        if el.get_attribute("media").is_some() {
                            return Ok(());
                        }

                        let href = match el.get_attribute("href") {
                            Some(h) if !h.is_empty() => h,
                            _ => return Ok(()),
                        };

                        // Skip external URLs.
                        if href.starts_with("http://") || href.starts_with("https://") {
                            return Ok(());
                        }

                        // Check exclude patterns.
                        if exclude_patterns_css.iter().any(|p| p.matches(&href)) {
                            return Ok(());
                        }

                        page_css_clone.borrow_mut().push(href);
                        Ok(())
                    }),
                    // Collect JS srcs.
                    lol_html::element!("script", move |el| {
                        // Skip inline scripts (no src).
                        let src = match el.get_attribute("src") {
                            Some(s) if !s.is_empty() => s,
                            _ => return Ok(()),
                        };

                        // Skip external URLs.
                        if src.starts_with("http://") || src.starts_with("https://") {
                            return Ok(());
                        }

                        // Skip defer, async, type="module".
                        if el.get_attribute("defer").is_some()
                            || el.get_attribute("async").is_some()
                        {
                            return Ok(());
                        }
                        if el.get_attribute("type").as_deref() == Some("module") {
                            return Ok(());
                        }

                        // Check exclude patterns.
                        if exclude_patterns_js.iter().any(|p| p.matches(&src)) {
                            return Ok(());
                        }

                        page_js_clone.borrow_mut().push(src);
                        Ok(())
                    }),
                ],
                ..lol_html::RewriteStrSettings::new()
            },
        );

        // Deduplicate: add to ordered lists if not seen before.
        for href in page_css.borrow().iter() {
            if css_seen.insert(href.clone()) {
                css_hrefs.push(href.clone());
            }
        }
        for src in page_js.borrow().iter() {
            if js_seen.insert(src.clone()) {
                js_srcs.push(src.clone());
            }
        }
    }

    Ok(CollectedRefs {
        css_hrefs,
        js_srcs,
        html_files,
    })
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
    fn test_collect_single_css() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
        assert!(refs.js_srcs.is_empty());
        assert_eq!(refs.html_files.len(), 1);
    }

    #[test]
    fn test_collect_multiple_css() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/reset.css", "/css/style.css"]);
    }

    #[test]
    fn test_collect_dedup_css() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#);
        write_file(dist, "about.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
        assert_eq!(refs.html_files.len(), 2);
    }

    #[test]
    fn test_collect_skips_external() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="https://cdn.example.com/lib.css">
            <link rel="stylesheet" href="/css/local.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/local.css"]);
    }

    #[test]
    fn test_collect_skips_media() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/print.css" media="print">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
    }

    #[test]
    fn test_collect_skips_excluded() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/vendor/bootstrap.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &["**/vendor/**".to_string()]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
    }

    #[test]
    fn test_collect_single_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body><script src="/js/app.js"></script></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert!(refs.css_hrefs.is_empty());
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_skips_module_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body>
            <script type="module" src="/js/mod.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_skips_async_defer() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body>
            <script defer src="/js/deferred.js"></script>
            <script async src="/js/async.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_skips_inline_script() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body>
            <script>console.log("inline");</script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_no_html_files() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        let refs = collect_references(dist, &[]).unwrap();
        assert!(refs.css_hrefs.is_empty());
        assert!(refs.js_srcs.is_empty());
        assert!(refs.html_files.is_empty());
    }

    #[test]
    fn test_collect_deterministic_order() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        // Two pages with different CSS sets -- order should be deterministic.
        write_file(dist, "a.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/a.css">
        </head><body></body></html>"#);
        write_file(dist, "b.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/b.css">
        </head><body></body></html>"#);

        let refs1 = collect_references(dist, &[]).unwrap();
        let refs2 = collect_references(dist, &[]).unwrap();
        assert_eq!(refs1.css_hrefs, refs2.css_hrefs);
        // reset.css should be first (encountered in a.html which sorts before b.html).
        assert_eq!(refs1.css_hrefs[0], "/css/reset.css");
    }
}
