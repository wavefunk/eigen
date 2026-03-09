//! Strapi plugin: tames Strapi v4/v5's deeply nested JSON responses.
//!
//! # What it does
//!
//! Strapi wraps every item in `{ "id": N, "attributes": { ...fields } }`.
//! This plugin flattens that structure so `id` and all `attributes` fields
//! live at the same level.  It also resolves nested media/relation wrappers.
//!
//! Before:
//! ```json
//! {
//!   "id": 1,
//!   "attributes": {
//!     "title": "Hello",
//!     "cover": {
//!       "data": {
//!         "id": 10,
//!         "attributes": {
//!           "url": "/uploads/photo.jpg",
//!           "formats": { "thumbnail": { "url": "/uploads/thumb.jpg" } }
//!         }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! After:
//! ```json
//! {
//!   "id": 1,
//!   "title": "Hello",
//!   "cover": {
//!     "id": 10,
//!     "url": "http://localhost:1337/uploads/photo.jpg",
//!     "formats": { "thumbnail": { "url": "http://localhost:1337/uploads/thumb.jpg" } }
//!   }
//! }
//! ```
//!
//! # Configuration
//!
//! ```toml
//! [plugins.strapi]
//! # Which source(s) from [sources.*] this plugin should process.
//! # Default: all sources whose name contains "strapi".
//! sources = ["strapi"]
//!
//! # Prepend this to media URLs that start with "/uploads/".
//! # Required for asset localization to download them.
//! media_base_url = "http://localhost:1337"
//! ```

use eyre::Result;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::path::Path;

use super::Plugin;

#[derive(Debug)]
pub struct StrapiPlugin {
    config: StrapiConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct StrapiConfig {
    /// Source names this plugin applies to.
    #[serde(default)]
    sources: Vec<String>,
    /// Base URL to prepend to relative media paths (e.g., "/uploads/...").
    #[serde(default)]
    media_base_url: Option<String>,
}

impl Default for StrapiConfig {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            media_base_url: None,
        }
    }
}

impl StrapiPlugin {
    pub fn new() -> Self {
        Self {
            config: StrapiConfig::default(),
        }
    }

    /// Check whether this plugin should process data from the given source.
    fn should_process(&self, source_name: Option<&str>) -> bool {
        let Some(name) = source_name else {
            return false; // Don't process local files.
        };

        if self.config.sources.is_empty() {
            // Default heuristic: process if the source name contains "strapi".
            name.to_lowercase().contains("strapi")
        } else {
            self.config.sources.iter().any(|s| s == name)
        }
    }
}

impl Plugin for StrapiPlugin {
    fn name(&self) -> &str {
        "strapi"
    }

    fn on_config_loaded(
        &mut self,
        plugin_config: Option<&toml::Value>,
        _project_root: &Path,
    ) -> Result<()> {
        if let Some(config) = plugin_config {
            self.config = config
                .clone()
                .try_into()
                .map_err(|e| eyre::eyre!("Invalid [plugins.strapi] config: {}", e))?;
        }

        if let Some(ref url) = self.config.media_base_url {
            tracing::info!("  Strapi plugin: media_base_url = {}", url);
        }

        Ok(())
    }

    fn transform_data(
        &self,
        value: Value,
        source_name: Option<&str>,
        _query_path: Option<&str>,
    ) -> Result<Value> {
        if !self.should_process(source_name) {
            return Ok(value);
        }

        Ok(flatten_strapi_value(
            value,
            self.config.media_base_url.as_deref(),
        ))
    }

    fn register_template_extensions(
        &self,
        env: &mut minijinja::Environment<'_>,
    ) -> Result<()> {
        // Add a `strapi_media(url)` function that prepends the media base URL.
        let base = self.config.media_base_url.clone().unwrap_or_default();
        env.add_function("strapi_media", move |path: &str| -> String {
            if path.starts_with("http://") || path.starts_with("https://") {
                path.to_string()
            } else {
                format!("{}{}", base, path)
            }
        });

        Ok(())
    }
}

