//! Open Graph / Twitter Card meta tag injection.
//!
//! Auto-injects `<meta property="og:*">`, `<meta name="twitter:*">`, and
//! `<link rel="canonical">` tags into `<head>` during the build pipeline.
//!
//! Two public functions:
//! - `inject_seo_tags`: the pipeline step (after hints, before minify).
//! - `resolve_seo_expressions`: template expression evaluation for dynamic
//!   page SEO fields (called earlier in the render flow).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::config::SiteMeta;
use crate::frontmatter::SeoMeta;

/// Fully resolved SEO metadata ready for injection into HTML.
///
/// All template expressions have been evaluated. All fallbacks have
/// been applied. URLs are absolute.
struct ResolvedSeo {
    /// Page title. Never empty -- falls back chain:
    /// frontmatter.seo.title > site.seo.title > site.name.
    title: String,
    /// Meta description. None if no description is available at any level.
    description: Option<String>,
    /// Absolute URL to the share image. None if no image is available.
    image: Option<String>,
    /// Open Graph type (e.g. "website", "article").
    og_type: String,
    /// Twitter card type. Forced to "summary" when image is None.
    twitter_card: String,
    /// Twitter @handle for the site. None if not configured.
    twitter_site: Option<String>,
    /// Canonical URL (absolute). Always present.
    canonical_url: String,
    /// The site name for `og:site_name`. Always `site.name`.
    site_name: String,
}

/// Escape characters that are unsafe in HTML attribute values.
///
/// Only the four characters that matter for double-quoted attributes
/// are escaped. This is intentionally minimal -- we are generating
/// well-formed `content="..."` attributes, not arbitrary HTML.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Resolve a URL to absolute form using the site's base_url.
///
/// - Already absolute (`http://` or `https://`): returned as-is.
/// - Site-relative (starts with `/`): `base_url` is prepended.
/// - Trailing-slash normalization: avoids `https://example.com//path`.
fn make_absolute_url(url: &str, base_url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    let base = base_url.trim_end_matches('/');
    if url.starts_with('/') {
        let mut result = String::with_capacity(base.len() + url.len());
        result.push_str(base);
        result.push_str(url);
        result
    } else {
        url.to_string()
    }
}

/// Merge frontmatter SEO fields with site-level defaults to produce
/// fully resolved values.
///
/// Priority: frontmatter > site.seo > auto-derived defaults.
fn resolve_seo(
    frontmatter_seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> ResolvedSeo {
    // Title: frontmatter > site.seo > site.name
    let title = frontmatter_seo
        .title
        .clone()
        .or_else(|| site_config.seo.title.clone())
        .unwrap_or_else(|| site_config.name.clone());

    // Description: frontmatter > site.seo (None if neither set)
    let description = frontmatter_seo
        .description
        .clone()
        .or_else(|| site_config.seo.description.clone());

    // Image: frontmatter > site.seo, resolved to absolute URL
    let image = frontmatter_seo
        .image
        .clone()
        .or_else(|| site_config.seo.image.clone())
        .map(|img| make_absolute_url(&img, &site_config.base_url));

    // og:type: frontmatter > site.seo > "website"
    let og_type = frontmatter_seo
        .og_type
        .clone()
        .unwrap_or_else(|| site_config.seo.og_type.clone());

    // twitter:card: frontmatter > site.seo > "summary_large_image",
    // but forced to "summary" when no image is available.
    let twitter_card = if image.is_none() {
        "summary".to_string()
    } else {
        frontmatter_seo
            .twitter_card
            .clone()
            .unwrap_or_else(|| site_config.seo.twitter_card.clone())
    };

    // twitter:site from site config only (no per-page override).
    let twitter_site = site_config.seo.twitter_site.clone();

    // Canonical URL: frontmatter override > auto-derived.
    let canonical_url = frontmatter_seo
        .canonical_url
        .clone()
        .unwrap_or_else(|| {
            // Strip index.html for cleaner canonical URLs.
            let clean_path = if current_url.ends_with("/index.html") {
                &current_url[..current_url.len() - "index.html".len()]
            } else {
                current_url
            };
            make_absolute_url(clean_path, &site_config.base_url)
        });

    ResolvedSeo {
        title,
        description,
        image,
        og_type,
        twitter_card,
        twitter_site,
        canonical_url,
        site_name: site_config.name.clone(),
    }
}

/// Scan rendered HTML for existing `<meta property="og:*">` and
/// `<meta name="twitter:*">` tags.
///
/// Returns the set of tag names already present (e.g. "og:title",
/// "twitter:card").
fn detect_existing_tags(html: &str) -> HashSet<String> {
    let existing: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));

    // Scan for og:* tags (property attribute).
    let existing_og = existing.clone();
    let og_handler = lol_html::element!("meta[property]", move |el| {
        if let Some(prop) = el.get_attribute("property") {
            if prop.starts_with("og:") {
                existing_og.borrow_mut().insert(prop);
            }
        }
        Ok(())
    });

    // Scan for twitter:* tags (name attribute).
    let existing_tw = existing.clone();
    let tw_handler = lol_html::element!("meta[name]", move |el| {
        if let Some(name) = el.get_attribute("name") {
            if name.starts_with("twitter:") {
                existing_tw.borrow_mut().insert(name);
            }
        }
        Ok(())
    });

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![og_handler, tw_handler],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    existing.borrow().clone()
}

