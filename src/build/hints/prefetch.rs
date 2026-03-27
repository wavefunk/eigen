//! Navigation prefetch: scan HTML for hx-get/href attributes, collect
//! unique local URLs, filter self-references and exclusions.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Scan HTML for navigation link URLs suitable for prefetching.
///
/// When `fragments_enabled` is true, scans `hx-get` attributes.
/// When false, scans `<a href>` attributes (excluding `target="_blank"`).
///
/// Excludes:
/// - External URLs (http:// or https://)
/// - Anchor-only links (#...)
/// - The current page's own URL (self-reference)
/// - URLs matching exclude patterns (glob)
/// - Duplicates (first occurrence wins)
///
/// Returns at most `max_prefetch` URLs in document order.
pub fn collect_prefetch_urls(
    html: &str,
    current_url: &str,
    fragment_dir: &str,
    fragments_enabled: bool,
    max_prefetch: usize,
    exclude_patterns: &[String],
) -> Vec<String> {
    if max_prefetch == 0 {
        return Vec::new();
    }

    // Compile exclude patterns, skipping invalid ones with a warning.
    let patterns: Vec<glob::Pattern> = exclude_patterns
        .iter()
        .filter_map(|p| match glob::Pattern::new(p) {
            Ok(pat) => Some(pat),
            Err(e) => {
                tracing::warn!(
                    "Invalid glob pattern in exclude_prefetch '{}': {}",
                    p,
                    e
                );
                None
            }
        })
        .collect();

    // Derive the self-reference URL to exclude.
    let self_url = compute_self_url(current_url, fragment_dir, fragments_enabled);

    // Collect URLs from HTML via lol_html.
    let urls = if fragments_enabled {
        collect_hx_get_urls(html)
    } else {
        collect_href_urls(html)
    };

    // Filter and deduplicate.
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for url in urls {
        if result.len() >= max_prefetch {
            break;
        }

        // Skip external URLs.
        if url.starts_with("http://") || url.starts_with("https://") {
            continue;
        }

        // Skip anchor-only links.
        if url.starts_with('#') {
            continue;
        }

        // Skip empty URLs.
        if url.is_empty() {
            continue;
        }

        // Normalize for self-reference check.
        let normalized = normalize_url(&url);
        if normalized == self_url {
            continue;
        }

        // Skip excluded patterns.
        if patterns.iter().any(|p| p.matches(&url)) {
            continue;
        }

        // Deduplicate.
        if !seen.insert(url.clone()) {
            continue;
        }

        result.push(url);
    }

    result
}

/// Compute the URL that represents "this page" for self-reference detection.
///
/// When fragments are enabled, the self URL is the fragment path
/// (e.g., `/_fragments/about.html`). When disabled, it is the page URL itself.
fn compute_self_url(current_url: &str, fragment_dir: &str, fragments_enabled: bool) -> String {
    let normalized = normalize_url(current_url);
    if fragments_enabled {
        let clean = normalized.trim_start_matches('/');
        format!("/{}/{}", fragment_dir, clean)
    } else {
        normalized
    }
}

/// Normalize a URL for comparison: treat trailing `/` as `/index.html`.
fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url == "/" || url.is_empty() {
        return "/index.html".to_string();
    }
    if url.ends_with('/') {
        return format!("{}index.html", url);
    }
    url.to_string()
}