/// Recursively flatten Strapi's nested structure.
///
/// Handles:
/// - `{ "id": N, "attributes": { ... } }` → merge attributes up
/// - `{ "data": { "id": N, "attributes": { ... } } }` → unwrap single relation
/// - `{ "data": [ ... ] }` → unwrap relation collection
/// - Relative URLs in `"url"` fields → prepend media_base_url
fn flatten_strapi_value(value: Value, media_base_url: Option<&str>) -> Value {
    match value {
        Value::Array(arr) => {
            Value::Array(
                arr.into_iter()
                    .map(|v| flatten_strapi_value(v, media_base_url))
                    .collect(),
            )
        }
        Value::Object(mut map) => {
            // Pattern 1: { "id": ..., "attributes": { ... } }
            // Merge attributes into the parent, keeping id.
            if map.contains_key("attributes") {
                if let Some(Value::Object(attrs)) = map.remove("attributes") {
                    for (k, v) in attrs {
                        // Don't overwrite `id` if it already exists.
                        if k != "id" || !map.contains_key("id") {
                            map.insert(k, v);
                        }
                    }
                }
            }

            // Pattern 2: { "data": <value> } where data is a relation wrapper.
            // Only unwrap if this looks like a Strapi relation (data + no other
            // content keys, or data is an object with id/attributes).
            if let Some(data) = map.get("data") {
                let is_relation_wrapper = is_strapi_relation_wrapper(&map);
                if is_relation_wrapper {
                    let data = map.remove("data").unwrap();
                    return flatten_strapi_value(data, media_base_url);
                }
            }

            // Recurse into all remaining fields.
            let mut result = Map::new();
            for (k, v) in map {
                let flattened = flatten_strapi_value(v, media_base_url);

                // Rewrite relative media URLs.
                if k == "url" {
                    if let Some(base) = media_base_url {
                        if let Value::String(ref s) = flattened {
                            if s.starts_with("/uploads/") || s.starts_with("/uploads\\") {
                                result.insert(
                                    k,
                                    Value::String(format!(
                                        "{}{}",
                                        base.trim_end_matches('/'),
                                        s
                                    )),
                                );
                                continue;
                            }
                        }
                    }
                }

                result.insert(k, flattened);
            }

            Value::Object(result)
        }
        other => other,
    }
}