/// Check if the HTML already contains a `<link rel="canonical">` tag.
fn has_canonical_link(html: &str) -> bool {
    let found: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_clone = found.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!(
                "link[rel='canonical']",
                move |_el| {
                    *found_clone.borrow_mut() = true;
                    Ok(())
                }
            )],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    *found.borrow()
}

/// Append a `<meta>` tag to `out` if `name` is not in `existing`.
///
/// `attr` is the HTML attribute name: `"property"` for OG tags,
/// `"name"` for Twitter tags.
fn push_meta(
    out: &mut String,
    existing: &HashSet<String>,
    attr: &str,
    name: &str,
    content: &str,
) {
    use std::fmt::Write;
    if !existing.contains(name) {
        let _ = write!(
            out,
            r#"<meta {}="{}" content="{}">"#,
            attr,
            name,
            escape_attr(content),
        );
        out.push('\n');
    }
}

/// Generate meta tag HTML string for all SEO tags that are not
/// already present in the document.
///
/// Returns an empty string if all tags are already present.
fn generate_meta_html(
    seo: &ResolvedSeo,
    existing: &HashSet<String>,
    has_canonical: bool,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    // Open Graph tags.
    push_meta(&mut out, existing, "property", "og:title", &seo.title);
    if let Some(ref desc) = seo.description {
        push_meta(&mut out, existing, "property", "og:description", desc);
    }
    if let Some(ref img) = seo.image {
        push_meta(&mut out, existing, "property", "og:image", img);
    }
    push_meta(&mut out, existing, "property", "og:url", &seo.canonical_url);
    push_meta(&mut out, existing, "property", "og:type", &seo.og_type);
    push_meta(&mut out, existing, "property", "og:site_name", &seo.site_name);

    // Twitter Card tags.
    push_meta(&mut out, existing, "name", "twitter:card", &seo.twitter_card);
    push_meta(&mut out, existing, "name", "twitter:title", &seo.title);
    if let Some(ref desc) = seo.description {
        push_meta(&mut out, existing, "name", "twitter:description", desc);
    }
    if let Some(ref img) = seo.image {
        push_meta(&mut out, existing, "name", "twitter:image", img);
    }
    if let Some(ref handle) = seo.twitter_site {
        push_meta(&mut out, existing, "name", "twitter:site", handle);
    }

    // Canonical URL.
    if !has_canonical {
        let _ = write!(
            out,
            r#"<link rel="canonical" href="{}">"#,
            escape_attr(&seo.canonical_url),
        );
        out.push('\n');
    }

    out
}

