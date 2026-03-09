//! Step 3.2: DataFetcher — fetch data from local files or remote sources with
//! caching, root extraction, and transforms.

use eyre::{Result, WrapErr, bail};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::SourceConfig;
use crate::frontmatter::DataQuery;
use crate::plugins::registry::PluginRegistry;

use super::transforms::apply_transforms;

/// Fetches and caches data from local files and remote HTTP sources.
pub struct DataFetcher {
    /// Source definitions from `site.toml`.
    sources: HashMap<String, SourceConfig>,
    /// Cache for HTTP responses keyed by full URL.
    url_cache: HashMap<String, Value>,
    /// Cache for local file data keyed by file path (relative to `_data/`).
    file_cache: HashMap<String, Value>,
    /// Path to the project's `_data/` directory.
    data_dir: PathBuf,
    /// HTTP client (reused across requests).
    client: reqwest::blocking::Client,
}

impl DataFetcher {
    /// Create a new fetcher with the given sources and project root.
    pub fn new(sources: &HashMap<String, SourceConfig>, project_root: &Path) -> Self {
        Self {
            sources: sources.clone(),
            url_cache: HashMap::new(),
            file_cache: HashMap::new(),
            data_dir: project_root.join("_data"),
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Fetch data for a single `DataQuery`.
    ///
    /// The query may reference a local file (`file` field) or a remote source
    /// (`source` + `path` fields). After fetching the raw data, `root`
    /// extraction, plugin transforms, and transforms (filter, sort, limit)
    /// are applied.
    pub fn fetch(
        &mut self,
        query: &DataQuery,
        plugin_registry: Option<&PluginRegistry>,
    ) -> Result<Value> {
        let source_name = query.source.as_deref();
        let query_path = query.path.as_deref();

        let raw = if let Some(ref file) = query.file {
            self.fetch_file(file)?
        } else if let Some(ref source_name) = query.source {
            self.fetch_source(source_name, query.path.as_deref().unwrap_or(""))?
        } else {
            bail!(
                "DataQuery has neither `file` nor `source` set. \
                 At least one must be provided."
            );
        };

        // Apply root extraction.
        let extracted = if let Some(ref root) = query.root {
            extract_root(&raw, root)?
        } else {
            raw
        };

        // Apply plugin data transforms (e.g., Strapi flattening).
        let transformed = if let Some(registry) = plugin_registry {
            registry.transform_data(extracted, source_name, query_path)?
        } else {
            extracted
        };

        // Apply transforms: filter → sort → limit.
        let result = apply_transforms(transformed, &query.filter, &query.sort, &query.limit);

        Ok(result)
    }

    /// Load a local file from `_data/`.
    fn fetch_file(&mut self, file_path: &str) -> Result<Value> {
        if let Some(cached) = self.file_cache.get(file_path) {
            return Ok(cached.clone());
        }

        let full_path = self.data_dir.join(file_path);
        let content = std::fs::read_to_string(&full_path)
            .wrap_err_with(|| format!("Failed to read data file: {}", full_path.display()))?;

        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let value: Value = match ext {
            "yaml" | "yml" => serde_yaml::from_str(&content)
                .wrap_err_with(|| format!("Failed to parse YAML: {}", full_path.display()))?,
            "json" => serde_json::from_str(&content)
                .wrap_err_with(|| format!("Failed to parse JSON: {}", full_path.display()))?,
            _ => bail!(
                "Unsupported data file extension '.{}' for {}. Use .yaml, .yml, or .json.",
                ext,
                full_path.display()
            ),
        };

        self.file_cache.insert(file_path.to_string(), value.clone());
        Ok(value)
    }

    /// Fetch data from a remote source defined in `site.toml`.
    fn fetch_source(&mut self, source_name: &str, path: &str) -> Result<Value> {
        let source = self
            .sources
            .get(source_name)
            .ok_or_else(|| eyre::eyre!(
                "Source '{}' not found in site.toml. Available: {}",
                source_name,
                self.sources.keys().cloned().collect::<Vec<_>>().join(", ")
            ))?
            .clone();

        // Build full URL: base URL + path.
        let full_url = format!(
            "{}{}",
            source.url.trim_end_matches('/'),
            if path.starts_with('/') { path.to_string() } else { format!("/{}", path) }
        );

        // Check URL cache.
        if let Some(cached) = self.url_cache.get(&full_url) {
            return Ok(cached.clone());
        }

        // Perform HTTP GET.
        let mut request = self.client.get(&full_url);
        for (key, val) in &source.headers {
            request = request.header(key.as_str(), val.as_str());
        }

        let response = request.send().wrap_err_with(|| {
            format!("HTTP request failed for {}", full_url)
        })?;

        let status = response.status();
        if !status.is_success() {
            bail!(
                "HTTP {} from {}",
                status,
                full_url,
            );
        }

        let value: Value = response.json().wrap_err_with(|| {
            format!("Failed to parse JSON response from {}", full_url)
        })?;

        self.url_cache.insert(full_url, value.clone());
        Ok(value)
    }

    /// Clear the file cache (used when `_data/` files change during dev).
    pub fn clear_file_cache(&mut self) {
        self.file_cache.clear();
    }

    /// Clear the URL cache (used when frontmatter queries change during dev).
    pub fn clear_url_cache(&mut self) {
        self.url_cache.clear();
    }
}

/// Walk into a JSON value using a dot-separated path.
///
/// For example, `extract_root(value, "data.posts")` returns `value["data"]["posts"]`.
fn extract_root(value: &Value, root: &str) -> Result<Value> {
    let mut current = value;
    for segment in root.split('.') {
        match current.get(segment) {
            Some(inner) => current = inner,
            None => bail!(
                "Root path '{}' not found in data. Failed at segment '{}'. \
                 Available keys: {}",
                root,
                segment,
                available_keys(current),
            ),
        }
    }
    Ok(current.clone())
}

/// List the keys of a JSON object for error messages, or describe its type.
fn available_keys(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                "(empty object)".to_string()
            } else {
                map.keys().cloned().collect::<Vec<_>>().join(", ")
            }
        }
        Value::Array(_) => "(value is an array, not an object)".to_string(),
        _ => format!("(value is {}, not an object)", value_type_name(value)),
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    /// Create a fetcher with no remote sources, pointed at a temp dir.
    fn test_fetcher(root: &Path) -> DataFetcher {
        DataFetcher::new(&HashMap::new(), root)
    }

