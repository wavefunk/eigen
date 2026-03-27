//! Step 3.1: Global data loader — reads `_data/` directory.
//!
//! Walks the `_data/` directory and parses `.yaml`/`.yml` and `.json` files
//! into `serde_json::Value`. Files are keyed by filename (without extension),
//! and nested directories are flattened with underscores:
//!
//! - `_data/nav.yaml`           → key `"nav"`
//! - `_data/footer/links.yaml`  → key `"footer_links"`

use crate::config::interpolate_env_vars;
use eyre::{Result, WrapErr};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

/// Load all data files from the `_data/` directory under `project_root`.
///
/// Returns a map of key → parsed JSON value. If the `_data/` directory does
/// not exist, returns an empty map.
pub fn load_global_data(project_root: &Path) -> Result<HashMap<String, Value>> {
    let data_dir = project_root.join("_data");
    let mut data = HashMap::new();

    if !data_dir.is_dir() {
        return Ok(data);
    }

    for entry in WalkDir::new(&data_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let value = match ext {
            "yaml" | "yml" => {
                let content = std::fs::read_to_string(path)
                    .wrap_err_with(|| format!("Failed to read {}", path.display()))?;
                let content = interpolate_env_vars(&content)
                    .wrap_err_with(|| format!("Failed to interpolate env vars in {}", path.display()))?;
                let v: Value = serde_yaml::from_str(&content)
                    .wrap_err_with(|| format!("Failed to parse YAML in {}", path.display()))?;
                v
            }
            "json" => {
                let content = std::fs::read_to_string(path)
                    .wrap_err_with(|| format!("Failed to read {}", path.display()))?;
                let content = interpolate_env_vars(&content)
                    .wrap_err_with(|| format!("Failed to interpolate env vars in {}", path.display()))?;
                let v: Value = serde_json::from_str(&content)
                    .wrap_err_with(|| format!("Failed to parse JSON in {}", path.display()))?;
                v
            }
            _ => continue,
        };

        // Compute the key: relative path from _data/ without extension,
        // with path separators replaced by underscores.
        let rel = path
            .strip_prefix(&data_dir)
            .wrap_err("Data file path is not inside _data/")?;

        let key = rel
            .with_extension("")
            .to_string_lossy()
            .replace(['/', '\\'], "_");

        if key.is_empty() {
            continue;
        }

        data.insert(key, value);
    }

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to write a file, creating parent directories as needed.
    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_load_yaml() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/nav.yaml",
            "- label: Home\n  url: /\n- label: About\n  url: /about\n",
        );

        let data = load_global_data(root).unwrap();
        assert_eq!(data.len(), 1);

        let nav = &data["nav"];
        assert!(nav.is_array());
        let arr = nav.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["label"], "Home");
        assert_eq!(arr[1]["url"], "/about");
    }

    #[test]
    fn test_load_yml_extension() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/settings.yml", "theme: dark\nlang: en\n");

        let data = load_global_data(root).unwrap();
        assert_eq!(data.len(), 1);

        let settings = &data["settings"];
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["lang"], "en");
    }

    #[test]
    fn test_load_json() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/meta.json",
            r#"{"version": "1.0", "author": "Test"}"#,
        );

        let data = load_global_data(root).unwrap();
        assert_eq!(data.len(), 1);

        let meta = &data["meta"];
        assert_eq!(meta["version"], "1.0");
        assert_eq!(meta["author"], "Test");
    }

    #[test]
    fn test_nested_directory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            root,
            "_data/footer/links.yaml",
            "- label: GitHub\n  url: https://github.com\n",
        );

        let data = load_global_data(root).unwrap();
        assert_eq!(data.len(), 1);
        assert!(data.contains_key("footer_links"));
        assert!(data["footer_links"].is_array());
    }

    #[test]
    fn test_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("_data")).unwrap();

        let data = load_global_data(root).unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn test_no_data_directory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let data = load_global_data(root).unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn test_non_data_files_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/readme.md", "# Data docs");
        write(root, "_data/nav.yaml", "- label: Home\n  url: /\n");

        let data = load_global_data(root).unwrap();
        assert_eq!(data.len(), 1);
        assert!(data.contains_key("nav"));
    }

    #[test]
    fn test_multiple_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/nav.yaml", "- label: Home\n  url: /\n");
        write(root, "_data/config.json", r#"{"debug": false}"#);
        write(root, "_data/social/links.yml", "- name: Twitter\n");

        let data = load_global_data(root).unwrap();
        assert_eq!(data.len(), 3);
        assert!(data.contains_key("nav"));
        assert!(data.contains_key("config"));
        assert!(data.contains_key("social_links"));
    }

    #[test]
    fn test_deeply_nested() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(root, "_data/a/b/c.yaml", "value: deep\n");

        let data = load_global_data(root).unwrap();
        assert_eq!(data.len(), 1);
        assert!(data.contains_key("a_b_c"));
        assert_eq!(data["a_b_c"]["value"], "deep");
    }
}
