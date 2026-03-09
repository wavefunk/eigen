//! Steps 3.3 & 3.4: Nested query interpolation and the high-level data query
//! executor.
//!
//! - **Nested query interpolation**: resolve `{{ item.field }}` patterns in
//!   `DataQuery.filter` values when rendering dynamic pages.
//! - **Query executor**: the main entry point that takes a `Frontmatter` and
//!   an optional current item, resolves all `data` entries, and returns a
//!   context map.

use eyre::{Result, WrapErr, bail};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

use crate::frontmatter::{DataQuery, Frontmatter};
use crate::plugins::registry::PluginRegistry;

use super::fetcher::DataFetcher;

/// Resolve all `data` queries in a static page's frontmatter.
///
/// Returns a map of query name → resolved value, ready to be merged into the
/// template context.
pub fn resolve_page_data(
    frontmatter: &Frontmatter,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
) -> Result<HashMap<String, Value>> {
    let mut result = HashMap::new();

    for (name, query) in &frontmatter.data {
        let value = fetcher
            .fetch(query, plugin_registry)
            .wrap_err_with(|| format!("Failed to resolve data query '{}'", name))?;
        result.insert(name.clone(), value);
    }

    Ok(result)
}

/// Resolve all data for a dynamic page: fetch the collection first, then for
/// each item, resolve the `data` queries (with interpolation).
///
/// Returns a tuple of:
/// - The collection items (a `Vec<Value>`)
/// - A function-like closure isn't easy here, so instead we return the raw
///   collection and let the caller iterate, calling `resolve_item_data` for
///   each item.
pub fn resolve_dynamic_page_data(
    frontmatter: &Frontmatter,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
) -> Result<Vec<Value>> {
    let collection_query = frontmatter
        .collection
        .as_ref()
        .ok_or_else(|| eyre::eyre!("Dynamic page has no `collection` in frontmatter"))?;

    let collection = fetcher
        .fetch(collection_query, plugin_registry)
        .wrap_err("Failed to fetch collection")?;

    match collection {
        Value::Array(items) => Ok(items),
        _ => {
            // Not an array — return empty. The build engine will skip silently.
            Ok(Vec::new())
        }
    }
}

/// Resolve the `data` queries for a single item of a dynamic page.
///
/// Filter values containing `{{ item.field }}` patterns are interpolated using
/// the current item's data. Interpolation is ONE level deep — if an
/// interpolated query itself references `{{ }}`, an error is returned.
pub fn resolve_item_data(
    frontmatter: &Frontmatter,
    item: &Value,
    item_as: &str,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
) -> Result<HashMap<String, Value>> {
    let mut result = HashMap::new();

    for (name, query) in &frontmatter.data {
        let interpolated = interpolate_query(query, item, item_as)
            .wrap_err_with(|| {
                format!(
                    "Failed to interpolate data query '{}' for current item",
                    name,
                )
            })?;

        // Verify the interpolated query doesn't still contain {{ }} patterns.
        verify_no_remaining_interpolation(&interpolated, name)?;

        let value = fetcher
            .fetch(&interpolated, plugin_registry)
            .wrap_err_with(|| format!("Failed to resolve data query '{}'", name))?;
        result.insert(name.clone(), value);
    }

    Ok(result)
}

/// Convenience wrapper: resolve data queries for a single item of a dynamic page.
///
/// Uses the frontmatter's `item_as` field as the interpolation prefix.
pub fn resolve_dynamic_page_data_for_item(
    frontmatter: &Frontmatter,
    item: &Value,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
) -> Result<HashMap<String, Value>> {
    resolve_item_data(frontmatter, item, &frontmatter.item_as, fetcher, plugin_registry)
}

/// Interpolate `{{ item_as.field }}` patterns in a DataQuery's filter values.
///
/// Given an item `{"author_id": 42}` and `item_as = "post"`, a filter value
/// of `"{{ post.author_id }}"` becomes `"42"`.
fn interpolate_query(query: &DataQuery, item: &Value, item_as: &str) -> Result<DataQuery> {
    let new_filter = match &query.filter {
        Some(filters) => {
            let mut interpolated = HashMap::new();
            for (key, value_template) in filters {
                let resolved = interpolate_string(value_template, item, item_as)?;
                interpolated.insert(key.clone(), resolved);
            }
            Some(interpolated)
        }
        None => None,
    };

    // Also interpolate `path` in case it contains item references.
    let new_path = match &query.path {
        Some(path_template) => Some(interpolate_string(path_template, item, item_as)?),
        None => query.path.clone(),
    };

    Ok(DataQuery {
        file: query.file.clone(),
        source: query.source.clone(),
        path: new_path,
        root: query.root.clone(),
        sort: query.sort.clone(),
        limit: query.limit,
        filter: new_filter,
    })
}