    /// Helper to write a file.
    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // --- extract_root tests ---

    #[test]
    fn test_extract_root_single_level() {
        let value = json!({"data": [1, 2, 3]});
        let result = extract_root(&value, "data").unwrap();
        assert_eq!(result, json!([1, 2, 3]));
    }

    #[test]
    fn test_extract_root_nested() {
        let value = json!({"data": {"posts": [{"id": 1}]}});
        let result = extract_root(&value, "data.posts").unwrap();
        assert_eq!(result, json!([{"id": 1}]));
    }

    #[test]
    fn test_extract_root_missing_key() {
        let value = json!({"data": {"users": []}});
        let result = extract_root(&value, "data.posts");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("posts"));
        assert!(err.contains("users")); // should show available keys
    }

    #[test]
    fn test_extract_root_from_non_object() {
        let value = json!([1, 2, 3]);
        let result = extract_root(&value, "items");
        assert!(result.is_err());
    }

    // --- fetch_file tests ---

    #[test]
    fn test_fetch_file_yaml() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/nav.yaml", "- label: Home\n  url: /\n");

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("nav.yaml".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, None).unwrap();
        assert!(result.is_array());
        assert_eq!(result.as_array().unwrap().len(), 1);
        assert_eq!(result[0]["label"], "Home");
    }

    #[test]
    fn test_fetch_file_json() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/config.json", r#"{"debug": true}"#);

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("config.json".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, None).unwrap();
        assert_eq!(result["debug"], true);
    }

    #[test]
    fn test_fetch_file_with_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/response.json",
            r#"{"data": {"items": [1, 2, 3]}}"#,
        );

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("response.json".into()),
            root: Some("data.items".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, None).unwrap();
        assert_eq!(result, json!([1, 2, 3]));
    }

    #[test]
    fn test_fetch_file_with_transforms() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/posts.json",
            r#"[
                {"id": 3, "status": "draft"},
                {"id": 1, "status": "published"},
                {"id": 5, "status": "published"},
                {"id": 2, "status": "published"},
                {"id": 4, "status": "draft"}
            ]"#,
        );

        let mut filter = HashMap::new();
        filter.insert("status".into(), "published".into());

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("posts.json".into()),
            filter: Some(filter),
            sort: Some("id".into()),
            limit: Some(2),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, None).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[1]["id"], 2);
    }

    #[test]
    fn test_fetch_file_caching() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/nav.yaml", "- label: Home\n");

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("nav.yaml".into()),
            ..Default::default()
        };

        // First fetch reads file.
        let r1 = fetcher.fetch(&query, None).unwrap();

        // Delete the file — cached result should still work.
        fs::remove_file(root.join("_data/nav.yaml")).unwrap();
        let r2 = fetcher.fetch(&query, None).unwrap();

        assert_eq!(r1, r2);
    }

    #[test]
    fn test_fetch_file_missing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("_data")).unwrap();

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("nonexistent.yaml".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_fetch_no_file_or_source_errors() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let mut fetcher = test_fetcher(root);
        let query = DataQuery::default();

        let result = fetcher.fetch(&query, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("neither"));
    }

    #[test]
    fn test_fetch_unknown_source_errors() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            source: Some("nonexistent".into()),
            path: Some("/posts".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent"));
    }

    #[test]
    fn test_clear_caches() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/data.json", r#"{"v": 1}"#);

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("data.json".into()),
            ..Default::default()
        };

        let _ = fetcher.fetch(&query, None).unwrap();
        assert!(!fetcher.file_cache.is_empty());

        fetcher.clear_file_cache();
        assert!(fetcher.file_cache.is_empty());

        fetcher.url_cache.insert("http://test".into(), json!(null));
        assert!(!fetcher.url_cache.is_empty());
        fetcher.clear_url_cache();
        assert!(fetcher.url_cache.is_empty());
    }

    // --- Plugin integration tests ---

    #[test]
    fn test_fetch_with_plugin_registry_transforms_data() {
        use crate::plugins::Plugin;
        use crate::plugins::registry::PluginRegistry;

        #[derive(Debug)]
        struct AddFieldPlugin;

        impl Plugin for AddFieldPlugin {
            fn name(&self) -> &str { "add_field" }

            fn transform_data(
                &self,
                mut value: serde_json::Value,
                _source: Option<&str>,
                _path: Option<&str>,
            ) -> eyre::Result<serde_json::Value> {
                if let serde_json::Value::Array(ref mut arr) = value {
                    for item in arr.iter_mut() {
                        if let Some(obj) = item.as_object_mut() {
                            obj.insert("added".into(), json!(true));
                        }
                    }
                }
                Ok(value)
            }
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/items.json", r#"[{"id": 1}, {"id": 2}]"#);

        let mut registry = PluginRegistry::new();
        registry.register(Box::new(AddFieldPlugin));

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("items.json".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, Some(&registry)).unwrap();
        let arr = result.as_array().unwrap();
        assert!(arr[0]["added"].as_bool().unwrap());
        assert!(arr[1]["added"].as_bool().unwrap());
    }

    #[test]
    fn test_fetch_with_none_plugin_registry_no_transform() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/items.json", r#"[{"id": 1}]"#);

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("items.json".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, None).unwrap();
        let arr = result.as_array().unwrap();
        // No "added" field since no plugin.
        assert!(arr[0].get("added").is_none());
        assert_eq!(arr[0]["id"], 1);
    }

    #[test]
    fn test_fetch_plugin_runs_after_root_extraction() {
        use crate::plugins::Plugin;
        use crate::plugins::registry::PluginRegistry;

        #[derive(Debug)]
        struct CountPlugin;

        impl Plugin for CountPlugin {
            fn name(&self) -> &str { "count" }

            fn transform_data(
                &self,
                value: serde_json::Value,
                _source: Option<&str>,
                _path: Option<&str>,
            ) -> eyre::Result<serde_json::Value> {
                // This should receive the root-extracted value (the array),
                // NOT the full wrapper object.
                if let serde_json::Value::Array(ref arr) = value {
                    assert_eq!(arr.len(), 2, "Plugin should receive the extracted array");
                }
                Ok(value)
            }
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/response.json",
            r#"{"data": [{"id": 1}, {"id": 2}]}"#,
        );

        let mut registry = PluginRegistry::new();
        registry.register(Box::new(CountPlugin));

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("response.json".into()),
            root: Some("data".into()),
            ..Default::default()
        };

        let result = fetcher.fetch(&query, Some(&registry)).unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_fetch_plugin_runs_before_filter_sort_limit() {
        use crate::plugins::Plugin;
        use crate::plugins::registry::PluginRegistry;

        /// Plugin that adds a "status" field to all items.
        #[derive(Debug)]
        struct StatusPlugin;

        impl Plugin for StatusPlugin {
            fn name(&self) -> &str { "status" }

            fn transform_data(
                &self,
                mut value: serde_json::Value,
                _source: Option<&str>,
                _path: Option<&str>,
            ) -> eyre::Result<serde_json::Value> {
                if let serde_json::Value::Array(ref mut arr) = value {
                    for item in arr.iter_mut() {
                        if let Some(obj) = item.as_object_mut() {
                            // Add status=published to all items.
                            obj.insert("status".into(), json!("published"));
                        }
                    }
                }
                Ok(value)
            }
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Items don't have a "status" field — the plugin adds it.
        write(
            root,
            "_data/items.json",
            r#"[{"id": 1}, {"id": 2}, {"id": 3}]"#,
        );

        let mut registry = PluginRegistry::new();
        registry.register(Box::new(StatusPlugin));

        let mut filter = HashMap::new();
        filter.insert("status".into(), "published".into());

        let mut fetcher = test_fetcher(root);
        let query = DataQuery {
            file: Some("items.json".into()),
            filter: Some(filter),
            limit: Some(2),
            ..Default::default()
        };

        // The plugin adds "status" = "published" to all items,
        // then the filter keeps only those (all 3), then limit=2.
        let result = fetcher.fetch(&query, Some(&registry)).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["status"], "published");
    }
}