/// Collect all `hx-get` attribute values from HTML elements.
fn collect_hx_get_urls(html: &str) -> Vec<String> {
    let urls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let urls_clone = urls.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("*[hx-get]", move |el| {
                if let Some(val) = el.get_attribute("hx-get") {
                    urls_clone.borrow_mut().push(val);
                }
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = urls.borrow().clone();
    result
}

/// Collect `href` attribute values from `<a>` elements, excluding
/// links with `target="_blank"`.
fn collect_href_urls(html: &str) -> Vec<String> {
    let urls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let urls_clone = urls.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("a[href]", move |el| {
                // Skip target="_blank" links.
                if let Some(target) = el.get_attribute("target") {
                    if target == "_blank" {
                        return Ok(());
                    }
                }
                if let Some(val) = el.get_attribute("href") {
                    urls_clone.borrow_mut().push(val);
                }
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = urls.borrow().clone();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_hx_get_urls() {
        let html = r##"
            <a href="/about.html" hx-get="/_fragments/about.html" hx-target="#content">About</a>
            <a href="/blog.html" hx-get="/_fragments/blog.html" hx-target="#content">Blog</a>
        "##;
        let urls = collect_hx_get_urls(html);
        assert_eq!(urls, vec!["/_fragments/about.html", "/_fragments/blog.html"]);
    }

    #[test]
    fn test_collect_href_urls_no_fragments() {
        let html = r#"
            <a href="/about.html">About</a>
            <a href="/blog.html">Blog</a>
            <a href="https://external.com" target="_blank">Ext</a>
        "#;
        let urls = collect_href_urls(html);
        assert_eq!(urls, vec!["/about.html", "/blog.html"]);
    }

    #[test]
    fn test_excludes_external_urls() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="https://evil.com/steal">Bad</a>
        "#;
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", true, 10, &[]);
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_self_reference() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/index.html">Home</a>
        "#;
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", true, 10, &[]);
        // Self-reference to /index.html (-> /_fragments/index.html) should be excluded.
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_self_reference_index() {
        // Test that "/" and "/index.html" are recognized as the same page.
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/index.html">Home</a>
        "#;
        // current_url is "/" -- should still match /_fragments/index.html
        let result = collect_prefetch_urls(html, "/", "_fragments", true, 10, &[]);
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_pattern_match() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/archive/2023.html">Archive</a>
        "#;
        let result = collect_prefetch_urls(
            html,
            "/index.html",
            "_fragments",
            true,
            10,
            &["**/archive/**".to_string()],
        );
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_max_prefetch_limit() {
        let html = r#"
            <a hx-get="/_fragments/a.html">A</a>
            <a hx-get="/_fragments/b.html">B</a>
            <a hx-get="/_fragments/c.html">C</a>
            <a hx-get="/_fragments/d.html">D</a>
        "#;
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", true, 2, &[]);
        assert_eq!(result.len(), 2);
        assert_eq!(result, vec!["/_fragments/a.html", "/_fragments/b.html"]);
    }

    #[test]
    fn test_deduplicates_urls() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/about.html">About Again</a>
            <a hx-get="/_fragments/blog.html">Blog</a>
        "#;
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", true, 10, &[]);
        assert_eq!(
            result,
            vec!["/_fragments/about.html", "/_fragments/blog.html"]
        );
    }

    #[test]
    fn test_excludes_anchor_only() {
        let html = r##"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="#section">Section</a>
        "##;
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", true, 10, &[]);
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_target_blank() {
        let html = r#"
            <a href="/about.html">About</a>
            <a href="/ext.html" target="_blank">External</a>
        "#;
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", false, 10, &[]);
        assert_eq!(result, vec!["/about.html"]);
    }

    #[test]
    fn test_max_prefetch_zero() {
        let html = r#"<a hx-get="/_fragments/about.html">About</a>"#;
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", true, 0, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_normalize_url_trailing_slash() {
        assert_eq!(normalize_url("/posts/"), "/posts/index.html");
        assert_eq!(normalize_url("/"), "/index.html");
        assert_eq!(normalize_url("/about.html"), "/about.html");
    }

    #[test]
    fn test_compute_self_url_fragments_enabled() {
        let result = compute_self_url("/about.html", "_fragments", true);
        assert_eq!(result, "/_fragments/about.html");
    }

    #[test]
    fn test_compute_self_url_fragments_disabled() {
        let result = compute_self_url("/about.html", "_fragments", false);
        assert_eq!(result, "/about.html");
    }

    #[test]
    fn test_glob_pattern_error_handled() {
        // An invalid glob pattern should not crash -- it should be skipped.
        let html = r#"<a hx-get="/_fragments/about.html">About</a>"#;
        let result = collect_prefetch_urls(
            html,
            "/index.html",
            "_fragments",
            true,
            10,
            &["[invalid".to_string()],
        );
        // The invalid pattern is skipped; the URL is still collected.
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_no_links_in_html() {
        let html = "<html><head></head><body><p>No links here</p></body></html>";
        let result = collect_prefetch_urls(html, "/index.html", "_fragments", true, 10, &[]);
        assert!(result.is_empty());
    }
}
