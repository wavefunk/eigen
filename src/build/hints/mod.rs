//! Preload/prefetch resource hints: inject `<link rel="preload">` for hero
//! images and `<link rel="prefetch">` for navigation links into `<head>`.
//!
//! This module provides `inject_resource_hints`, the main entry point called
//! from the build pipeline. It:
//! 1. Resolves the hero image (frontmatter or auto-detection).
//! 2. Builds a preload hint with responsive variant info.
//! 3. Collects navigation link URLs for prefetching.
//! 4. Injects all hints as `<link>` tags early in `<head>`.

pub mod prefetch;
pub mod preload;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::config::HintsConfig;

/// A resource hint to be injected into the page's `<head>`.
#[derive(Debug, Clone, PartialEq)]
enum ResourceHint {
    /// `<link rel="preload">` for the hero image.
    HeroPreload(preload::HeroPreload),
    /// `<link rel="prefetch">` for a likely next navigation.
    NavigationPrefetch {
        /// The fragment or page URL to prefetch.
        href: String,
        /// Whether this targets a full document (vs a fragment).
        is_document: bool,
    },
}

/// Inject preload and prefetch resource hints into HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
pub fn inject_resource_hints(
    html: &str,
    config: &HintsConfig,
    dist_dir: &Path,
    hero_image: Option<&str>,
    current_url: &str,
    fragment_dir: &str,
    fragments_enabled: bool,
) -> String {
    if !config.enabled {
        return html.to_string();
    }

    let mut hints: Vec<ResourceHint> = Vec::new();

    // 1. Hero image preload.
    if let Some(hero_src) =
        preload::resolve_hero_image(html, hero_image, config.auto_detect_hero)
    {
        // Check if there's already a preload hint for this image.
        if !has_existing_preload(html, &hero_src) {
            let hero_preload = preload::build_hero_preload(
                &hero_src,
                html,
                dist_dir,
                &config.hero_image_sizes,
            );
            hints.push(ResourceHint::HeroPreload(hero_preload));
        }
    }

    // 2. Navigation prefetch.
    if config.prefetch_links {
        let urls = prefetch::collect_prefetch_urls(
            html,
            current_url,
            fragment_dir,
            fragments_enabled,
            config.max_prefetch,
            &config.exclude_prefetch,
        );
        for url in urls {
            hints.push(ResourceHint::NavigationPrefetch {
                href: url,
                is_document: !fragments_enabled,
            });
        }
    }

    // 3. Generate and inject HTML.
    if hints.is_empty() {
        return html.to_string();
    }

    let hint_html = hints_to_html(&hints);

    match inject_into_head(html, &hint_html) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("Failed to inject resource hints: {}", e);
            html.to_string()
        }
    }
}

/// Check if the HTML already has a `<link rel="preload">` for the given href.
fn has_existing_preload(html: &str, href: &str) -> bool {
    let found: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_clone = found.clone();
    let href_owned = href.to_string();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!(
                "link[rel='preload']",
                move |el| {
                    if let Some(existing_href) = el.get_attribute("href") {
                        if existing_href == href_owned {
                            *found_clone.borrow_mut() = true;
                        }
                    }
                    Ok(())
                }
            )],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    *found.borrow()
}