/// Inject meta tag HTML into the `<head>` element.
///
/// Uses lol_html to append content at the end of `<head>`. Unlike
/// resource hints (which prepend early for performance), SEO tags are
/// appended late because their position does not affect performance
/// and this avoids interfering with preload hint ordering.
fn inject_into_head(html: &str, meta_html: &str) -> Result<String, String> {
    let meta_owned = meta_html.to_string();

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("head", move |el| {
                el.append(
                    &meta_owned,
                    lol_html::html_content::ContentType::Html,
                );
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| e.to_string())
}

/// Inject Open Graph and Twitter Card meta tags into HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
///
/// Steps:
/// 1. Resolve SEO values from frontmatter + site config + defaults.
/// 2. Scan HTML for existing OG/Twitter meta tags.
/// 3. Generate meta tag HTML for missing tags only.
/// 4. Inject the tags into `<head>` using lol_html.
///
/// Returns the (possibly rewritten) HTML string.
pub fn inject_seo_tags(
    html: &str,
    frontmatter_seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> String {
    // 1. Resolve SEO values.
    let resolved = resolve_seo(frontmatter_seo, site_config, current_url);

    // 2. Detect existing tags.
    let existing = detect_existing_tags(html);
    let has_canonical = has_canonical_link(html);

    // 3. Generate meta HTML for missing tags.
    let meta_html = generate_meta_html(&resolved, &existing, has_canonical);

    if meta_html.is_empty() {
        return html.to_string();
    }

    // 4. Inject into <head>.
    match inject_into_head(html, &meta_html) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("Failed to inject SEO meta tags: {}", e);
            html.to_string()
        }
    }
}