/// Replace all `{{ item_as.field.subfield }}` patterns in a string with the
/// corresponding value from the item.
fn interpolate_string(template: &str, item: &Value, item_as: &str) -> Result<String> {
    let re = Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}\}").unwrap();

    let mut result = template.to_string();
    let captures: Vec<(String, String)> = re
        .captures_iter(template)
        .map(|cap| (cap[0].to_string(), cap[1].to_string()))
        .collect();

    for (full_match, path) in captures {
        let value = resolve_item_path(&path, item, item_as)?;
        let replacement = value_to_string(&value);
        result = result.replace(&full_match, &replacement);
    }

    Ok(result)
}

/// Resolve a dot-separated path like `"post.author_id"` against the current item.
///
/// The first segment must match `item_as` (e.g., `"post"`). The remaining
/// segments walk into the item's value.
fn resolve_item_path(path: &str, item: &Value, item_as: &str) -> Result<Value> {
    let segments: Vec<&str> = path.split('.').collect();

    if segments.is_empty() {
        bail!("Empty interpolation path");
    }

    if segments[0] != item_as {
        bail!(
            "Interpolation path '{}' does not start with '{}'. \
             In dynamic page data queries, interpolation paths must begin \
             with the item_as name.",
            path,
            item_as,
        );
    }

    let mut current = item;
    for &segment in &segments[1..] {
        match current.get(segment) {
            Some(inner) => current = inner,
            None => bail!(
                "Interpolation path '{}': field '{}' not found in item. \
                 Available fields: {}",
                path,
                segment,
                available_fields(current),
            ),
        }
    }

    Ok(current.clone())
}

/// Convert a JSON value to a string for interpolation.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        // For complex types, use JSON serialization.
        other => other.to_string(),
    }
}

/// List available fields in a JSON value (for error messages).
fn available_fields(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                "(empty object)".to_string()
            } else {
                map.keys().cloned().collect::<Vec<_>>().join(", ")
            }
        }
        _ => format!("(value is {}, not an object)", type_name(value)),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Verify that an interpolated DataQuery has no remaining `{{ }}` patterns.