/// Heuristic: does this object look like a Strapi relation wrapper?
///
/// A relation wrapper is `{ "data": ... }` where `data` is either:
/// - An object with `id` and/or `attributes`
/// - An array of such objects
/// - `null` (empty relation)
///
/// We also check that the object has no other "content" keys (only "data"
/// and possibly "meta").
fn is_strapi_relation_wrapper(map: &Map<String, Value>) -> bool {
    // Must have "data" key.
    let Some(data) = map.get("data") else {
        return false;
    };

    // The only other allowed key is "meta" (Strapi pagination metadata).
    let non_data_keys: Vec<&String> = map
        .keys()
        .filter(|k| *k != "data" && *k != "meta")
        .collect();
    if !non_data_keys.is_empty() {
        return false;
    }

    match data {
        Value::Null => true,
        Value::Object(obj) => obj.contains_key("id") || obj.contains_key("attributes"),
        Value::Array(arr) => {
            // Check first item.
            arr.first()
                .and_then(|v| v.as_object())
                .map(|obj| obj.contains_key("id") || obj.contains_key("attributes"))
                .unwrap_or(true) // empty array is fine
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // flatten_strapi_value — basic cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_flatten_single_item() {
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Hello",
                "body": "World"
            }
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["id"], 1);
        assert_eq!(result["title"], "Hello");
        assert_eq!(result["body"], "World");
        assert!(result.get("attributes").is_none());
    }

    #[test]
    fn test_flatten_array_of_items() {
        let input = json!([
            {"id": 1, "attributes": {"title": "First"}},
            {"id": 2, "attributes": {"title": "Second"}},
        ]);

        let result = flatten_strapi_value(input, None);
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[0]["title"], "First");
        assert_eq!(arr[1]["id"], 2);
        assert_eq!(arr[1]["title"], "Second");
    }

    #[test]
    fn test_flatten_preserves_id() {
        // If both the outer object and attributes have an "id", keep the outer one.
        let input = json!({
            "id": 42,
            "attributes": {
                "id": 999,
                "title": "Test"
            }
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["id"], 42);
        assert_eq!(result["title"], "Test");
    }

    #[test]
    fn test_flatten_empty_attributes() {
        let input = json!({
            "id": 1,
            "attributes": {}
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["id"], 1);
        assert!(result.get("attributes").is_none());
    }

    #[test]
    fn test_flatten_no_attributes_field() {
        // Object without "attributes" — should pass through unchanged.
        let input = json!({"id": 1, "title": "Direct"});
        let result = flatten_strapi_value(input, None);
        assert_eq!(result["id"], 1);
        assert_eq!(result["title"], "Direct");
    }

    #[test]
    fn test_flatten_empty_array() {
        let input = json!([]);
        let result = flatten_strapi_value(input, None);
        assert_eq!(result, json!([]));
    }

    #[test]
    fn test_flatten_scalar_values_passthrough() {
        assert_eq!(flatten_strapi_value(json!(42), None), json!(42));
        assert_eq!(flatten_strapi_value(json!("hello"), None), json!("hello"));
        assert_eq!(flatten_strapi_value(json!(true), None), json!(true));
        assert_eq!(flatten_strapi_value(json!(null), None), json!(null));
    }

    // -----------------------------------------------------------------------
    // flatten_strapi_value — relations
    // -----------------------------------------------------------------------

    #[test]
    fn test_flatten_nested_relation() {
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Post",
                "author": {
                    "data": {
                        "id": 5,
                        "attributes": {
                            "name": "Alice"
                        }
                    }
                }
            }
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["title"], "Post");
        assert_eq!(result["author"]["id"], 5);
        assert_eq!(result["author"]["name"], "Alice");
    }

    #[test]
    fn test_flatten_null_relation() {
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Post",
                "author": {
                    "data": null
                }
            }
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["title"], "Post");
        assert!(result["author"].is_null());
    }

    #[test]
    fn test_flatten_relation_collection() {
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Post",
                "tags": {
                    "data": [
                        {"id": 1, "attributes": {"name": "rust"}},
                        {"id": 2, "attributes": {"name": "web"}},
                    ]
                }
            }
        });

        let result = flatten_strapi_value(input, None);
        let tags = result["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0]["name"], "rust");
        assert_eq!(tags[0]["id"], 1);
        assert_eq!(tags[1]["name"], "web");
        assert_eq!(tags[1]["id"], 2);
    }

    #[test]
    fn test_flatten_empty_relation_collection() {
        let input = json!({
            "id": 1,
            "attributes": {
                "tags": {
                    "data": []
                }
            }
        });

        let result = flatten_strapi_value(input, None);
        let tags = result["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 0);
    }

    #[test]
    fn test_flatten_relation_with_meta() {
        // Strapi pagination metadata should be stripped along with data wrapper.
        let input = json!({
            "id": 1,
            "attributes": {
                "comments": {
                    "data": [
                        {"id": 10, "attributes": {"text": "Great!"}}
                    ],
                    "meta": {"pagination": {"total": 1}}
                }
            }
        });

        let result = flatten_strapi_value(input, None);
        let comments = result["comments"].as_array().unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0]["text"], "Great!");
    }

    #[test]
    fn test_flatten_non_strapi_data_field_preserved() {
        // Object with "data" that is a plain string — not a relation.
        let input = json!({
            "title": "Normal",
            "data": "this is content, not a relation"
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["data"], "this is content, not a relation");
    }

    #[test]
    fn test_flatten_data_field_with_extra_keys_not_a_relation() {
        // {"data": ..., "someOtherKey": ...} — not just data+meta, so not a relation.
        let input = json!({
            "data": {"id": 1, "attributes": {"name": "x"}},
            "extra": "stuff"
        });

        let result = flatten_strapi_value(input, None);
        // Should NOT unwrap data as a relation because "extra" is present.
        assert!(result.get("data").is_some());
        assert_eq!(result["extra"], "stuff");
    }

    // -----------------------------------------------------------------------
    // flatten_strapi_value — deeply nested
    // -----------------------------------------------------------------------

    #[test]
    fn test_flatten_deeply_nested_relations() {
        // Post -> author -> avatar (three levels deep).
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Deep Post",
                "author": {
                    "data": {
                        "id": 5,
                        "attributes": {
                            "name": "Alice",
                            "avatar": {
                                "data": {
                                    "id": 20,
                                    "attributes": {
                                        "url": "/uploads/avatar.jpg"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let result = flatten_strapi_value(input, Some("http://cms.example.com"));
        assert_eq!(result["title"], "Deep Post");
        assert_eq!(result["author"]["name"], "Alice");
        assert_eq!(
            result["author"]["avatar"]["url"],
            "http://cms.example.com/uploads/avatar.jpg"
        );
    }

    // -----------------------------------------------------------------------
    // flatten_strapi_value — media URL rewriting
    // -----------------------------------------------------------------------

    #[test]
    fn test_flatten_media_with_base_url() {
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Post",
                "cover": {
                    "data": {
                        "id": 10,
                        "attributes": {
                            "url": "/uploads/photo.jpg",
                            "formats": {
                                "thumbnail": {
                                    "url": "/uploads/thumb.jpg"
                                }
                            }
                        }
                    }
                }
            }
        });

        let result = flatten_strapi_value(input, Some("http://localhost:1337"));
        assert_eq!(result["title"], "Post");
        assert_eq!(result["cover"]["id"], 10);
        assert_eq!(
            result["cover"]["url"],
            "http://localhost:1337/uploads/photo.jpg"
        );
        assert_eq!(
            result["cover"]["formats"]["thumbnail"]["url"],
            "http://localhost:1337/uploads/thumb.jpg"
        );
    }

    #[test]
    fn test_flatten_media_without_base_url() {
        let input = json!({
            "id": 1,
            "attributes": {
                "cover": {
                    "data": {
                        "id": 10,
                        "attributes": {
                            "url": "/uploads/photo.jpg"
                        }
                    }
                }
            }
        });

        let result = flatten_strapi_value(input, None);
        // Without base URL, the relative path stays as-is.
        assert_eq!(result["cover"]["url"], "/uploads/photo.jpg");
    }

    #[test]
    fn test_flatten_media_base_url_trailing_slash_stripped() {
        let input = json!({
            "url": "/uploads/photo.jpg"
        });

        let result = flatten_strapi_value(input, Some("http://localhost:1337/"));
        assert_eq!(
            result["url"],
            "http://localhost:1337/uploads/photo.jpg"
        );
    }

    #[test]
    fn test_flatten_media_absolute_url_not_rewritten() {
        // URLs that don't start with /uploads/ should NOT be rewritten.
        let input = json!({
            "url": "https://cdn.example.com/photo.jpg"
        });

        let result = flatten_strapi_value(input, Some("http://localhost:1337"));
        assert_eq!(result["url"], "https://cdn.example.com/photo.jpg");
    }

    #[test]
    fn test_flatten_non_upload_relative_url_not_rewritten() {
        let input = json!({
            "url": "/api/something"
        });

        let result = flatten_strapi_value(input, Some("http://localhost:1337"));
        // /api/something doesn't start with /uploads/, so leave it alone.
        assert_eq!(result["url"], "/api/something");
    }

    // -----------------------------------------------------------------------
    // should_process
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_process_default_heuristic() {
        let plugin = StrapiPlugin::new();
        assert!(plugin.should_process(Some("strapi")));
        assert!(plugin.should_process(Some("my_strapi")));
        assert!(plugin.should_process(Some("Strapi_CMS")));
        assert!(!plugin.should_process(Some("wordpress")));
        assert!(!plugin.should_process(None));
    }

    #[test]
    fn test_should_process_explicit_sources() {
        let mut plugin = StrapiPlugin::new();
        plugin.config.sources = vec!["cms".to_string()];

        assert!(plugin.should_process(Some("cms")));
        assert!(!plugin.should_process(Some("strapi"))); // heuristic disabled
        assert!(!plugin.should_process(Some("other")));
    }

    #[test]
    fn test_should_process_multiple_explicit_sources() {
        let mut plugin = StrapiPlugin::new();
        plugin.config.sources = vec!["cms".to_string(), "backend".to_string()];

        assert!(plugin.should_process(Some("cms")));
        assert!(plugin.should_process(Some("backend")));
        assert!(!plugin.should_process(Some("other")));
    }

    // -----------------------------------------------------------------------
    // Plugin trait — on_config_loaded
    // -----------------------------------------------------------------------

    #[test]
    fn test_on_config_loaded_parses_config() {
        let toml_str = r#"
            sources = ["cms"]
            media_base_url = "http://localhost:1337"
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();

        let mut plugin = StrapiPlugin::new();
        plugin
            .on_config_loaded(Some(&value), Path::new("/tmp"))
            .unwrap();

        assert_eq!(plugin.config.sources, vec!["cms"]);
        assert_eq!(
            plugin.config.media_base_url.as_deref(),
            Some("http://localhost:1337")
        );
    }

    #[test]
    fn test_on_config_loaded_no_config() {
        let mut plugin = StrapiPlugin::new();
        plugin.on_config_loaded(None, Path::new("/tmp")).unwrap();
        // Defaults are preserved.
        assert!(plugin.config.sources.is_empty());
        assert!(plugin.config.media_base_url.is_none());
    }

    #[test]
    fn test_on_config_loaded_invalid_config() {
        let value = toml::Value::String("not a table".into());
        let mut plugin = StrapiPlugin::new();
        let result = plugin.on_config_loaded(Some(&value), Path::new("/tmp"));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Plugin trait — transform_data skips non-matching sources
    // -----------------------------------------------------------------------

    #[test]
    fn test_transform_data_skips_non_matching_source() {
        let plugin = StrapiPlugin::new(); // default heuristic: requires "strapi" in name
        let input = json!([{"id": 1, "attributes": {"title": "Test"}}]);

        // Non-strapi source — data passes through unchanged.
        let result = plugin
            .transform_data(input.clone(), Some("wordpress"), None)
            .unwrap();
        assert!(result[0].get("attributes").is_some());
    }

    #[test]
    fn test_transform_data_processes_matching_source() {
        let plugin = StrapiPlugin::new();
        let input = json!([{"id": 1, "attributes": {"title": "Test"}}]);

        let result = plugin
            .transform_data(input, Some("strapi"), None)
            .unwrap();
        assert_eq!(result[0]["title"], "Test");
        assert!(result[0].get("attributes").is_none());
    }

    #[test]
    fn test_transform_data_skips_local_files() {
        let plugin = StrapiPlugin::new();
        let input = json!([{"id": 1, "attributes": {"title": "Test"}}]);

        // source_name = None means local file.
        let result = plugin.transform_data(input.clone(), None, None).unwrap();
        assert!(result[0].get("attributes").is_some());
    }

    // -----------------------------------------------------------------------
    // Plugin trait — register_template_extensions
    // -----------------------------------------------------------------------

    #[test]
    fn test_strapi_media_function_with_base_url() {
        let mut plugin = StrapiPlugin::new();
        plugin.config.media_base_url = Some("http://localhost:1337".into());

        let mut env = minijinja::Environment::new();
        plugin.register_template_extensions(&mut env).unwrap();

        env.add_template("test", "{{ strapi_media('/uploads/photo.jpg') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(minijinja::context! {}).unwrap();
        assert_eq!(result, "http://localhost:1337/uploads/photo.jpg");
    }

    #[test]
    fn test_strapi_media_function_absolute_url_passthrough() {
        let mut plugin = StrapiPlugin::new();
        plugin.config.media_base_url = Some("http://localhost:1337".into());

        let mut env = minijinja::Environment::new();
        plugin.register_template_extensions(&mut env).unwrap();

        env.add_template("test", "{{ strapi_media('https://cdn.example.com/photo.jpg') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(minijinja::context! {}).unwrap();
        assert_eq!(result, "https://cdn.example.com/photo.jpg");
    }

    #[test]
    fn test_strapi_media_function_no_base_url() {
        let plugin = StrapiPlugin::new(); // no media_base_url

        let mut env = minijinja::Environment::new();
        plugin.register_template_extensions(&mut env).unwrap();

        env.add_template("test", "{{ strapi_media('/uploads/photo.jpg') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(minijinja::context! {}).unwrap();
        // Empty base + path.
        assert_eq!(result, "/uploads/photo.jpg");
    }

    // -----------------------------------------------------------------------
    // Full Strapi v4 response simulation
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_strapi_response() {
        let input = json!([
            {
                "id": 1,
                "attributes": {
                    "title": "First Post",
                    "slug": "first-post",
                    "body": "Hello world",
                    "publishedAt": "2025-01-15T10:00:00.000Z",
                    "cover": {
                        "data": {
                            "id": 100,
                            "attributes": {
                                "name": "cover.jpg",
                                "url": "/uploads/cover_abc123.jpg",
                                "formats": {
                                    "thumbnail": {
                                        "url": "/uploads/thumbnail_cover_abc123.jpg",
                                        "width": 156,
                                        "height": 156
                                    },
                                    "small": {
                                        "url": "/uploads/small_cover_abc123.jpg",
                                        "width": 500,
                                        "height": 333
                                    }
                                }
                            }
                        }
                    }
                }
            },
            {
                "id": 2,
                "attributes": {
                    "title": "Second Post",
                    "slug": "second-post",
                    "body": "Another post",
                    "publishedAt": "2025-01-16T12:00:00.000Z",
                    "cover": {
                        "data": null
                    }
                }
            }
        ]);

        let result = flatten_strapi_value(input, Some("https://cms.example.com"));
        let arr = result.as_array().unwrap();

        // First post — fully flattened.
        let p1 = &arr[0];
        assert_eq!(p1["id"], 1);
        assert_eq!(p1["title"], "First Post");
        assert_eq!(p1["slug"], "first-post");
        assert_eq!(p1["body"], "Hello world");
        assert_eq!(
            p1["cover"]["url"],
            "https://cms.example.com/uploads/cover_abc123.jpg"
        );
        assert_eq!(
            p1["cover"]["formats"]["thumbnail"]["url"],
            "https://cms.example.com/uploads/thumbnail_cover_abc123.jpg"
        );
        assert_eq!(p1["cover"]["formats"]["thumbnail"]["width"], 156);

        // Second post — null cover relation.
        let p2 = &arr[1];
        assert_eq!(p2["id"], 2);
        assert_eq!(p2["title"], "Second Post");
        assert!(p2["cover"].is_null());
    }

    #[test]
    fn test_full_strapi_single_type_response() {
        // Strapi single types (like homepage) have: {"data": {"id": 1, "attributes": {...}}}
        // After root: data extraction, we get just the inner object.
        let input = json!({
            "id": 1,
            "attributes": {
                "heroTitle": "Welcome",
                "heroSubtitle": "To my site",
                "heroImage": {
                    "data": {
                        "id": 50,
                        "attributes": {
                            "url": "/uploads/hero.jpg",
                            "alternativeText": "Hero image"
                        }
                    }
                }
            }
        });

        let result = flatten_strapi_value(input, Some("http://cms.local"));
        assert_eq!(result["id"], 1);
        assert_eq!(result["heroTitle"], "Welcome");
        assert_eq!(result["heroSubtitle"], "To my site");
        assert_eq!(result["heroImage"]["url"], "http://cms.local/uploads/hero.jpg");
        assert_eq!(result["heroImage"]["alternativeText"], "Hero image");
    }

    #[test]
    fn test_strapi_component_field() {
        // Strapi components are just nested objects without data/attributes wrapper.
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Post",
                "seo": {
                    "metaTitle": "Post | Blog",
                    "metaDescription": "A blog post"
                }
            }
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["title"], "Post");
        assert_eq!(result["seo"]["metaTitle"], "Post | Blog");
        assert_eq!(result["seo"]["metaDescription"], "A blog post");
    }

    #[test]
    fn test_strapi_repeatable_component() {
        // Repeatable components come as plain arrays (no data wrapper).
        let input = json!({
            "id": 1,
            "attributes": {
                "title": "Page",
                "sections": [
                    {"__component": "page.hero", "heading": "Welcome"},
                    {"__component": "page.text", "body": "Hello"}
                ]
            }
        });

        let result = flatten_strapi_value(input, None);
        assert_eq!(result["title"], "Page");
        let sections = result["sections"].as_array().unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0]["heading"], "Welcome");
        assert_eq!(sections[1]["body"], "Hello");
    }

    // -----------------------------------------------------------------------
    // is_strapi_relation_wrapper
    // -----------------------------------------------------------------------

    #[test]
    fn test_relation_wrapper_null_data() {
        let map: Map<String, Value> =
            serde_json::from_value(json!({"data": null})).unwrap();
        assert!(is_strapi_relation_wrapper(&map));
    }

    #[test]
    fn test_relation_wrapper_object_with_id() {
        let map: Map<String, Value> =
            serde_json::from_value(json!({"data": {"id": 1}})).unwrap();
        assert!(is_strapi_relation_wrapper(&map));
    }

    #[test]
    fn test_relation_wrapper_object_with_attributes() {
        let map: Map<String, Value> = serde_json::from_value(
            json!({"data": {"attributes": {"name": "test"}}}),
        )
        .unwrap();
        assert!(is_strapi_relation_wrapper(&map));
    }

    #[test]
    fn test_relation_wrapper_array_of_objects_with_id() {
        let map: Map<String, Value> = serde_json::from_value(
            json!({"data": [{"id": 1}, {"id": 2}]}),
        )
        .unwrap();
        assert!(is_strapi_relation_wrapper(&map));
    }

    #[test]
    fn test_not_relation_wrapper_string_data() {
        let map: Map<String, Value> =
            serde_json::from_value(json!({"data": "plain string"})).unwrap();
        assert!(!is_strapi_relation_wrapper(&map));
    }

    #[test]
    fn test_not_relation_wrapper_extra_keys() {
        let map: Map<String, Value> = serde_json::from_value(
            json!({"data": {"id": 1}, "extra": "value"}),
        )
        .unwrap();
        assert!(!is_strapi_relation_wrapper(&map));
    }

    #[test]
    fn test_relation_wrapper_data_plus_meta_is_ok() {
        let map: Map<String, Value> = serde_json::from_value(
            json!({"data": [{"id": 1}], "meta": {"pagination": {"total": 1}}}),
        )
        .unwrap();
        assert!(is_strapi_relation_wrapper(&map));
    }
}