/// Resolve template expressions in SEO frontmatter fields.
///
/// Uses `minijinja::Environment::render_str` to evaluate expressions
/// like `{{ post.title }}` in the context of the current page.
///
/// If rendering fails for any field, the field is left as `None`
/// (falls back to site defaults) and a warning is logged.
///
/// For fields that contain no template expressions (no `{{` / `{%`),
/// the value is returned as-is without calling render_str (fast path).
pub fn resolve_seo_expressions(
    seo: &SeoMeta,
    env: &minijinja::Environment<'_>,
    ctx: &minijinja::Value,
) -> SeoMeta {
    fn resolve_field(
        field: &Option<String>,
        env: &minijinja::Environment<'_>,
        ctx: &minijinja::Value,
        field_name: &str,
    ) -> Option<String> {
        let value = field.as_ref()?;

        // Fast path: no template expressions.
        if !value.contains("{{") && !value.contains("{%") {
            return Some(value.clone());
        }

        match env.render_str(value, ctx.clone()) {
            Ok(rendered) => {
                let trimmed = rendered.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            Err(err) => {
                tracing::warn!(
                    "Failed to resolve SEO field '{}' expression '{}': {}",
                    field_name,
                    value,
                    err,
                );
                None
            }
        }
    }

    SeoMeta {
        title: resolve_field(&seo.title, env, ctx, "title"),
        description: resolve_field(&seo.description, env, ctx, "description"),
        image: resolve_field(&seo.image, env, ctx, "image"),
        og_type: resolve_field(&seo.og_type, env, ctx, "og_type"),
        twitter_card: resolve_field(&seo.twitter_card, env, ctx, "twitter_card"),
        canonical_url: resolve_field(&seo.canonical_url, env, ctx, "canonical_url"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SiteSchemaConfig, SiteSeoConfig};
    use hegel::generators;

    /// Build a minimal SiteMeta for testing.
    fn test_site(name: &str, base_url: &str) -> SiteMeta {
        SiteMeta {
            name: name.to_string(),
            base_url: base_url.to_string(),
            seo: SiteSeoConfig::default(),
            schema: SiteSchemaConfig::default(),
            extra: std::collections::HashMap::new(),
        }
    }

    /// Build a SiteMeta with custom SEO defaults.
    fn test_site_with_seo(
        name: &str,
        base_url: &str,
        seo: SiteSeoConfig,
    ) -> SiteMeta {
        SiteMeta {
            name: name.to_string(),
            base_url: base_url.to_string(),
            seo,
            schema: SiteSchemaConfig::default(),
            extra: std::collections::HashMap::new(),
        }
    }

    // --- escape_attr tests ---

    #[test]
    fn test_escape_attr_special_chars() {
        assert_eq!(escape_attr(r#"He said "hello""#), r#"He said &quot;hello&quot;"#);
        assert_eq!(escape_attr("A & B"), "A &amp; B");
        assert_eq!(escape_attr("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_attr("plain text"), "plain text");
    }

    #[test]
    fn test_escape_attr_empty() {
        assert_eq!(escape_attr(""), "");
    }

    // --- make_absolute_url tests ---

    #[test]
    fn test_make_absolute_url_already_absolute() {
        assert_eq!(
            make_absolute_url("https://cdn.example.com/img.jpg", "https://example.com"),
            "https://cdn.example.com/img.jpg",
        );
    }

    #[test]
    fn test_make_absolute_url_relative_path() {
        assert_eq!(
            make_absolute_url("/assets/hero.jpg", "https://example.com"),
            "https://example.com/assets/hero.jpg",
        );
    }

    #[test]
    fn test_make_absolute_url_base_trailing_slash() {
        assert_eq!(
            make_absolute_url("/about.html", "https://example.com/"),
            "https://example.com/about.html",
        );
    }

    // --- resolve_seo tests ---

    #[test]
    fn test_resolve_seo_all_defaults() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let resolved = resolve_seo(&seo, &site, "/about.html");

        assert_eq!(resolved.title, "My Site");
        assert!(resolved.description.is_none());
        assert!(resolved.image.is_none());
        assert_eq!(resolved.og_type, "website");
        assert_eq!(resolved.twitter_card, "summary"); // no image -> summary
        assert!(resolved.twitter_site.is_none());
        assert_eq!(resolved.canonical_url, "https://example.com/about.html");
        assert_eq!(resolved.site_name, "My Site");
    }

    #[test]
    fn test_resolve_seo_frontmatter_overrides() {
        let site = test_site_with_seo("My Site", "https://example.com", SiteSeoConfig {
            title: Some("Site Default Title".into()),
            description: Some("Site default desc".into()),
            ..SiteSeoConfig::default()
        });
        let seo = SeoMeta {
            title: Some("Page Title".into()),
            description: Some("Page description".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.title, "Page Title");
        assert_eq!(resolved.description.as_deref(), Some("Page description"));
    }

    #[test]
    fn test_resolve_seo_site_defaults_fill_gaps() {
        let site = test_site_with_seo("My Site", "https://example.com", SiteSeoConfig {
            description: Some("Site description".into()),
            image: Some("/assets/default-share.jpg".into()),
            ..SiteSeoConfig::default()
        });
        let seo = SeoMeta::default(); // no frontmatter overrides
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.description.as_deref(), Some("Site description"));
        assert_eq!(resolved.image.as_deref(), Some("https://example.com/assets/default-share.jpg"));
    }

    #[test]
    fn test_resolve_seo_canonical_url_auto() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let resolved = resolve_seo(&seo, &site, "/blog/post.html");

        assert_eq!(resolved.canonical_url, "https://example.com/blog/post.html");
    }

    #[test]
    fn test_resolve_seo_canonical_url_override() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            canonical_url: Some("https://original-source.com/article".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/syndicated.html");

        assert_eq!(resolved.canonical_url, "https://original-source.com/article");
    }

    #[test]
    fn test_resolve_seo_canonical_strips_index() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();

        let resolved = resolve_seo(&seo, &site, "/index.html");
        assert_eq!(resolved.canonical_url, "https://example.com/");

        let resolved = resolve_seo(&seo, &site, "/blog/index.html");
        assert_eq!(resolved.canonical_url, "https://example.com/blog/");
    }

    #[test]
    fn test_resolve_seo_image_absolute_url() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            image: Some("https://cdn.example.com/photo.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.image.as_deref(), Some("https://cdn.example.com/photo.jpg"));
    }

    #[test]
    fn test_resolve_seo_image_relative_path() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            image: Some("/assets/hero.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.image.as_deref(), Some("https://example.com/assets/hero.jpg"));
    }

    #[test]
    fn test_resolve_seo_base_url_slash_normalization() {
        let site = test_site("My Site", "https://example.com/");
        let seo = SeoMeta {
            image: Some("/assets/hero.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/about.html");

        // No double slash.
        assert_eq!(resolved.image.as_deref(), Some("https://example.com/assets/hero.jpg"));
        assert_eq!(resolved.canonical_url, "https://example.com/about.html");
    }

    #[test]
    fn test_resolve_seo_twitter_card_no_image() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default(); // no image
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.twitter_card, "summary");
    }

    #[test]
    fn test_resolve_seo_twitter_card_with_image() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            image: Some("/assets/hero.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.twitter_card, "summary_large_image");
    }

    // --- detect_existing_tags / has_canonical_link tests ---

    #[test]
    fn test_detect_existing_og_tags() {
        let html = r#"<html><head><meta property="og:title" content="Test"></head><body></body></html>"#;
        let tags = detect_existing_tags(html);
        assert!(tags.contains("og:title"));
        assert!(!tags.contains("og:description"));
    }

    #[test]
    fn test_detect_existing_twitter_tags() {
        let html = r#"<html><head><meta name="twitter:card" content="summary"></head><body></body></html>"#;
        let tags = detect_existing_tags(html);
        assert!(tags.contains("twitter:card"));
        assert!(!tags.contains("twitter:title"));
    }

    #[test]
    fn test_detect_no_existing_tags() {
        let html = r#"<html><head><title>Page</title></head><body></body></html>"#;
        let tags = detect_existing_tags(html);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_has_canonical_link_present() {
        let html = r#"<html><head><link rel="canonical" href="https://example.com/page"></head><body></body></html>"#;
        assert!(has_canonical_link(html));
    }

    #[test]
    fn test_has_canonical_link_absent() {
        let html = r#"<html><head><title>Page</title></head><body></body></html>"#;
        assert!(!has_canonical_link(html));
    }

    // --- generate_meta_html tests ---

    #[test]
    fn test_generate_meta_html_full() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: Some("A description".into()),
            image: Some("https://example.com/img.jpg".into()),
            og_type: "website".into(),
            twitter_card: "summary_large_image".into(),
            twitter_site: Some("@mysite".into()),
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), false);

        assert!(html.contains(r#"<meta property="og:title" content="My Page">"#));
        assert!(html.contains(r#"<meta property="og:description" content="A description">"#));
        assert!(html.contains(r#"<meta property="og:image" content="https://example.com/img.jpg">"#));
        assert!(html.contains(r#"<meta property="og:url" content="https://example.com/page.html">"#));
        assert!(html.contains(r#"<meta property="og:type" content="website">"#));
        assert!(html.contains(r#"<meta property="og:site_name" content="My Site">"#));
        assert!(html.contains(r#"<meta name="twitter:card" content="summary_large_image">"#));
        assert!(html.contains(r#"<meta name="twitter:title" content="My Page">"#));
        assert!(html.contains(r#"<meta name="twitter:description" content="A description">"#));
        assert!(html.contains(r#"<meta name="twitter:image" content="https://example.com/img.jpg">"#));
        assert!(html.contains(r#"<meta name="twitter:site" content="@mysite">"#));
        assert!(html.contains(r#"<link rel="canonical" href="https://example.com/page.html">"#));
    }

    #[test]
    fn test_generate_meta_html_no_description() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: None,
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), false);

        assert!(!html.contains("og:description"));
        assert!(!html.contains("twitter:description"));
    }

    #[test]
    fn test_generate_meta_html_no_image() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: None,
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), false);

        assert!(!html.contains("og:image"));
        assert!(!html.contains("twitter:image"));
    }

    #[test]
    fn test_generate_meta_html_skips_existing() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: Some("Desc".into()),
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let mut existing = HashSet::new();
        existing.insert("og:title".into());
        existing.insert("twitter:card".into());
        let html = generate_meta_html(&seo, &existing, false);

        assert!(!html.contains(r#"<meta property="og:title""#));
        assert!(!html.contains(r#"<meta name="twitter:card""#));
        // Other tags should still be present.
        assert!(html.contains(r#"<meta property="og:description""#));
        assert!(html.contains(r#"<meta name="twitter:title""#));
    }

    #[test]
    fn test_generate_meta_html_skips_canonical() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: None,
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), true);

        assert!(!html.contains("canonical"));
    }

    // --- inject_seo_tags integration tests ---

    #[test]
    fn test_inject_seo_tags_basic() {
        let html = "<html><head><title>Test</title></head><body>Hello</body></html>";
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            title: Some("Page Title".into()),
            description: Some("Page desc".into()),
            ..SeoMeta::default()
        };
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        assert!(result.contains(r#"<meta property="og:title" content="Page Title">"#));
        assert!(result.contains(r#"<meta property="og:description" content="Page desc">"#));
        assert!(result.contains(r#"<meta name="twitter:card" content="summary">"#));
        assert!(result.contains(r#"<link rel="canonical" href="https://example.com/page.html">"#));
        // Original content is preserved.
        assert!(result.contains("<title>Test</title>"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn test_inject_seo_tags_no_head() {
        let html = "<div>No head element</div>";
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        // Should not crash. No <head> means no injection target --
        // lol_html returns the HTML without matching the `head` selector.
        assert!(!result.is_empty());
        assert!(result.contains("No head element"));
        // No meta tags injected.
        assert!(!result.contains("og:title"));
    }

    #[test]
    fn test_inject_seo_tags_all_existing() {
        let html = concat!(
            r#"<html><head>"#,
            r#"<meta property="og:title" content="Existing">"#,
            r#"<meta property="og:url" content="https://example.com/page">"#,
            r#"<meta property="og:type" content="website">"#,
            r#"<meta property="og:site_name" content="Site">"#,
            r#"<meta name="twitter:card" content="summary">"#,
            r#"<meta name="twitter:title" content="Existing">"#,
            r#"<link rel="canonical" href="https://example.com/page">"#,
            r#"</head><body></body></html>"#,
        );
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        // No new tags injected -- the existing ones are preserved, no duplicates.
        // Count og:title occurrences to ensure no duplication.
        assert_eq!(result.matches("og:title").count(), 1);
        assert_eq!(result.matches("twitter:card").count(), 1);
    }

    #[test]
    fn test_inject_seo_tags_partial_existing() {
        let html = concat!(
            r#"<html><head>"#,
            r#"<meta property="og:title" content="Custom Title">"#,
            r#"</head><body></body></html>"#,
        );
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            title: Some("Page Title".into()),
            ..SeoMeta::default()
        };
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        // og:title should NOT be duplicated (existing one kept).
        assert_eq!(result.matches("og:title").count(), 1);
        assert!(result.contains("Custom Title"));
        // But other tags should be injected.
        assert!(result.contains("og:url"));
        assert!(result.contains("og:type"));
        assert!(result.contains("twitter:card"));
        assert!(result.contains("canonical"));
    }

    // --- resolve_seo_expressions tests ---

    #[test]
    fn test_resolve_seo_expressions_basic() {
        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        let ctx = minijinja::Value::from_serialize(
            &serde_json::json!({"post": {"title": "Hello World", "excerpt": "An intro"}}),
        );
        let seo = SeoMeta {
            title: Some("{{ post.title }} | Blog".into()),
            description: Some("{{ post.excerpt }}".into()),
            image: None,
            og_type: Some("article".into()),
            twitter_card: None,
            canonical_url: None,
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        assert_eq!(resolved.title.as_deref(), Some("Hello World | Blog"));
        assert_eq!(resolved.description.as_deref(), Some("An intro"));
        assert_eq!(resolved.og_type.as_deref(), Some("article")); // no expression, pass-through
    }

    #[test]
    fn test_resolve_seo_expressions_no_expressions() {
        let env = minijinja::Environment::new();
        let ctx = minijinja::Value::from(true); // dummy context
        let seo = SeoMeta {
            title: Some("Static Title".into()),
            description: Some("Static desc".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        assert_eq!(resolved.title.as_deref(), Some("Static Title"));
        assert_eq!(resolved.description.as_deref(), Some("Static desc"));
    }

    #[test]
    fn test_resolve_seo_expressions_empty_result() {
        let env = minijinja::Environment::new();
        let ctx = minijinja::Value::from_serialize(
            &serde_json::json!({"post": {"excerpt": ""}}),
        );
        let seo = SeoMeta {
            description: Some("{{ post.excerpt }}".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        // Empty result treated as None.
        assert!(resolved.description.is_none());
    }

    #[test]
    fn test_resolve_seo_expressions_missing_var() {
        let mut env = minijinja::Environment::new();
        // Match production config: strict mode causes error on undefined vars.
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        let ctx = minijinja::Value::from_serialize(&serde_json::json!({}));
        let seo = SeoMeta {
            title: Some("{{ nonexistent.field }}".into()),
            description: Some("Static fallback".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        // Missing var in strict mode -> render_str errors -> field becomes None.
        assert!(resolved.title.is_none());
        // Static field unaffected.
        assert_eq!(resolved.description.as_deref(), Some("Static fallback"));
    }

    // --- property-based tests (hegeltest) ---

    #[hegel::test]
    fn escape_attr_no_bare_specials(tc: hegel::TestCase) {
        let s = tc.draw(generators::text());
        let escaped = escape_attr(&s);
        let cleaned = escaped
            .replace("&amp;", "")
            .replace("&quot;", "")
            .replace("&lt;", "")
            .replace("&gt;", "");
        assert!(!cleaned.contains('&'), "bare '&' found in: {escaped}");
        assert!(!cleaned.contains('"'), "bare '\"' found in: {escaped}");
        assert!(!cleaned.contains('<'), "bare '<' found in: {escaped}");
        assert!(!cleaned.contains('>'), "bare '>' found in: {escaped}");
    }

    #[hegel::test]
    fn escape_attr_monotonic_length(tc: hegel::TestCase) {
        let s = tc.draw(generators::text());
        assert!(
            escape_attr(&s).len() >= s.len(),
            "escaped output shorter than input for: {s:?}"
        );
    }

    #[hegel::test]
    fn escape_attr_no_crash(tc: hegel::TestCase) {
        let s = tc.draw(generators::text());
        let _ = escape_attr(&s);
    }

    #[hegel::test]
    fn make_absolute_url_idempotent_for_absolute(tc: hegel::TestCase) {
        let url = tc.draw(generators::urls());
        let base = tc.draw(generators::urls());
        assert_eq!(
            make_absolute_url(&url, &base),
            url,
            "absolute URL was modified: url={url:?}, base={base:?}"
        );
    }

    #[hegel::test]
    fn make_absolute_url_no_double_slash(tc: hegel::TestCase) {
        let raw_suffix = tc.draw(generators::text().max_size(50));
        let path_suffix = raw_suffix.trim_start_matches('/');
        let path = format!("/{path_suffix}");

        for base in &["https://example.com", "https://example.com/"] {
            let result = make_absolute_url(&path, base);
            // After the scheme's "://", there should be no "//".
            if let Some(rest) = result.strip_prefix("https://") {
                assert!(
                    !rest.contains("//"),
                    "double slash in result: {result:?} (path={path:?}, base={base:?})"
                );
            }
        }
    }
}