/// This prevents accidental multi-level interpolation.
fn verify_no_remaining_interpolation(query: &DataQuery, query_name: &str) -> Result<()> {
    let re = Regex::new(r"\{\{.*?\}\}").unwrap();

    // Check filter values.
    if let Some(ref filters) = query.filter {
        for (key, value) in filters {
            if re.is_match(value) {
                bail!(
                    "Data query '{}' still contains interpolation pattern in \
                     filter key '{}' after resolution: \"{}\". \
                     Nested interpolation (more than one level) is not supported.",
                    query_name,
                    key,
                    value,
                );
            }
        }
    }

    // Check path.
    if let Some(ref path) = query.path {
        if re.is_match(path) {
            bail!(
                "Data query '{}' still contains interpolation pattern in \
                 path after resolution: \"{}\". \
                 Nested interpolation (more than one level) is not supported.",
                query_name,
                path,
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to write a file.
    fn write(dir: &std::path::Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // --- interpolate_string tests ---

    #[test]
    fn test_interpolate_simple() {
        let item = json!({"author_id": 42});
        let result = interpolate_string("{{ post.author_id }}", &item, "post").unwrap();
        assert_eq!(result, "42");
    }

    #[test]
    fn test_interpolate_string_value() {
        let item = json!({"slug": "hello-world"});
        let result = interpolate_string("{{ post.slug }}", &item, "post").unwrap();
        assert_eq!(result, "hello-world");
    }

    #[test]
    fn test_interpolate_in_path() {
        let item = json!({"id": 7});
        let result = interpolate_string("/authors/{{ post.id }}/bio", &item, "post").unwrap();
        assert_eq!(result, "/authors/7/bio");
    }

    #[test]
    fn test_interpolate_multiple() {
        let item = json!({"first": "John", "last": "Doe"});
        let result =
            interpolate_string("{{ person.first }}-{{ person.last }}", &item, "person").unwrap();
        assert_eq!(result, "John-Doe");
    }

    #[test]
    fn test_interpolate_nested_field() {
        let item = json!({"meta": {"author_id": 99}});
        let result = interpolate_string("{{ post.meta.author_id }}", &item, "post").unwrap();
        assert_eq!(result, "99");
    }

    #[test]
    fn test_interpolate_wrong_prefix() {
        let item = json!({"id": 1});
        let result = interpolate_string("{{ wrong.id }}", &item, "post");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("wrong"));
        assert!(err.contains("post"));
    }

    #[test]
    fn test_interpolate_missing_field() {
        let item = json!({"id": 1});
        let result = interpolate_string("{{ post.nonexistent }}", &item, "post");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent"));
    }

    #[test]
    fn test_interpolate_no_patterns() {
        let item = json!({"id": 1});
        let result = interpolate_string("plain string", &item, "post").unwrap();
        assert_eq!(result, "plain string");
    }

    // --- interpolate_query tests ---

    #[test]
    fn test_interpolate_query_filter() {
        let item = json!({"author_id": 42});
        let query = DataQuery {
            source: Some("api".into()),
            path: Some("/authors".into()),
            filter: Some({
                let mut m = HashMap::new();
                m.insert("id".into(), "{{ post.author_id }}".into());
                m
            }),
            ..Default::default()
        };

        let result = interpolate_query(&query, &item, "post").unwrap();
        let filter = result.filter.unwrap();
        assert_eq!(filter["id"], "42");
    }

    #[test]
    fn test_interpolate_query_path() {
        let item = json!({"id": 7});
        let query = DataQuery {
            source: Some("api".into()),
            path: Some("/posts/{{ post.id }}/comments".into()),
            ..Default::default()
        };

        let result = interpolate_query(&query, &item, "post").unwrap();
        assert_eq!(result.path.unwrap(), "/posts/7/comments");
    }

    // --- verify_no_remaining_interpolation tests ---

    #[test]
    fn test_verify_clean_query() {
        let query = DataQuery {
            filter: Some({
                let mut m = HashMap::new();
                m.insert("id".into(), "42".into());
                m
            }),
            path: Some("/posts".into()),
            ..Default::default()
        };
        assert!(verify_no_remaining_interpolation(&query, "test").is_ok());
    }

    #[test]
    fn test_verify_remaining_in_filter() {
        let query = DataQuery {
            filter: Some({
                let mut m = HashMap::new();
                m.insert("id".into(), "{{ nested.ref }}".into());
                m
            }),
            ..Default::default()
        };
        let result = verify_no_remaining_interpolation(&query, "test");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.to_lowercase().contains("nested interpolation"));
    }

    #[test]
    fn test_verify_remaining_in_path() {
        let query = DataQuery {
            path: Some("/posts/{{ nested.id }}".into()),
            ..Default::default()
        };
        let result = verify_no_remaining_interpolation(&query, "test");
        assert!(result.is_err());
    }

    // --- resolve_page_data tests ---

    #[test]
    fn test_resolve_page_data_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/nav.yaml", "- label: Home\n  url: /\n");

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let fm = Frontmatter {
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "nav".into(),
                    DataQuery {
                        file: Some("nav.yaml".into()),
                        ..Default::default()
                    },
                );
                m
            },
            ..Default::default()
        };

        let result = resolve_page_data(&fm, &mut fetcher, None).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result["nav"].is_array());
    }

    #[test]
    fn test_resolve_page_data_multiple() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/nav.yaml", "- label: Home\n  url: /\n");
        write(root, "_data/config.json", r#"{"debug": false}"#);

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let fm = Frontmatter {
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "nav".into(),
                    DataQuery {
                        file: Some("nav.yaml".into()),
                        ..Default::default()
                    },
                );
                m.insert(
                    "config".into(),
                    DataQuery {
                        file: Some("config.json".into()),
                        ..Default::default()
                    },
                );
                m
            },
            ..Default::default()
        };

        let result = resolve_page_data(&fm, &mut fetcher, None).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("nav"));
        assert!(result.contains_key("config"));
    }

    #[test]
    fn test_resolve_page_data_empty() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let fm = Frontmatter::default();

        let result = resolve_page_data(&fm, &mut fetcher, None).unwrap();
        assert!(result.is_empty());
    }

    // --- resolve_dynamic_page_data tests ---

    #[test]
    fn test_resolve_dynamic_collection_from_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/posts.json",
            r#"[{"id": 1, "title": "First"}, {"id": 2, "title": "Second"}]"#,
        );

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let fm = Frontmatter {
            collection: Some(DataQuery {
                file: Some("posts.json".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let items = resolve_dynamic_page_data(&fm, &mut fetcher, None).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["title"], "First");
    }

    #[test]
    fn test_resolve_dynamic_no_collection() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let fm = Frontmatter::default();

        let result = resolve_dynamic_page_data(&fm, &mut fetcher, None);
        assert!(result.is_err());
    }

    // --- resolve_item_data tests ---

    #[test]
    fn test_resolve_item_data_with_interpolation() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/authors.json",
            r#"[{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]"#,
        );

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);

        let fm = Frontmatter {
            item_as: "post".into(),
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "author".into(),
                    DataQuery {
                        file: Some("authors.json".into()),
                        filter: Some({
                            let mut f = HashMap::new();
                            f.insert("id".into(), "{{ post.author_id }}".into());
                            f
                        }),
                        ..Default::default()
                    },
                );
                m
            },
            ..Default::default()
        };

        let item = json!({"author_id": 2, "title": "My Post"});
        let result = resolve_item_data(&fm, &item, "post", &mut fetcher, None).unwrap();

        assert_eq!(result.len(), 1);
        let authors = result["author"].as_array().unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0]["name"], "Bob");
    }

    #[test]
    fn test_resolve_item_data_no_interpolation() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/sidebar.yaml", "- widget: recent\n");

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);

        let fm = Frontmatter {
            item_as: "post".into(),
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "sidebar".into(),
                    DataQuery {
                        file: Some("sidebar.yaml".into()),
                        ..Default::default()
                    },
                );
                m
            },
            ..Default::default()
        };

        let item = json!({"id": 1});
        let result = resolve_item_data(&fm, &item, "post", &mut fetcher, None).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result["sidebar"].is_array());
    }

    // --- Plugin registry integration tests ---

    #[test]
    fn test_resolve_page_data_with_plugin_registry() {
        use crate::plugins::Plugin;
        use crate::plugins::registry::PluginRegistry;

        #[derive(Debug)]
        struct TagPlugin;

        impl Plugin for TagPlugin {
            fn name(&self) -> &str { "tag" }

            fn transform_data(
                &self,
                mut value: serde_json::Value,
                _source: Option<&str>,
                _path: Option<&str>,
            ) -> eyre::Result<serde_json::Value> {
                if let serde_json::Value::Array(ref mut arr) = value {
                    for item in arr.iter_mut() {
                        if let Some(obj) = item.as_object_mut() {
                            obj.insert("tagged".into(), json!(true));
                        }
                    }
                }
                Ok(value)
            }
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/items.json", r#"[{"id": 1}, {"id": 2}]"#);

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TagPlugin));

        let fm = Frontmatter {
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "items".into(),
                    DataQuery {
                        file: Some("items.json".into()),
                        ..Default::default()
                    },
                );
                m
            },
            ..Default::default()
        };

        let result = resolve_page_data(&fm, &mut fetcher, Some(&registry)).unwrap();
        let items = result["items"].as_array().unwrap();
        assert!(items[0]["tagged"].as_bool().unwrap());
        assert!(items[1]["tagged"].as_bool().unwrap());
    }

    #[test]
    fn test_resolve_dynamic_page_data_with_plugin_registry() {
        use crate::plugins::Plugin;
        use crate::plugins::registry::PluginRegistry;

        #[derive(Debug)]
        struct EnrichPlugin;

        impl Plugin for EnrichPlugin {
            fn name(&self) -> &str { "enrich" }

            fn transform_data(
                &self,
                mut value: serde_json::Value,
                _source: Option<&str>,
                _path: Option<&str>,
            ) -> eyre::Result<serde_json::Value> {
                if let serde_json::Value::Array(ref mut arr) = value {
                    for item in arr.iter_mut() {
                        if let Some(obj) = item.as_object_mut() {
                            obj.insert("enriched".into(), json!(true));
                        }
                    }
                }
                Ok(value)
            }
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/posts.json",
            r#"[{"slug": "a", "title": "A"}, {"slug": "b", "title": "B"}]"#,
        );

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(EnrichPlugin));

        let fm = Frontmatter {
            collection: Some(DataQuery {
                file: Some("posts.json".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let items = resolve_dynamic_page_data(&fm, &mut fetcher, Some(&registry)).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0]["enriched"].as_bool().unwrap());
        assert!(items[1]["enriched"].as_bool().unwrap());
    }

    #[test]
    fn test_resolve_dynamic_page_data_for_item_with_plugin() {
        use crate::plugins::Plugin;
        use crate::plugins::registry::PluginRegistry;

        #[derive(Debug)]
        struct PassthroughPlugin;

        impl Plugin for PassthroughPlugin {
            fn name(&self) -> &str { "passthrough" }
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/sidebar.yaml", "- widget: recent\n");

        let mut fetcher = DataFetcher::new(&HashMap::new(), root);
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(PassthroughPlugin));

        let fm = Frontmatter {
            item_as: "post".into(),
            data: {
                let mut m = HashMap::new();
                m.insert(
                    "sidebar".into(),
                    DataQuery {
                        file: Some("sidebar.yaml".into()),
                        ..Default::default()
                    },
                );
                m
            },
            ..Default::default()
        };

        let item = json!({"id": 1, "title": "Test"});
        let result =
            resolve_dynamic_page_data_for_item(&fm, &item, &mut fetcher, Some(&registry)).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result["sidebar"].is_array());
    }
}
