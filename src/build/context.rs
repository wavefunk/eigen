//! Step 5.2: Context assembly.
//!
//! Builds the minijinja template context by merging:
//! 1. Global data (from `_data/`)
//! 2. Page-specific data (from frontmatter queries)
//! 3. Page metadata (`page.current_url`, `page.base_url`, etc.)
//! 4. Dynamic item (if applicable)
//!
//! Site config is already injected as a global by the template functions module.

use minijinja::Value;
use serde_json;
use std::collections::BTreeMap;

use crate::config::SiteConfig;

/// Metadata about the page currently being rendered.
pub struct PageMeta {
    /// The full URL path of the page, e.g. `/about.html`.
    pub current_url: String,
    /// The path relative to dist/, e.g. `about.html`.
    pub current_path: String,
    /// The site's base URL, e.g. `https://example.com`.
    pub base_url: String,
    /// ISO 8601 timestamp of when the build started.
    pub build_time: String,
}

/// Build the full template context for a page.
///
/// Merge order (later wins on conflict):
/// 1. Global data — each key at top level
/// 2. Page-specific data — from frontmatter queries
/// 3. `page` — metadata object
/// 4. Dynamic item — if this is a dynamic page, the current item
///
/// If a frontmatter `data` key has the same name as a `_data/` global data
/// key, the frontmatter data wins and a warning is logged.
pub fn build_page_context(
    _config: &SiteConfig,
    global_data: &std::collections::HashMap<String, serde_json::Value>,
    page_data: &std::collections::HashMap<String, serde_json::Value>,
    page_meta: PageMeta,
    item: Option<(&str, &serde_json::Value)>,
) -> Value {
    let mut ctx = BTreeMap::new();

    // 1. Global data.
    for (k, v) in global_data {
        ctx.insert(k.clone(), Value::from_serialize(v));
    }

    // 2. Page-specific data (overrides global data on conflict).
    for (k, v) in page_data {
        if global_data.contains_key(k) {
            tracing::warn!(
                "Frontmatter data key '{}' overrides global data from _data/. \
                 The global data for '{}' will not be available in this template.",
                k, k,
            );
        }
        ctx.insert(k.clone(), Value::from_serialize(v));
    }

    // 3. Page metadata.
    ctx.insert(
        "page".to_string(),
        Value::from_iter([
            ("current_url", Value::from(page_meta.current_url)),
            ("current_path", Value::from(page_meta.current_path)),
            ("base_url", Value::from(page_meta.base_url)),
            ("build_time", Value::from(page_meta.build_time)),
        ]),
    );

    // 4. Dynamic item.
    if let Some((name, val)) = item {
        ctx.insert(name.to_string(), Value::from_serialize(val));
    }

    Value::from_iter(ctx.into_iter())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BuildConfig, SiteMeta};
    use std::collections::HashMap;

    fn test_config() -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://test.com".into(),
            },
            build: BuildConfig::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
        }
    }

    fn test_page_meta() -> PageMeta {
        PageMeta {
            current_url: "/about.html".into(),
            current_path: "about.html".into(),
            base_url: "https://test.com".into(),
            build_time: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn test_build_context_basic() {
        let config = test_config();
        let global = HashMap::new();
        let page_data = HashMap::new();
        let meta = test_page_meta();

        let ctx = build_page_context(&config, &global, &page_data, meta, None);

        // Should have page metadata.
        let page = ctx.get_attr("page").unwrap();
        let current_url = page.get_attr("current_url").unwrap();
        // minijinja Value::to_string() on a string returns the string itself (no extra quotes).
        // But as_str() is more reliable for extracting the raw string.
        assert_eq!(current_url.as_str(), Some("/about.html"));
    }

    #[test]
    fn test_build_context_with_global_data() {
        let config = test_config();
        let mut global = HashMap::new();
        global.insert("nav".into(), serde_json::json!([{"label": "Home"}]));
        let page_data = HashMap::new();
        let meta = test_page_meta();

        let ctx = build_page_context(&config, &global, &page_data, meta, None);
        assert!(ctx.get_attr("nav").is_ok());
    }

    #[test]
    fn test_build_context_page_data_overrides_global() {
        let config = test_config();
        let mut global = HashMap::new();
        global.insert("data".into(), serde_json::json!("global"));
        let mut page_data = HashMap::new();
        page_data.insert("data".into(), serde_json::json!("page"));
        let meta = test_page_meta();

        let ctx = build_page_context(&config, &global, &page_data, meta, None);
        let data_val = ctx.get_attr("data").unwrap();
        assert_eq!(data_val.as_str(), Some("page"));
    }

    #[test]
    fn test_build_context_with_item() {
        let config = test_config();
        let global = HashMap::new();
        let page_data = HashMap::new();
        let meta = test_page_meta();
        let item = serde_json::json!({"title": "My Post", "slug": "my-post"});

        let ctx = build_page_context(&config, &global, &page_data, meta, Some(("post", &item)));

        let post = ctx.get_attr("post").unwrap();
        assert!(post.get_attr("title").is_ok());
    }
}
