//! HTML rewriting: replace <link> and <script> tags with bundle references.
//!
//! Uses `lol_html` for streaming HTML rewriting. Handles:
//! - Replacing first bundled `<link>`/`<script>` with bundle path
//! - Removing subsequent bundled `<link>`/`<script>` tags
//! - Rewriting critical CSS preload pattern (`<link rel="preload" as="style">`)
//! - Rewriting `<noscript>` fallback links (via text replacement, since
//!   lol_html treats noscript content as raw text)

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eyre::{Result, WrapErr};

/// Rewrite all HTML files in dist/ to reference bundled CSS/JS files.
///
/// For each HTML file:
/// 1. Replace the FIRST `<link>` / `<script>` that references a bundled
///    file with a reference to the bundle.
/// 2. Remove subsequent `<link>` / `<script>` tags that reference other
///    bundled files (they are now in the bundle).
/// 3. Handle the critical CSS preload pattern: rewrite both the preload
///    `<link>` and the `<noscript>` fallback `<link>`.
pub fn rewrite_html_for_bundles(
    html_files: &[PathBuf],
    _dist_dir: &Path,
    css_bundle_href: Option<&str>,
    js_bundle_src: Option<&str>,
    css_hrefs: &[String],
    js_srcs: &[String],
) -> Result<()> {
    let css_set: HashSet<&str> = css_hrefs.iter().map(|s| s.as_str()).collect();
    let js_set: HashSet<&str> = js_srcs.iter().map(|s| s.as_str()).collect();

    for html_path in html_files {
        let html_content = std::fs::read_to_string(html_path)
            .wrap_err_with(|| format!(
                "Bundling rewrite: failed to read '{}'", html_path.display()
            ))?;

        let rewritten = rewrite_single_html(
            &html_content,
            css_bundle_href,
            js_bundle_src,
            &css_set,
            &js_set,
        ).wrap_err_with(|| format!(
            "Bundling rewrite: failed to rewrite '{}'", html_path.display()
        ))?;

        std::fs::write(html_path, rewritten)
            .wrap_err_with(|| format!(
                "Bundling rewrite: failed to write '{}'", html_path.display()
            ))?;
    }

    Ok(())
}

