//! JSON-LD structured data injection.
//!
//! Auto-injects `<script type="application/ld+json">` blocks into `<head>`
//! during the build pipeline. Supports Article, BreadcrumbList, and WebSite
//! schema.org types.
//!
//! Two public functions:
//! - `inject_json_ld`: the pipeline step (after seo, before minify).
//! - `resolve_schema_expressions`: template expression evaluation for dynamic
//!   page schema fields (called earlier in the render flow).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::config::{SiteMeta, SiteSchemaConfig};
use crate::frontmatter::{SchemaConfig, SchemaConfigValue, SchemaFullConfig, SeoMeta};

/// Check if the HTML already contains a `<script type="application/ld+json">`
/// block.
fn has_existing_json_ld(html: &str) -> bool {
    let found: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_clone = found.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!(
                "script[type='application/ld+json']",
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

/// Determine which schema types to generate from page-level and site-level config.
///
/// Returns an empty vec if no schemas should be generated.
fn resolve_schema_types(
    schema: &SchemaConfig,
    site_schema: &SiteSchemaConfig,
) -> Vec<String> {
    match schema {
        Some(SchemaConfigValue::TypeName(t)) => vec![t.clone()],
        Some(SchemaConfigValue::TypeList(types)) => types.clone(),
        Some(SchemaConfigValue::Full(full)) => {
            full.types.to_vec().into_iter().map(|s| s.to_string()).collect()
        }
        None => {
            // Fall back to site-level default_types.
            site_schema.default_types.clone()
        }
    }
}

/// Resolve a URL to absolute form using the site's base_url.
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

/// Compute the canonical URL for a page.
fn canonical_url(current_url: &str, base_url: &str) -> String {
    let clean_path = if current_url.ends_with("/index.html") {
        &current_url[..current_url.len() - "index.html".len()]
    } else {
        current_url
    };
    make_absolute_url(clean_path, base_url)
}

/// Build the Article JSON-LD object.
fn build_article_schema(
    schema: &SchemaConfig,
    seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("@context".into(), "https://schema.org".into());
    obj.insert("@type".into(), "Article".into());

    // Headline: seo.title > site.seo.title > site.name
    let headline = seo.title.as_deref()
        .or(site_config.seo.title.as_deref())
        .unwrap_or(&site_config.name);
    obj.insert("headline".into(), headline.into());

    // URL
    let url = canonical_url(current_url, &site_config.base_url);
    obj.insert("url".into(), url.into());

    // Description (optional)
    let description = seo.description.as_deref()
        .or(site_config.seo.description.as_deref());
    if let Some(desc) = description {
        obj.insert("description".into(), desc.into());
    }

    // Image (optional, must be absolute)
    let image = seo.image.as_deref()
        .or(site_config.seo.image.as_deref());
    if let Some(img) = image {
        let abs_img = make_absolute_url(img, &site_config.base_url);
        obj.insert("image".into(), abs_img.into());
    }

    // Extract overrides from SchemaFullConfig if available.
    let full = match schema {
        Some(SchemaConfigValue::Full(f)) => Some(f),
        _ => None,
    };

    // Dates (optional)
    if let Some(date) = full.and_then(|f| f.date_published.as_deref()) {
        obj.insert("datePublished".into(), date.into());
    }
    if let Some(date) = full.and_then(|f| f.date_modified.as_deref()) {
        obj.insert("dateModified".into(), date.into());
    } else if let Some(date) = full.and_then(|f| f.date_published.as_deref()) {
        // Fall back dateModified to datePublished.
        obj.insert("dateModified".into(), date.into());
    }

    // Author (optional)
    let author_name = full.and_then(|f| f.author.as_deref())
        .or(site_config.schema.author.as_deref());
    if let Some(name) = author_name {
        obj.insert("author".into(), serde_json::json!({
            "@type": "Person",
            "name": name,
        }));
    }

    // Publisher (always present -- site.name)
    obj.insert("publisher".into(), serde_json::json!({
        "@type": "Organization",
        "name": site_config.name,
    }));

    serde_json::Value::Object(obj)
}

/// Build the BreadcrumbList JSON-LD object from the page URL path.
///
/// Returns `None` for root pages (single-item breadcrumbs are not useful).
fn build_breadcrumb_schema(
    schema: &SchemaConfig,
    site_config: &SiteMeta,
    current_url: &str,
) -> Option<serde_json::Value> {
    // Parse URL path into segments.
    let path = current_url.trim_start_matches('/');
    if path.is_empty() || path == "index.html" {
        return None; // Root page -- skip.
    }

    let segments: Vec<&str> = path.split('/').collect();
    if segments.is_empty() {
        return None;
    }

    // Extract breadcrumb name overrides.
    let empty = HashMap::new();
    let name_overrides: &HashMap<String, String> = match schema {
        Some(SchemaConfigValue::Full(f)) => {
            f.breadcrumb_names.as_ref().unwrap_or(&empty)
        }
        _ => &empty,
    };

    let base = site_config.base_url.trim_end_matches('/');
    let mut items = Vec::with_capacity(segments.len() + 1);

    // First item: Home
    items.push(serde_json::json!({
        "@type": "ListItem",
        "position": 1,
        "name": "Home",
        "item": format!("{}/", base),
    }));

    // Build path progressively.
    let mut accumulated_path = String::new();
    for (i, segment) in segments.iter().enumerate() {
        accumulated_path.push('/');
        accumulated_path.push_str(segment);

        let display_name = name_overrides.get(*segment)
            .map(|s| s.as_str())
            .unwrap_or_else(|| {
                // Strip .html extension for the last segment.
                segment.strip_suffix(".html").unwrap_or(segment)
            });

        let item_url = if i == segments.len() - 1 {
            // Last segment: use the full URL.
            format!("{}{}", base, accumulated_path)
        } else {
            // Intermediate: directory URL.
            format!("{}{}/", base, accumulated_path)
        };

        items.push(serde_json::json!({
            "@type": "ListItem",
            "position": i + 2, // 1-indexed, Home is position 1
            "name": display_name,
            "item": item_url,
        }));
    }

    Some(serde_json::json!({
        "@context": "https://schema.org",
        "@type": "BreadcrumbList",
        "itemListElement": items,
    }))
}

/// Build the WebSite JSON-LD object.
fn build_website_schema(site_config: &SiteMeta) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("@context".into(), "https://schema.org".into());
    obj.insert("@type".into(), "WebSite".into());
    obj.insert("name".into(), site_config.name.clone().into());

    let url = site_config.base_url.trim_end_matches('/');
    obj.insert("url".into(), format!("{}/", url).into());

    if let Some(ref desc) = site_config.seo.description {
        obj.insert("description".into(), desc.clone().into());
    }

    serde_json::Value::Object(obj)
}

/// Inject JSON-LD script block(s) into the `<head>` element.
fn inject_into_head(html: &str, script_html: &str) -> Result<String, String> {
    let script_owned = script_html.to_string();

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("head", move |el| {
                el.append(
                    &script_owned,
                    lol_html::html_content::ContentType::Html,
                );
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| e.to_string())
}

/// Inject JSON-LD structured data script block into HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
pub fn inject_json_ld(
    html: &str,
    schema: &SchemaConfig,
    seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> String {
    // 1. Determine which schema types to generate.
    let types = resolve_schema_types(schema, &site_config.schema);
    if types.is_empty() {
        return html.to_string();
    }

    // 2. Check for existing JSON-LD.
    if has_existing_json_ld(html) {
        tracing::debug!("Existing JSON-LD found, skipping injection for {}", current_url);
        return html.to_string();
    }

    // 3. Build JSON-LD objects.
    let mut script_blocks = String::new();
    for type_name in &types {
        let json_value = match type_name.as_str() {
            "Article" => Some(build_article_schema(schema, seo, site_config, current_url)),
            "BreadcrumbList" => build_breadcrumb_schema(schema, site_config, current_url),
            "WebSite" => Some(build_website_schema(site_config)),
            other => {
                tracing::warn!(
                    "Unrecognized schema type '{}' for page {}, skipping.",
                    other,
                    current_url,
                );
                None
            }
        };

        if let Some(value) = json_value {
            match serde_json::to_string(&value) {
                Ok(json_str) => {
                    script_blocks.push_str(r#"<script type="application/ld+json">"#);
                    script_blocks.push_str(&json_str);
                    script_blocks.push_str("</script>\n");
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to serialize JSON-LD for type '{}': {}",
                        type_name, e,
                    );
                }
            }
        }
    }

    if script_blocks.is_empty() {
        return html.to_string();
    }

    // 4. Inject into <head>.
    match inject_into_head(html, &script_blocks) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("Failed to inject JSON-LD: {}", e);
            html.to_string()
        }
    }
}

/// Resolve template expressions in schema frontmatter fields.
///
/// For fields that contain no template expressions (no `{{` / `{%`),
/// the value is returned as-is without calling render_str (fast path).
pub fn resolve_schema_expressions(
    schema: &SchemaConfig,
    env: &minijinja::Environment<'_>,
    ctx: &minijinja::Value,
) -> SchemaConfig {
    match schema {
        Some(SchemaConfigValue::Full(full)) => {
            Some(SchemaConfigValue::Full(SchemaFullConfig {
                types: full.types.clone(),
                author: resolve_field(&full.author, env, ctx, "schema.author"),
                date_published: resolve_field(&full.date_published, env, ctx, "schema.date_published"),
                date_modified: resolve_field(&full.date_modified, env, ctx, "schema.date_modified"),
                breadcrumb_names: full.breadcrumb_names.clone(), // No expression resolution for map values.
            }))
        }
        // TypeName, TypeList, and None have no fields to resolve.
        other => other.clone(),
    }
}

/// Resolve a single optional string field that may contain template expressions.
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
                "Failed to resolve '{}' expression '{}': {}",
                field_name, value, err,
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SiteSeoConfig, SiteSchemaConfig};
    use crate::frontmatter::SchemaTypes;

    fn test_site(name: &str, base_url: &str) -> SiteMeta {
        SiteMeta {
            name: name.to_string(),
            base_url: base_url.to_string(),
            seo: SiteSeoConfig::default(),
            schema: SiteSchemaConfig::default(),
            extra: std::collections::HashMap::new(),
        }
    }

    fn test_seo(title: Option<&str>, desc: Option<&str>, image: Option<&str>) -> SeoMeta {
        SeoMeta {
            title: title.map(|s| s.to_string()),
            description: desc.map(|s| s.to_string()),
            image: image.map(|s| s.to_string()),
            ..SeoMeta::default()
        }
    }

    // --- has_existing_json_ld tests ---

    #[test]
    fn test_has_existing_json_ld_present() {
        let html = r#"<html><head><script type="application/ld+json">{"@type":"Article"}</script></head><body></body></html>"#;
        assert!(has_existing_json_ld(html));
    }

    #[test]
    fn test_has_existing_json_ld_absent() {
        let html = r#"<html><head><title>Page</title></head><body></body></html>"#;
        assert!(!has_existing_json_ld(html));
    }

    #[test]
    fn test_has_existing_json_ld_other_script() {
        let html = r#"<html><head><script src="app.js"></script></head><body></body></html>"#;
        assert!(!has_existing_json_ld(html));
    }

    // --- resolve_schema_types tests ---

    #[test]
    fn test_resolve_schema_types_from_frontmatter_string() {
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Article".into()));
        let site = SiteSchemaConfig::default();
        let types = resolve_schema_types(&schema, &site);
        assert_eq!(types, vec!["Article"]);
    }

    #[test]
    fn test_resolve_schema_types_from_frontmatter_list() {
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeList(
            vec!["Article".into(), "BreadcrumbList".into()],
        ));
        let site = SiteSchemaConfig::default();
        let types = resolve_schema_types(&schema, &site);
        assert_eq!(types, vec!["Article", "BreadcrumbList"]);
    }

    #[test]
    fn test_resolve_schema_types_from_frontmatter_full() {
        let schema: SchemaConfig = Some(SchemaConfigValue::Full(SchemaFullConfig {
            types: SchemaTypes::Multiple(vec!["Article".into(), "WebSite".into()]),
            author: None,
            date_published: None,
            date_modified: None,
            breadcrumb_names: None,
        }));
        let site = SiteSchemaConfig::default();
        let types = resolve_schema_types(&schema, &site);
        assert_eq!(types, vec!["Article", "WebSite"]);
    }

    #[test]
    fn test_resolve_schema_types_site_defaults() {
        let schema: SchemaConfig = None;
        let site = SiteSchemaConfig {
            default_types: vec!["BreadcrumbList".into()],
            ..SiteSchemaConfig::default()
        };
        let types = resolve_schema_types(&schema, &site);
        assert_eq!(types, vec!["BreadcrumbList"]);
    }

    #[test]
    fn test_resolve_schema_types_none() {
        let schema: SchemaConfig = None;
        let site = SiteSchemaConfig::default();
        let types = resolve_schema_types(&schema, &site);
        assert!(types.is_empty());
    }

    // --- build_article_schema tests ---

    #[test]
    fn test_build_article_schema_full() {
        let schema: SchemaConfig = Some(SchemaConfigValue::Full(SchemaFullConfig {
            types: SchemaTypes::Single("Article".into()),
            author: Some("Jane Doe".into()),
            date_published: Some("2026-03-15".into()),
            date_modified: Some("2026-03-17".into()),
            breadcrumb_names: None,
        }));
        let seo = test_seo(
            Some("My Post"),
            Some("A post description"),
            Some("/assets/hero.jpg"),
        );
        let site = test_site("My Blog", "https://example.com");

        let value = build_article_schema(&schema, &seo, &site, "/blog/my-post.html");

        assert_eq!(value["@type"], "Article");
        assert_eq!(value["headline"], "My Post");
        assert_eq!(value["description"], "A post description");
        assert_eq!(value["image"], "https://example.com/assets/hero.jpg");
        assert_eq!(value["url"], "https://example.com/blog/my-post.html");
        assert_eq!(value["datePublished"], "2026-03-15");
        assert_eq!(value["dateModified"], "2026-03-17");
        assert_eq!(value["author"]["name"], "Jane Doe");
        assert_eq!(value["publisher"]["name"], "My Blog");
    }

    #[test]
    fn test_build_article_schema_minimal() {
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Article".into()));
        let seo = SeoMeta::default();
        let site = test_site("My Site", "https://example.com");

        let value = build_article_schema(&schema, &seo, &site, "/page.html");

        assert_eq!(value["headline"], "My Site"); // falls back to site.name
        assert_eq!(value["url"], "https://example.com/page.html");
        assert_eq!(value["publisher"]["name"], "My Site");
        // Optional fields should be absent.
        assert!(value.get("description").is_none());
        assert!(value.get("image").is_none());
        assert!(value.get("datePublished").is_none());
        assert!(value.get("author").is_none());
    }

    #[test]
    fn test_build_article_schema_auto_populates_from_seo() {
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Article".into()));
        let seo = test_seo(Some("SEO Title"), Some("SEO Desc"), Some("https://cdn.example.com/img.jpg"));
        let site = test_site("My Site", "https://example.com");

        let value = build_article_schema(&schema, &seo, &site, "/page.html");

        assert_eq!(value["headline"], "SEO Title");
        assert_eq!(value["description"], "SEO Desc");
        assert_eq!(value["image"], "https://cdn.example.com/img.jpg");
    }

    // --- build_breadcrumb_schema tests ---

    #[test]
    fn test_build_breadcrumb_schema() {
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("BreadcrumbList".into()));
        let site = test_site("My Site", "https://example.com");

        let value = build_breadcrumb_schema(&schema, &site, "/blog/posts/my-article.html");
        assert!(value.is_some());
        let value = value.unwrap();

        let items = value["itemListElement"].as_array().unwrap();
        assert_eq!(items.len(), 4); // Home + blog + posts + my-article

        assert_eq!(items[0]["name"], "Home");
        assert_eq!(items[0]["item"], "https://example.com/");
        assert_eq!(items[0]["position"], 1);

        assert_eq!(items[1]["name"], "blog");
        assert_eq!(items[1]["item"], "https://example.com/blog/");
        assert_eq!(items[1]["position"], 2);

        assert_eq!(items[2]["name"], "posts");
        assert_eq!(items[2]["item"], "https://example.com/blog/posts/");
        assert_eq!(items[2]["position"], 3);

        assert_eq!(items[3]["name"], "my-article");
        assert_eq!(items[3]["item"], "https://example.com/blog/posts/my-article.html");
        assert_eq!(items[3]["position"], 4);
    }

    #[test]
    fn test_build_breadcrumb_schema_root_page() {
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("BreadcrumbList".into()));
        let site = test_site("My Site", "https://example.com");

        assert!(build_breadcrumb_schema(&schema, &site, "/index.html").is_none());
        assert!(build_breadcrumb_schema(&schema, &site, "/").is_none());
    }

    #[test]
    fn test_build_breadcrumb_schema_custom_names() {
        let mut names = HashMap::new();
        names.insert("blog".into(), "Blog Posts".into());
        let schema: SchemaConfig = Some(SchemaConfigValue::Full(SchemaFullConfig {
            types: SchemaTypes::Single("BreadcrumbList".into()),
            author: None,
            date_published: None,
            date_modified: None,
            breadcrumb_names: Some(names),
        }));
        let site = test_site("My Site", "https://example.com");

        let value = build_breadcrumb_schema(&schema, &site, "/blog/post.html");
        assert!(value.is_some());
        let value = value.unwrap();

        let items = value["itemListElement"].as_array().unwrap();
        assert_eq!(items[1]["name"], "Blog Posts"); // overridden
        assert_eq!(items[2]["name"], "post"); // not overridden, .html stripped
    }

    // --- build_website_schema tests ---

    #[test]
    fn test_build_website_schema() {
        let mut site = test_site("My Site", "https://example.com");
        site.seo.description = Some("A great site".into());

        let value = build_website_schema(&site);

        assert_eq!(value["@type"], "WebSite");
        assert_eq!(value["name"], "My Site");
        assert_eq!(value["url"], "https://example.com/");
        assert_eq!(value["description"], "A great site");
    }

    #[test]
    fn test_build_website_schema_no_description() {
        let site = test_site("My Site", "https://example.com");

        let value = build_website_schema(&site);

        assert_eq!(value["name"], "My Site");
        assert!(value.get("description").is_none());
    }

    // --- inject_json_ld integration tests ---

    #[test]
    fn test_inject_json_ld_basic() {
        let html = "<html><head><title>Test</title></head><body>Hello</body></html>";
        let site = test_site("My Site", "https://example.com");
        let seo = test_seo(Some("Page Title"), None, None);
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Article".into()));

        let result = inject_json_ld(html, &schema, &seo, &site, "/page.html");

        assert!(result.contains(r#"<script type="application/ld+json">"#));
        assert!(result.contains(r#""@type":"Article""#));
        assert!(result.contains(r#""headline":"Page Title""#));
        // Original content preserved.
        assert!(result.contains("<title>Test</title>"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn test_inject_json_ld_no_head() {
        let html = "<div>No head element</div>";
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Article".into()));

        let result = inject_json_ld(html, &schema, &seo, &site, "/page.html");
        assert!(result.contains("No head element"));
        assert!(!result.contains("application/ld+json"));
    }

    #[test]
    fn test_inject_json_ld_existing_json_ld() {
        let html = concat!(
            r#"<html><head>"#,
            r#"<script type="application/ld+json">{"@type":"Organization"}</script>"#,
            r#"</head><body></body></html>"#,
        );
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Article".into()));

        let result = inject_json_ld(html, &schema, &seo, &site, "/page.html");

        // Should NOT inject additional JSON-LD.
        assert_eq!(result.matches("application/ld+json").count(), 1);
    }

    #[test]
    fn test_inject_json_ld_multiple_types() {
        let html = "<html><head></head><body></body></html>";
        let mut site = test_site("My Site", "https://example.com");
        site.seo.description = Some("A great site".into());
        let seo = test_seo(Some("My Post"), None, None);
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeList(
            vec!["Article".into(), "WebSite".into()],
        ));

        let result = inject_json_ld(html, &schema, &seo, &site, "/page.html");

        // Should have two JSON-LD blocks.
        assert_eq!(result.matches("application/ld+json").count(), 2);
        assert!(result.contains(r#""@type":"Article""#));
        assert!(result.contains(r#""@type":"WebSite""#));
    }

    #[test]
    fn test_inject_json_ld_no_schema() {
        let html = "<html><head></head><body></body></html>";
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let schema: SchemaConfig = None;

        let result = inject_json_ld(html, &schema, &seo, &site, "/page.html");

        // No injection.
        assert!(!result.contains("application/ld+json"));
    }

    #[test]
    fn test_inject_json_ld_unrecognized_type() {
        let html = "<html><head></head><body></body></html>";
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Product".into()));

        let result = inject_json_ld(html, &schema, &seo, &site, "/page.html");

        // Unrecognized type is skipped, no injection.
        assert!(!result.contains("application/ld+json"));
    }

    // --- resolve_schema_expressions tests ---

    #[test]
    fn test_resolve_schema_expressions_basic() {
        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        let ctx = minijinja::Value::from_serialize(
            &serde_json::json!({"post": {"author_name": "Jane", "published_at": "2026-03-15"}}),
        );
        let schema: SchemaConfig = Some(SchemaConfigValue::Full(SchemaFullConfig {
            types: SchemaTypes::Single("Article".into()),
            author: Some("{{ post.author_name }}".into()),
            date_published: Some("{{ post.published_at }}".into()),
            date_modified: None,
            breadcrumb_names: None,
        }));

        let resolved = resolve_schema_expressions(&schema, &env, &ctx);

        match &resolved {
            Some(SchemaConfigValue::Full(f)) => {
                assert_eq!(f.author.as_deref(), Some("Jane"));
                assert_eq!(f.date_published.as_deref(), Some("2026-03-15"));
            }
            other => panic!("Expected Some(Full), got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_schema_expressions_no_expressions() {
        let env = minijinja::Environment::new();
        let ctx = minijinja::Value::from(true);
        let schema: SchemaConfig = Some(SchemaConfigValue::Full(SchemaFullConfig {
            types: SchemaTypes::Single("Article".into()),
            author: Some("Static Author".into()),
            date_published: Some("2026-03-15".into()),
            date_modified: None,
            breadcrumb_names: None,
        }));

        let resolved = resolve_schema_expressions(&schema, &env, &ctx);

        match &resolved {
            Some(SchemaConfigValue::Full(f)) => {
                assert_eq!(f.author.as_deref(), Some("Static Author"));
                assert_eq!(f.date_published.as_deref(), Some("2026-03-15"));
            }
            other => panic!("Expected Some(Full), got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_schema_expressions_missing_var() {
        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        let ctx = minijinja::Value::from_serialize(&serde_json::json!({}));
        let schema: SchemaConfig = Some(SchemaConfigValue::Full(SchemaFullConfig {
            types: SchemaTypes::Single("Article".into()),
            author: Some("{{ nonexistent.field }}".into()),
            date_published: Some("Static date".into()),
            date_modified: None,
            breadcrumb_names: None,
        }));

        let resolved = resolve_schema_expressions(&schema, &env, &ctx);

        match &resolved {
            Some(SchemaConfigValue::Full(f)) => {
                // Missing var -> None.
                assert!(f.author.is_none());
                // Static field unaffected.
                assert_eq!(f.date_published.as_deref(), Some("Static date"));
            }
            other => panic!("Expected Some(Full), got {:?}", other),
        }
    }

    #[test]
    fn test_resolve_schema_expressions_type_name_passthrough() {
        let env = minijinja::Environment::new();
        let ctx = minijinja::Value::from(true);
        let schema: SchemaConfig = Some(SchemaConfigValue::TypeName("Article".into()));

        let resolved = resolve_schema_expressions(&schema, &env, &ctx);

        match &resolved {
            Some(SchemaConfigValue::TypeName(t)) => assert_eq!(t, "Article"),
            other => panic!("Expected Some(TypeName), got {:?}", other),
        }
    }
}
