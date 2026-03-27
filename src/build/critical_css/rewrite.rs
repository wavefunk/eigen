//! HTML rewriting for critical CSS: inject `<style>` blocks and
//! rewrite `<link>` tags for deferred loading.
//!
//! Uses `lol_html` for streaming HTML rewriting. This is the same library
//! used by the asset localization module.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Rewrite HTML to inline critical CSS and defer full stylesheets.
///
/// 1. Injects a `<style>` block containing `critical_css` before the first
///    processed `<link>` tag.
/// 2. Rewrites each `<link>` in `processed_hrefs` to load asynchronously
///    (if `preload_full` is true) or removes it entirely (if false).
/// 3. Leaves `<link>` tags not in `processed_hrefs` unchanged.
pub fn rewrite_html(
    html: &str,
    critical_css: &str,
    processed_hrefs: &[String],
    preload_full: bool,
) -> Result<String, String> {
    let style_injected = Rc::new(RefCell::new(false));
    let style_injected_clone = style_injected.clone();

    let critical_css_owned = critical_css.to_string();
    let processed_set: HashSet<String> = processed_hrefs.iter().cloned().collect();
    let processed_set = Rc::new(processed_set);
    let processed_set_clone = processed_set.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("link[rel='stylesheet']", move |el| {
                    let href = match el.get_attribute("href") {
                        Some(h) => h,
                        None => return Ok(()),
                    };

                    // Only process links that we have critical CSS for.
                    if !processed_set_clone.contains(&href) {
                        return Ok(());
                    }

                    // Inject <style> block before the first processed link.
                    if !*style_injected_clone.borrow() {
                        let style_tag = format!("<style>{}</style>", critical_css_owned);
                        el.before(&style_tag, lol_html::html_content::ContentType::Html);
                        *style_injected_clone.borrow_mut() = true;
                    }

                    if preload_full {
                        // Rewrite to preload pattern with noscript fallback.
                        let preload_html = format!(
                            concat!(
                                r#"<link rel="preload" href="{}" as="style" "#,
                                r#"onload="this.onload=null;this.rel='stylesheet'">"#,
                                r#"<noscript><link rel="stylesheet" href="{}"></noscript>"#,
                            ),
                            href, href
                        );
                        el.replace(&preload_html, lol_html::html_content::ContentType::Html);
                    } else {
                        // Remove the link entirely (tree-shaking mode).
                        el.remove();
                    }

                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| format!("lol_html rewrite error: {e}"))?;

    Ok(output)
}

/// Extract local stylesheet hrefs from HTML.
///
/// Returns hrefs of `<link rel="stylesheet">` tags that:
/// - Have a non-empty `href` attribute
/// - Are not external (http/https)
/// - Do not have a `media` attribute (those are already non-blocking)
/// - Do not match any exclude patterns
pub fn extract_stylesheet_hrefs(html: &str, exclude: &[String]) -> Vec<String> {
    let hrefs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let hrefs_clone = hrefs.clone();
    let exclude_owned: Vec<String> = exclude.to_vec();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("link[rel='stylesheet']", move |el| {
                    // Skip if has media attribute (already non-blocking).
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
                    for pattern in &exclude_owned {
                        if let Ok(pat) = glob::Pattern::new(pattern) {
                            if pat.matches(&href) {
                                return Ok(());
                            }
                        }
                    }

                    hrefs_clone.borrow_mut().push(href);
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = hrefs.borrow().clone();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_stylesheet_hrefs tests ---

    #[test]
    fn test_extract_single_link() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert_eq!(hrefs, vec!["/css/style.css"]);
    }

    #[test]
    fn test_extract_multiple_links() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/main.css">
        </head></html>"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert_eq!(hrefs, vec!["/css/reset.css", "/css/main.css"]);
    }

    #[test]
    fn test_extract_skips_external() {
        let html = r#"<link rel="stylesheet" href="https://cdn.example.com/style.css">"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert!(hrefs.is_empty());
    }

    #[test]
    fn test_extract_skips_media_attr() {
        let html = r#"<link rel="stylesheet" href="/css/print.css" media="print">"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert!(hrefs.is_empty());
    }

    #[test]
    fn test_extract_skips_excluded_pattern() {
        let html = r#"<link rel="stylesheet" href="/css/vendor/bootstrap.css">"#;
        let hrefs = extract_stylesheet_hrefs(html, &["**/vendor/**".to_string()]);
        assert!(hrefs.is_empty());
    }

    #[test]
    fn test_extract_preserves_non_stylesheet_links() {
        let html = r#"<link rel="icon" href="/favicon.ico"><link rel="stylesheet" href="/css/style.css">"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert_eq!(hrefs, vec!["/css/style.css"]);
    }

    // --- rewrite_html tests ---

    #[test]
    fn test_rewrite_single_link() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><p>Hi</p></body></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            true,
        ).unwrap();

        assert!(result.contains("<style>body { color: red; }</style>"));
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains(r#"as="style""#));
        assert!(result.contains("onload="));
        assert!(result.contains("<noscript>"));
        // The only remaining stylesheet link should be inside <noscript>.
        // Count occurrences: rel="stylesheet" should appear exactly once (in noscript).
        assert_eq!(result.matches(r#"rel="stylesheet""#).count(), 1);
    }

    #[test]
    fn test_rewrite_no_preload() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            false,
        ).unwrap();

        assert!(result.contains("<style>body { color: red; }</style>"));
        // No preload link, no noscript fallback.
        assert!(!result.contains("preload"));
        assert!(!result.contains("<noscript>"));
    }

    #[test]
    fn test_rewrite_multiple_links() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/main.css">
        </head></html>"#;
        let result = rewrite_html(
            html,
            ".a { color: red; } .b { color: blue; }",
            &["/css/reset.css".to_string(), "/css/main.css".to_string()],
            true,
        ).unwrap();

        // Should have exactly one <style> block.
        assert_eq!(result.matches("<style>").count(), 1);
        // Both links should be rewritten to preload.
        assert_eq!(result.matches(r#"rel="preload""#).count(), 2);
    }

    #[test]
    fn test_rewrite_preserves_other_links() {
        let html = r#"<html><head>
            <link rel="icon" href="/favicon.ico">
            <link rel="stylesheet" href="/css/style.css">
        </head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            true,
        ).unwrap();

        // Favicon link should be untouched.
        assert!(result.contains(r#"<link rel="icon" href="/favicon.ico">"#));
    }

    #[test]
    fn test_rewrite_noscript_fallback() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            true,
        ).unwrap();

        assert!(result.contains(r#"<noscript><link rel="stylesheet" href="/css/style.css"></noscript>"#));
    }

    #[test]
    fn test_rewrite_external_link_untouched() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="https://cdn.example.com/lib.css">
            <link rel="stylesheet" href="/css/local.css">
        </head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/local.css".to_string()],
            true,
        ).unwrap();

        // External link should remain unchanged.
        assert!(result.contains(r#"href="https://cdn.example.com/lib.css""#));
        // Local link should be rewritten.
        assert!(result.contains(r#"rel="preload""#));
    }
}