/// Serialize a list of ResourceHint values into HTML `<link>` tag strings.
fn hints_to_html(hints: &[ResourceHint]) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match hint {
            ResourceHint::HeroPreload(p) => {
                let _ = write!(out, r#"<link rel="preload" as="image" href="{}""#, p.href);
                if let Some(ref srcset) = p.imagesrcset {
                    let _ = write!(out, r#" imagesrcset="{}""#, srcset);
                }
                if let Some(ref sizes) = p.imagesizes {
                    let _ = write!(out, r#" imagesizes="{}""#, sizes);
                }
                let _ = write!(out, r#" type="{}">"#, p.mime_type);
            }
            ResourceHint::NavigationPrefetch { href, is_document } => {
                let _ = write!(out, r#"<link rel="prefetch" href="{}""#, href);
                if *is_document {
                    out.push_str(r#" as="document""#);
                }
                out.push('>');
            }
        }
    }

    out
}

/// Inject hint `<link>` tags into the HTML `<head>` as early children.
///
/// Uses lol_html to find the opening `<head>` tag and prepend the
/// hint tags. Returns the original HTML if no `<head>` is found.
fn inject_into_head(html: &str, hint_html: &str) -> Result<String, String> {
    let hint_html_owned = hint_html.to_string();

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("head", move |el| {
                el.prepend(
                    &format!("\n{}\n", hint_html_owned),
                    lol_html::html_content::ContentType::Html,
                );
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| format!("lol_html rewrite error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HintsConfig {
        HintsConfig {
            enabled: true,
            auto_detect_hero: true,
            prefetch_links: true,
            max_prefetch: 5,
            hero_image_sizes: "100vw".to_string(),
            exclude_prefetch: Vec::new(),
        }
    }

    #[test]
    fn test_inject_hints_disabled() {
        let html = r#"<html><head><title>T</title></head><body><img src="/hero.jpg" alt="H"></body></html>"#;
        let config = HintsConfig {
            enabled: false,
            ..test_config()
        };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            true,
        );
        assert_eq!(result, html);
    }

    #[test]
    fn test_inject_hero_from_frontmatter() {
        let html =
            r#"<html><head><title>T</title></head><body><p>No img</p></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            Some("/assets/hero.jpg"),
            "/index.html",
            "_fragments",
            true,
        );
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains(r#"href="/assets/hero.jpg""#));
        assert!(result.contains(r#"as="image""#));
    }

    #[test]
    fn test_inject_hero_auto_detect() {
        let html = r#"<html><head><title>T</title></head><body><img src="/assets/banner.jpg" alt="Banner"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            true,
        );
        assert!(result.contains(r#"href="/assets/banner.jpg""#));
        assert!(result.contains(r#"rel="preload""#));
    }

    #[test]
    fn test_auto_detect_skips_lazy() {
        let html = r#"<html><head></head><body><img src="/lazy.jpg" loading="lazy" alt="L"><img src="/hero.jpg" alt="H"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            true,
        );
        assert!(result.contains(r#"href="/hero.jpg""#));
        assert!(!result.contains(r#"href="/lazy.jpg""#));
    }

    #[test]
    fn test_inject_prefetch_links() {
        let html = r##"<html><head></head><body>
            <a href="/about.html" hx-get="/_fragments/about.html" hx-target="#content">About</a>
            <a href="/blog.html" hx-get="/_fragments/blog.html" hx-target="#content">Blog</a>
        </body></html>"##;
        let config = HintsConfig {
            auto_detect_hero: false,
            ..test_config()
        };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            true,
        );
        assert!(result.contains(r#"rel="prefetch" href="/_fragments/about.html""#));
        assert!(result.contains(r#"rel="prefetch" href="/_fragments/blog.html""#));
    }

    #[test]
    fn test_inject_combined() {
        let html = r#"<html><head></head><body>
            <img src="/hero.jpg" alt="Hero">
            <a hx-get="/_fragments/about.html">About</a>
        </body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            true,
        );
        // Both preload and prefetch should be present.
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains(r#"rel="prefetch""#));
    }

    #[test]
    fn test_no_head_element() {
        let html = r#"<div>No head element here</div>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        // Should not crash; hint_html has nowhere to go, but inject_into_head
        // will still return the HTML (lol_html won't match the `head` selector).
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            Some("/hero.jpg"),
            "/index.html",
            "_fragments",
            true,
        );
        // The function should return something (possibly unchanged if injection
        // had no effect, or with hints if lol_html just prepends to nothing).
        assert!(!result.is_empty());
    }

    #[test]
    fn test_skip_existing_preload() {
        let html = r#"<html><head>
            <link rel="preload" as="image" href="/hero.jpg">
        </head><body><img src="/hero.jpg" alt="Hero"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            Some("/hero.jpg"),
            "/index.html",
            "_fragments",
            true,
        );
        // Should not duplicate the preload.
        let preload_count = result.matches(r#"rel="preload""#).count();
        assert_eq!(preload_count, 1);
    }

    #[test]
    fn test_hints_injected_early_in_head() {
        let html = r#"<html><head><meta charset="UTF-8"><title>T</title></head><body><img src="/hero.jpg" alt="H"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            Some("/hero.jpg"),
            "/index.html",
            "_fragments",
            true,
        );
        // Hints should appear before the <meta> tag.
        let head_pos = result.find("<head>").unwrap();
        let preload_pos = result.find(r#"rel="preload""#).unwrap();
        let meta_pos = result.find("<meta").unwrap();
        assert!(preload_pos > head_pos);
        assert!(preload_pos < meta_pos);
    }

    #[test]
    fn test_prefetch_is_document_when_no_fragments() {
        let html = r#"<html><head></head><body>
            <a href="/about.html">About</a>
        </body></html>"#;
        let config = HintsConfig {
            auto_detect_hero: false,
            ..test_config()
        };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            false,
        );
        assert!(result.contains(r#"as="document""#));
    }

    #[test]
    fn test_prefetch_no_as_document_with_fragments() {
        let html = r#"<html><head></head><body>
            <a hx-get="/_fragments/about.html">About</a>
        </body></html>"#;
        let config = HintsConfig {
            auto_detect_hero: false,
            ..test_config()
        };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            true,
        );
        assert!(!result.contains(r#"as="document""#));
    }

    #[test]
    fn test_hints_to_html_preload() {
        let hints = vec![ResourceHint::HeroPreload(preload::HeroPreload {
            href: "/hero.jpg".to_string(),
            mime_type: "image/jpeg".to_string(),
            imagesrcset: None,
            imagesizes: None,
        })];
        let html = hints_to_html(&hints);
        assert_eq!(
            html,
            r#"<link rel="preload" as="image" href="/hero.jpg" type="image/jpeg">"#
        );
    }

    #[test]
    fn test_hints_to_html_preload_responsive() {
        let hints = vec![ResourceHint::HeroPreload(preload::HeroPreload {
            href: "/hero.jpg".to_string(),
            mime_type: "image/avif".to_string(),
            imagesrcset: Some(
                "/hero-480w.avif 480w, /hero-768w.avif 768w".to_string(),
            ),
            imagesizes: Some("100vw".to_string()),
        })];
        let html = hints_to_html(&hints);
        assert!(html.contains(
            r#"imagesrcset="/hero-480w.avif 480w, /hero-768w.avif 768w""#
        ));
        assert!(html.contains(r#"imagesizes="100vw""#));
        assert!(html.contains(r#"type="image/avif""#));
    }

    #[test]
    fn test_hints_to_html_prefetch() {
        let hints = vec![ResourceHint::NavigationPrefetch {
            href: "/_fragments/about.html".to_string(),
            is_document: false,
        }];
        let html = hints_to_html(&hints);
        assert_eq!(
            html,
            r#"<link rel="prefetch" href="/_fragments/about.html">"#
        );
    }

    #[test]
    fn test_hints_to_html_prefetch_document() {
        let hints = vec![ResourceHint::NavigationPrefetch {
            href: "/about.html".to_string(),
            is_document: true,
        }];
        let html = hints_to_html(&hints);
        assert_eq!(
            html,
            r#"<link rel="prefetch" href="/about.html" as="document">"#
        );
    }

    #[test]
    fn test_inject_into_head() {
        let html = r#"<html><head><title>T</title></head></html>"#;
        let hint =
            r#"<link rel="preload" as="image" href="/hero.jpg" type="image/jpeg">"#;
        let result = inject_into_head(html, hint).unwrap();
        assert!(result.contains(hint));
        // Hint should be before <title>
        let hint_pos = result.find(r#"rel="preload""#).unwrap();
        let title_pos = result.find("<title>").unwrap();
        assert!(hint_pos < title_pos);
    }

    #[test]
    fn test_has_existing_preload() {
        let html =
            r#"<head><link rel="preload" as="image" href="/hero.jpg"></head>"#;
        assert!(has_existing_preload(html, "/hero.jpg"));
        assert!(!has_existing_preload(html, "/other.jpg"));
    }

    #[test]
    fn test_hero_from_picture_element() {
        let html = r#"<html><head></head><body>
            <picture>
                <source srcset="/hero-480w.avif 480w, /hero-768w.avif 768w" type="image/avif">
                <img src="/hero.jpg" alt="Hero">
            </picture>
        </body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            None,
            "/index.html",
            "_fragments",
            true,
        );
        assert!(result.contains(r#"imagesrcset="#));
        assert!(result.contains(r#"type="image/avif""#));
    }
}