/// Rewrite a single HTML string to reference bundled CSS/JS files.
///
/// All four handlers (stylesheet, preload, noscript, script) are always
/// registered. Handlers for disabled bundle types (css_bundle_href=None
/// or js_bundle_src=None) are no-ops because the bundled set will be
/// empty and no elements will match.
fn rewrite_single_html(
    html: &str,
    css_bundle_href: Option<&str>,
    js_bundle_src: Option<&str>,
    css_set: &HashSet<&str>,
    js_set: &HashSet<&str>,
) -> Result<String> {
    let css_injected = Rc::new(RefCell::new(false));
    let js_injected = Rc::new(RefCell::new(false));

    let css_injected_link = css_injected.clone();
    let css_injected_preload = css_injected.clone();
    let js_injected_clone = js_injected.clone();

    let css_set_link: Rc<HashSet<String>> = Rc::new(css_set.iter().map(|s| s.to_string()).collect());
    let css_set_preload = css_set_link.clone();
    let js_set_rc: Rc<HashSet<String>> = Rc::new(js_set.iter().map(|s| s.to_string()).collect());

    let css_bundle = css_bundle_href.map(|s| s.to_string());
    let css_bundle_link = css_bundle.clone();
    let css_bundle_preload = css_bundle.clone();
    let js_bundle = js_bundle_src.map(|s| s.to_string());

    let noscript_css_bundle = css_bundle.clone();
    let noscript_css_set: Rc<HashSet<String>> = css_set_link.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                // CSS <link rel="stylesheet"> handler.
                lol_html::element!("link[rel='stylesheet']", move |el| {
                    let href = match el.get_attribute("href") {
                        Some(h) => h,
                        None => return Ok(()),
                    };

                    if !css_set_link.contains(&href) {
                        return Ok(());
                    }

                    if !*css_injected_link.borrow() {
                        if let Some(ref bundle) = css_bundle_link {
                            el.set_attribute("href", bundle)?;
                        }
                        *css_injected_link.borrow_mut() = true;
                    } else {
                        el.remove();
                    }

                    Ok(())
                }),
                // CSS <link rel="preload" as="style"> handler (critical CSS pattern).
                lol_html::element!("link[rel='preload'][as='style']", move |el| {
                    let href = match el.get_attribute("href") {
                        Some(h) => h,
                        None => return Ok(()),
                    };

                    if !css_set_preload.contains(&href) {
                        return Ok(());
                    }

                    if !*css_injected_preload.borrow() {
                        if let Some(ref bundle) = css_bundle_preload {
                            el.set_attribute("href", bundle)?;
                        }
                        *css_injected_preload.borrow_mut() = true;
                    } else {
                        el.remove();
                    }

                    Ok(())
                }),
                // Noscript text handler: rewrite <link> hrefs inside <noscript>.
                // lol_html treats noscript content as raw text, so we use string
                // replacement rather than element matching.
                lol_html::text!("noscript", move |text| {
                    let original = text.as_str().to_string();

                    if let Some(ref bundle) = noscript_css_bundle {
                        let mut modified = original.clone();
                        let mut first = true;

                        for href in noscript_css_set.iter() {
                            if modified.contains(href.as_str()) {
                                if first {
                                    modified = modified.replace(href.as_str(), bundle);
                                    first = false;
                                } else {
                                    let pattern = format!(
                                        r#"<link rel="stylesheet" href="{}">"#,
                                        href
                                    );
                                    modified = modified.replace(&pattern, "");
                                }
                            }
                        }

                        if modified != original {
                            text.replace(&modified, lol_html::html_content::ContentType::Html);
                        }
                    }

                    Ok(())
                }),
                // JS <script> handler.
                lol_html::element!("script", move |el| {
                    let src = match el.get_attribute("src") {
                        Some(s) if !s.is_empty() => s,
                        _ => return Ok(()),
                    };

                    // Skip defer, async, type="module".
                    if el.get_attribute("defer").is_some()
                        || el.get_attribute("async").is_some()
                    {
                        return Ok(());
                    }
                    if el.get_attribute("type").as_deref() == Some("module") {
                        return Ok(());
                    }

                    if !js_set_rc.contains(&src) {
                        return Ok(());
                    }

                    if !*js_injected_clone.borrow() {
                        if let Some(ref bundle) = js_bundle {
                            el.set_attribute("src", bundle)?;
                        }
                        *js_injected_clone.borrow_mut() = true;
                    } else {
                        el.remove();
                    }

                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| eyre::eyre!("lol_html rewrite error: {e}"))?;

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn css_set<'a>(hrefs: &[&'a str]) -> HashSet<&'a str> {
        hrefs.iter().copied().collect()
    }

    fn js_set<'a>(srcs: &[&'a str]) -> HashSet<&'a str> {
        srcs.iter().copied().collect()
    }

    #[test]
    fn test_rewrite_css_single_link() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#;
        let css = css_set(&["/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        assert!(result.contains(r#"href="/css/bundle.css""#));
        assert!(!result.contains(r#"href="/css/style.css""#));
    }

    #[test]
    fn test_rewrite_css_multiple_links() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#;
        let css = css_set(&["/css/reset.css", "/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        // First link rewritten to bundle, second removed.
        assert!(result.contains(r#"href="/css/bundle.css""#));
        assert!(!result.contains(r#"href="/css/reset.css""#));
        assert!(!result.contains(r#"href="/css/style.css""#));
        // Only one link tag should remain.
        assert_eq!(result.matches(r#"rel="stylesheet""#).count(), 1);
    }

    #[test]
    fn test_rewrite_css_preload_pattern() {
        let html = r#"<html><head>
            <style>.hero { color: red; }</style>
            <link rel="preload" href="/css/style.css" as="style" onload="this.onload=null;this.rel='stylesheet'">
            <noscript><link rel="stylesheet" href="/css/style.css"></noscript>
        </head><body></body></html>"#;
        let css = css_set(&["/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        // Preload link should be rewritten.
        assert!(result.contains(r#"href="/css/bundle.css""#));
        assert!(!result.contains(r#"href="/css/style.css""#));
        // Inline style should be untouched.
        assert!(result.contains("<style>.hero { color: red; }</style>"));
    }

    #[test]
    fn test_rewrite_css_noscript_pattern() {
        let html = r#"<html><head>
            <noscript><link rel="stylesheet" href="/css/style.css"></noscript>
        </head><body></body></html>"#;
        let css = css_set(&["/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        // Noscript link should be rewritten via text replacement.
        assert!(result.contains("/css/bundle.css"));
        assert!(!result.contains("/css/style.css"));
    }

    #[test]
    fn test_rewrite_js_single_script() {
        let html = r#"<html><body><script src="/js/app.js"></script></body></html>"#;
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        assert!(result.contains(r#"src="/js/bundle.js""#));
        assert!(!result.contains(r#"src="/js/app.js""#));
    }

    #[test]
    fn test_rewrite_js_multiple_scripts() {
        let html = r#"<html><body>
            <script src="/js/utils.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let js = js_set(&["/js/utils.js", "/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        // First script rewritten, second removed.
        assert!(result.contains(r#"src="/js/bundle.js""#));
        assert!(!result.contains(r#"src="/js/utils.js""#));
        assert!(!result.contains(r#"src="/js/app.js""#));
    }

    #[test]
    fn test_rewrite_preserves_external() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="https://cdn.example.com/lib.css">
            <link rel="stylesheet" href="/css/local.css">
        </head><body>
            <script src="https://cdn.example.com/lib.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let css = css_set(&["/css/local.css"]);
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), Some("/js/bundle.js"), &css, &js).unwrap();

        // External links should be untouched.
        assert!(result.contains("https://cdn.example.com/lib.css"));
        assert!(result.contains("https://cdn.example.com/lib.js"));
    }

    #[test]
    fn test_rewrite_preserves_module() {
        let html = r#"<html><body>
            <script type="module" src="/js/mod.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        // Module script untouched.
        assert!(result.contains(r#"type="module" src="/js/mod.js""#));
        // Regular script rewritten.
        assert!(result.contains(r#"src="/js/bundle.js""#));
    }

    #[test]
    fn test_rewrite_preserves_async_defer() {
        let html = r#"<html><body>
            <script defer src="/js/deferred.js"></script>
            <script async src="/js/async.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        // Defer and async scripts untouched.
        assert!(result.contains(r#"src="/js/deferred.js""#));
        assert!(result.contains(r#"src="/js/async.js""#));
        // Regular script rewritten.
        assert!(result.contains(r#"src="/js/bundle.js""#));
    }

    #[test]
    fn test_rewrite_no_bundled_refs() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/other.css"></head><body><script src="/js/other.js"></script></body></html>"#;
        let result = rewrite_single_html(html, Some("/css/bundle.css"), Some("/js/bundle.js"), &HashSet::new(), &HashSet::new()).unwrap();

        // No bundled refs -> no changes.
        assert!(result.contains(r#"href="/css/other.css""#));
        assert!(result.contains(r#"src="/js/other.js""#));
    }
}
