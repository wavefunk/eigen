# Content Hashing for Static Assets Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fingerprint all static assets with content-based hashes in their filenames so that CDNs and browsers can cache them indefinitely, and rewrite all references across the site to use the hashed URLs.

**Architecture:** A new single-file `build::content_hash` module (`src/build/content_hash.rs`) implements three coordinated phases. Phase 1 runs during `copy_static_assets`: after copying files from `static/` to `dist/`, it hashes each file (SHA-256, truncated to 16 hex chars), renames it to include the hash, and builds an `AssetManifest` (HashMap of original path to hashed path). Phase 2 threads the manifest into the `asset()` template function so templates render with correct hashed URLs at render time. Phase 3 runs as a post-build pass that rewrites any remaining hardcoded asset references in all `.html`, `.css`, and `.js` files in `dist/`. The feature is opt-in via `[build.content_hash]` in `site.toml`, disabled by default, and fully skipped during dev server operation.

**Tech Stack:** Rust, sha2 (SHA-256 hashing, already a dependency), walkdir (directory traversal, already a dependency), glob (exclude pattern matching, already a dependency)

**Design spec:** `docs/superpowers/specs/2026-03-17-content-hashing-design.md`

---

## File Structure

### New files to create

| File | Responsibility |
|------|---------------|
| `src/build/content_hash.rs` | `AssetManifest` struct, `build_manifest()` (Phase 1), `rewrite_references()` (Phase 3), helper functions (`content_hash()`, `hashed_filename()`, `is_excluded()`, `rewrite_file_content()`) |
| `docs/content_hashing.md` | Feature documentation for future reference |

### Existing files to modify

| File | Change |
|------|--------|
| `src/config/mod.rs` | Add `ContentHashConfig` struct with `enabled` and `exclude` fields, add `content_hash` field to `BuildConfig`, update `Default` impl |
| `src/build/mod.rs` | Add `pub mod content_hash;` declaration |
| `src/build/output.rs` | Change `copy_static_assets` signature to accept `&ContentHashConfig` and return `Result<AssetManifest>`, call `build_manifest()` when enabled |
| `src/build/render.rs` | Thread manifest through build pipeline: wrap in `Arc`, pass to `setup_environment`, pass to `inline_critical_css`, call `rewrite_references` after post-build hooks |
| `src/template/functions.rs` | Change `register_functions` signature to accept `Option<Arc<AssetManifest>>`, update `asset()` closure to look up manifest |
| `src/template/environment.rs` | Change `setup_environment` signature to accept and pass through `Option<Arc<AssetManifest>>` to `register_functions` |
| `src/build/critical_css/mod.rs` | Add manifest fallback to `get_or_load`, `load_stylesheet`, and `inline_critical_css` for resolving hashed stylesheet paths |
| `src/dev/rebuild.rs` | Update `copy_static_assets` calls to pass `&ContentHashConfig::default()` and discard returned manifest; update `setup_environment` call to pass `None` for manifest |

---

## Task 1: Add `ContentHashConfig` to configuration

**Depends on:** Nothing (starting point)

**Files:**
- Modify: `src/config/mod.rs`

This task adds the configuration struct and integrates it into `BuildConfig`. No new dependencies are needed -- `sha2`, `walkdir`, and `glob` are already in `Cargo.toml`.

- [ ] **Step 1: Add `ContentHashConfig` struct to `src/config/mod.rs`**

Add the following after the `HintsConfig` struct and its `Default` impl (after line 276, before the `SourceConfig` struct):

```rust
/// Configuration for content hashing of static assets.
///
/// Located under `[build.content_hash]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentHashConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Glob patterns for files in `static/` to exclude from hashing.
    /// Matched against the path relative to `static/`.
    /// Default: common files that must keep stable names.
    #[serde(default = "default_hash_exclude")]
    pub exclude: Vec<String>,
}

fn default_hash_exclude() -> Vec<String> {
    vec![
        "favicon.ico".into(),
        "robots.txt".into(),
        "CNAME".into(),
        "_headers".into(),
        "_redirects".into(),
        ".well-known/**".into(),
    ]
}

impl Default for ContentHashConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            exclude: default_hash_exclude(),
        }
    }
}
```

- [ ] **Step 2: Add `content_hash` field to `BuildConfig`**

In `src/config/mod.rs`, add a new field to the `BuildConfig` struct (after the `hints` field, around line 56):

```rust
    /// Content hashing for static assets.
    #[serde(default)]
    pub content_hash: ContentHashConfig,
```

Update the `Default` impl for `BuildConfig` (around line 59) to include:

```rust
            content_hash: ContentHashConfig::default(),
```

This goes after the `hints: HintsConfig::default(),` line.

- [ ] **Step 3: Write tests for the new config types**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/config/mod.rs`:

```rust
    // --- Content hash config tests ---

    #[test]
    fn test_content_hash_config_defaults() {
        let toml_str = r#"
[site]
name = "Hash Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.content_hash.enabled);
        assert_eq!(config.build.content_hash.exclude.len(), 6);
        assert!(config.build.content_hash.exclude.contains(&"favicon.ico".to_string()));
        assert!(config.build.content_hash.exclude.contains(&"CNAME".to_string()));
    }

    #[test]
    fn test_content_hash_config_enabled() {
        let toml_str = r#"
[site]
name = "Hash Enabled"
base_url = "https://example.com"

[build.content_hash]
enabled = true
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.content_hash.enabled);
        // Other fields should have defaults.
        assert_eq!(config.build.content_hash.exclude.len(), 6);
    }

    #[test]
    fn test_content_hash_config_custom_exclude() {
        let toml_str = r#"
[site]
name = "Hash Custom"
base_url = "https://example.com"

[build.content_hash]
enabled = true
exclude = ["favicon.ico", "sw.js", "manifest.json"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.content_hash.enabled);
        assert_eq!(config.build.content_hash.exclude.len(), 3);
        assert!(config.build.content_hash.exclude.contains(&"sw.js".to_string()));
    }
```

- [ ] **Step 4: Run tests to verify config changes compile and pass**

Run: `cargo test --lib config -- --nocapture`

Expected: All existing config tests pass, plus the 3 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat(content-hash): add ContentHashConfig to site configuration"
```

---

## Task 2: Create the `content_hash` module with core helpers and `AssetManifest`

**Depends on:** Task 1 (needs `ContentHashConfig`)

**Files:**
- Create: `src/build/content_hash.rs`
- Modify: `src/build/mod.rs`

This task creates the new module with the `AssetManifest` struct and three private helper functions: `content_hash()`, `hashed_filename()`, and `is_excluded()`. The two public functions (`build_manifest` and `rewrite_references`) are stubbed for now and implemented in Tasks 3 and 5.

- [ ] **Step 1: Add module declaration to `src/build/mod.rs`**

Add `pub mod content_hash;` to the module list. Insert it as the first entry to maintain alphabetical order. The existing `pub use render::build;` re-export at the end of the file stays unchanged:

```rust
pub mod content_hash;
pub mod context;
pub mod critical_css;
pub mod fragments;
pub mod hints;
pub mod minify;
pub mod output;
pub mod render;
pub mod sitemap;

pub use render::build;
```

- [ ] **Step 2: Create `src/build/content_hash.rs` with `AssetManifest` and helpers**

Create the file with:

```rust
//! Content hashing for static assets.
//!
//! Fingerprints files copied from `static/` to `dist/` by embedding a
//! content hash in the filename (e.g. `style.css` -> `style.a1b2c3d4e5f67890.css`).
//! This enables immutable caching (`Cache-Control: max-age=31536000, immutable`).
//!
//! Three phases:
//! - Phase 1 (`build_manifest`): hash and rename files in dist, build manifest.
//! - Phase 2: `asset()` template function uses manifest for render-time resolution.
//! - Phase 3 (`rewrite_references`): post-render string replacement in HTML/CSS/JS.

use eyre::{Result, WrapErr};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use crate::config::ContentHashConfig;

/// Mapping from original asset URL paths to content-hashed URL paths.
///
/// Built during `copy_static_assets` when content hashing is enabled.
/// Shared across the build pipeline via `Arc`.
///
/// All paths are URL paths relative to site root with a leading slash,
/// e.g. `/css/style.css` -> `/css/style.a1b2c3d4e5f67890.css`.
pub struct AssetManifest {
    /// Forward map: original path -> hashed path.
    entries: HashMap<String, String>,
}

impl AssetManifest {
    /// Create a new empty manifest.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Create with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(cap),
        }
    }

    /// Insert a mapping.
    pub fn insert(&mut self, original: String, hashed: String) {
        self.entries.insert(original, hashed);
    }

    /// Look up the hashed path for an original path.
    /// Returns the original path unchanged if not found in the manifest.
    pub fn resolve(&self, original: &str) -> &str {
        self.entries
            .get(original)
            .map(|s| s.as_str())
            .unwrap_or(original)
    }

    /// Return all (original, hashed) pairs sorted by original path
    /// length descending.
    ///
    /// Longest-match-first ordering prevents partial replacements during
    /// string rewriting.
    pub fn pairs_longest_first(&self) -> Vec<(&str, &str)> {
        let mut pairs: Vec<(&str, &str)> = self
            .entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        pairs
    }

    /// Whether the manifest is empty (no assets were hashed).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Compute the content hash for a file's bytes.
///
/// Returns a 16-character lowercase hex string (SHA-256 truncated to 8
/// bytes). This matches the hash length used by image optimization.
fn content_hash(data: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for b in &result[..8] {
        // write! to a String never fails.
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

/// Construct a hashed filename from the original filename and hash.
///
/// Inserts the hash between the stem and extension:
///   `style.css` + `a1b2c3d4` -> `style.a1b2c3d4.css`
///   `app.min.js` + `a1b2c3d4` -> `app.min.a1b2c3d4.js`
///
/// Files without an extension get the hash appended:
///   `LICENSE` + `a1b2c3d4` -> `LICENSE.a1b2c3d4`
fn hashed_filename(original_filename: &str, hash: &str) -> String {
    match original_filename.rfind('.') {
        Some(dot_pos) => {
            let stem = &original_filename[..dot_pos];
            let ext = &original_filename[dot_pos + 1..];
            format!("{stem}.{hash}.{ext}")
        }
        None => {
            format!("{original_filename}.{hash}")
        }
    }
}

/// Check whether a file should be excluded from content hashing.
///
/// Uses `glob::Pattern` matching against the relative path from `static/`.
fn is_excluded(relative_path: &str, exclude_patterns: &[String]) -> bool {
    for pattern_str in exclude_patterns {
        match glob::Pattern::new(pattern_str) {
            Ok(pattern) => {
                if pattern.matches(relative_path) {
                    return true;
                }
            }
            Err(e) => {
                tracing::warn!("Invalid exclude pattern '{}': {}", pattern_str, e);
            }
        }
    }
    false
}

/// Build the asset manifest by hashing and renaming static assets in
/// dist_dir.
///
/// Walks all files in dist_dir that correspond to files from static_dir,
/// computes content hashes, renames files to include the hash, and
/// returns the manifest.
///
/// Files matching the exclude patterns are left at their original names.
pub fn build_manifest(
    _dist_dir: &Path,
    _static_dir: &Path,
    _config: &ContentHashConfig,
) -> Result<AssetManifest> {
    // Implemented in Task 3.
    Ok(AssetManifest::new())
}

/// Rewrite all references to static assets in rendered HTML, CSS, and
/// JS files in dist_dir.
///
/// Walks all `.html`, `.css`, and `.js` files in dist_dir and replaces
/// occurrences of original asset paths with their hashed equivalents.
///
/// Fragment HTML files in `_fragments/` ARE rewritten.
pub fn rewrite_references(
    _dist_dir: &Path,
    _manifest: &AssetManifest,
) -> Result<()> {
    // Implemented in Task 5.
    Ok(())
}
```

- [ ] **Step 3: Write unit tests for the helper functions**

Add the following test module at the bottom of `src/build/content_hash.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // --- content_hash ---

    #[test]
    fn test_content_hash_deterministic() {
        let data = b"hello world";
        let h1 = content_hash(data);
        let h2 = content_hash(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_content_hash_different_data() {
        let h1 = content_hash(b"hello");
        let h2 = content_hash(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_content_hash_length() {
        let h = content_hash(b"test data");
        assert_eq!(h.len(), 16, "hash should be 16 hex chars (8 bytes)");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // --- hashed_filename ---

    #[test]
    fn test_hashed_filename_with_ext() {
        let result = hashed_filename("style.css", "a1b2c3d4");
        assert_eq!(result, "style.a1b2c3d4.css");
    }

    #[test]
    fn test_hashed_filename_multi_dot() {
        let result = hashed_filename("app.min.js", "a1b2c3d4");
        assert_eq!(result, "app.min.a1b2c3d4.js");
    }

    #[test]
    fn test_hashed_filename_no_ext() {
        let result = hashed_filename("LICENSE", "a1b2c3d4");
        assert_eq!(result, "LICENSE.a1b2c3d4");
    }

    #[test]
    fn test_hashed_filename_dotfile() {
        let result = hashed_filename(".htaccess", "a1b2c3d4");
        assert_eq!(result, ".a1b2c3d4.htaccess");
    }

    // --- is_excluded ---

    #[test]
    fn test_is_excluded_default_patterns() {
        let patterns = vec![
            "favicon.ico".into(),
            "robots.txt".into(),
            "CNAME".into(),
            "_headers".into(),
            "_redirects".into(),
            ".well-known/**".into(),
        ];
        assert!(is_excluded("favicon.ico", &patterns));
        assert!(is_excluded("robots.txt", &patterns));
        assert!(is_excluded("CNAME", &patterns));
        assert!(is_excluded("_headers", &patterns));
        assert!(is_excluded("_redirects", &patterns));
    }

    #[test]
    fn test_is_excluded_glob_wellknown() {
        let patterns = vec![".well-known/**".into()];
        assert!(is_excluded(".well-known/security.txt", &patterns));
        assert!(is_excluded(".well-known/assetlinks.json", &patterns));
    }

    #[test]
    fn test_is_excluded_no_match() {
        let patterns = vec!["favicon.ico".into(), "robots.txt".into()];
        assert!(!is_excluded("css/style.css", &patterns));
        assert!(!is_excluded("js/app.js", &patterns));
        assert!(!is_excluded("images/logo.png", &patterns));
    }

    // --- AssetManifest ---

    #[test]
    fn test_manifest_resolve_found() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());
        assert_eq!(m.resolve("/css/style.css"), "/css/style.abc123.css");
    }

    #[test]
    fn test_manifest_resolve_not_found() {
        let m = AssetManifest::new();
        assert_eq!(m.resolve("/unknown.css"), "/unknown.css");
    }

    #[test]
    fn test_manifest_pairs_longest_first() {
        let mut m = AssetManifest::new();
        m.insert("/a.css".into(), "/a.h1.css".into());
        m.insert("/css/style.css".into(), "/css/style.h2.css".into());
        m.insert("/css/b.css".into(), "/css/b.h3.css".into());

        let pairs = m.pairs_longest_first();
        // Longest original path first.
        assert_eq!(pairs[0].0, "/css/style.css");
        assert!(pairs[0].0.len() >= pairs[1].0.len());
        assert!(pairs[1].0.len() >= pairs[2].0.len());
    }

    #[test]
    fn test_manifest_empty() {
        let m = AssetManifest::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn test_manifest_len() {
        let mut m = AssetManifest::new();
        m.insert("/a.css".into(), "/a.h.css".into());
        m.insert("/b.js".into(), "/b.h.js".into());
        assert!(!m.is_empty());
        assert_eq!(m.len(), 2);
    }
}
```

- [ ] **Step 4: Run tests to verify the module compiles and passes**

Run: `cargo test --lib build::content_hash -- --nocapture`

Expected: All 15 tests pass (3 content_hash + 4 hashed_filename + 3 is_excluded + 5 AssetManifest).

- [ ] **Step 5: Commit**

```bash
git add src/build/content_hash.rs src/build/mod.rs
git commit -m "feat(content-hash): add AssetManifest and core helper functions"
```

---

## Task 3: Implement `build_manifest` (Phase 1)

**Depends on:** Task 2 (needs `AssetManifest`, helpers)

**Files:**
- Modify: `src/build/content_hash.rs`

This task implements the `build_manifest` function which walks the static files in `dist/`, hashes them, renames them, and populates the manifest.

- [ ] **Step 1: Implement `build_manifest`**

Replace the stub `build_manifest` function in `src/build/content_hash.rs` with:

```rust
/// Build the asset manifest by hashing and renaming static assets in
/// dist_dir.
///
/// Walks all files in dist_dir that correspond to files from static_dir,
/// computes content hashes, renames files to include the hash, and
/// returns the manifest.
///
/// Files matching the exclude patterns are left at their original names.
pub fn build_manifest(
    dist_dir: &Path,
    static_dir: &Path,
    config: &ContentHashConfig,
) -> Result<AssetManifest> {
    use walkdir::WalkDir;

    if !static_dir.is_dir() {
        return Ok(AssetManifest::new());
    }

    // Count files for capacity hint.
    let file_count = WalkDir::new(static_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count();

    let mut manifest = AssetManifest::with_capacity(file_count);

    for entry in WalkDir::new(static_dir).into_iter() {
        let entry = entry.wrap_err("Failed to read entry while building asset manifest")?;

        if !entry.file_type().is_file() {
            continue;
        }

        let src_path = entry.path();
        let rel_path = src_path
            .strip_prefix(static_dir)
            .wrap_err("Asset path is not inside static/ directory")?;

        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        // Check exclusion.
        if is_excluded(&rel_str, &config.exclude) {
            tracing::debug!("Content hash: excluding {}", rel_str);
            continue;
        }

        // The file in dist/ has the same relative path.
        let dist_path = dist_dir.join(rel_path);
        if !dist_path.exists() {
            tracing::warn!("Content hash: expected file not found in dist: {}", dist_path.display());
            continue;
        }

        // Read file and compute hash.
        let data = match std::fs::read(&dist_path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Content hash: failed to read {}: {}", dist_path.display(), e);
                continue;
            }
        };

        let hash = content_hash(&data);

        // Build hashed filename.
        let filename = rel_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&rel_str);
        let new_filename = hashed_filename(filename, &hash);

        // Build new dist path.
        let new_dist_path = match dist_path.parent() {
            Some(parent) => parent.join(&new_filename),
            None => dist_dir.join(&new_filename),
        };

        // Rename file in dist.
        std::fs::rename(&dist_path, &new_dist_path).wrap_err_with(|| {
            format!(
                "Failed to rename {} -> {}",
                dist_path.display(),
                new_dist_path.display()
            )
        })?;

        // Build URL paths (with leading slash).
        let original_url = format!("/{}", rel_str);
        let hashed_rel = match rel_path.parent() {
            Some(parent) if parent.as_os_str().len() > 0 => {
                let parent_str = parent.to_string_lossy().replace('\\', "/");
                format!("/{}/{}", parent_str, new_filename)
            }
            _ => format!("/{}", new_filename),
        };

        tracing::debug!("Content hash: {} -> {}", original_url, hashed_rel);
        manifest.insert(original_url, hashed_rel);
    }

    Ok(manifest)
}
```

- [ ] **Step 2: Write tests for `build_manifest`**

Add these tests to the `#[cfg(test)] mod tests` block in `src/build/content_hash.rs`:

```rust
    // --- build_manifest ---

    /// Helper to write a file into a temp directory.
    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    /// Helper to create a default config with hashing enabled.
    fn hash_config() -> ContentHashConfig {
        ContentHashConfig {
            enabled: true,
            ..ContentHashConfig::default()
        }
    }

    #[test]
    fn test_build_manifest_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let static_dir = root.join("static");
        let dist_dir = root.join("dist");

        write_file(&static_dir, "css/style.css", "body { color: red; }");
        write_file(&dist_dir, "css/style.css", "body { color: red; }");

        let config = hash_config();
        let manifest = build_manifest(&dist_dir, &static_dir, &config).unwrap();

        assert_eq!(manifest.len(), 1);
        let resolved = manifest.resolve("/css/style.css");
        assert_ne!(resolved, "/css/style.css", "should be hashed");
        assert!(resolved.starts_with("/css/style."));
        assert!(resolved.ends_with(".css"));

        // Original file should no longer exist.
        assert!(!dist_dir.join("css/style.css").exists());
        // Hashed file should exist.
        let hashed_name = resolved.trim_start_matches("/css/");
        assert!(dist_dir.join("css").join(hashed_name).exists());
    }

    #[test]
    fn test_build_manifest_excludes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let static_dir = root.join("static");
        let dist_dir = root.join("dist");

        write_file(&static_dir, "favicon.ico", "icon");
        write_file(&dist_dir, "favicon.ico", "icon");
        write_file(&static_dir, "robots.txt", "allow all");
        write_file(&dist_dir, "robots.txt", "allow all");

        let config = hash_config();
        let manifest = build_manifest(&dist_dir, &static_dir, &config).unwrap();

        assert!(manifest.is_empty(), "excluded files should not be in manifest");
        // Files should still be at their original names.
        assert!(dist_dir.join("favicon.ico").exists());
        assert!(dist_dir.join("robots.txt").exists());
    }

    #[test]
    fn test_build_manifest_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let static_dir = root.join("static");
        let dist_dir = root.join("dist");
        std::fs::create_dir_all(&static_dir).unwrap();
        std::fs::create_dir_all(&dist_dir).unwrap();

        let config = hash_config();
        let manifest = build_manifest(&dist_dir, &static_dir, &config).unwrap();

        assert!(manifest.is_empty());
    }

    #[test]
    fn test_build_manifest_no_static_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let static_dir = root.join("static"); // does not exist
        let dist_dir = root.join("dist");
        std::fs::create_dir_all(&dist_dir).unwrap();

        let config = hash_config();
        let manifest = build_manifest(&dist_dir, &static_dir, &config).unwrap();

        assert!(manifest.is_empty());
    }

    #[test]
    fn test_build_manifest_nested_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let static_dir = root.join("static");
        let dist_dir = root.join("dist");

        write_file(&static_dir, "a/b/c/deep.js", "deep()");
        write_file(&dist_dir, "a/b/c/deep.js", "deep()");

        let config = hash_config();
        let manifest = build_manifest(&dist_dir, &static_dir, &config).unwrap();

        assert_eq!(manifest.len(), 1);
        let resolved = manifest.resolve("/a/b/c/deep.js");
        assert!(resolved.starts_with("/a/b/c/deep."));
        assert!(resolved.ends_with(".js"));
    }

    #[test]
    fn test_build_manifest_deterministic_hash() {
        // Same content should produce same hash.
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let static_dir = root.join("static");
        let dist_dir = root.join("dist");

        let content = "body { color: blue; }";
        write_file(&static_dir, "a.css", content);
        write_file(&dist_dir, "a.css", content);

        let config = hash_config();
        let m1 = build_manifest(&dist_dir, &static_dir, &config).unwrap();
        let r1 = m1.resolve("/a.css").to_string();

        // Rebuild with same content.
        write_file(&dist_dir, "a.css", content);
        let m2 = build_manifest(&dist_dir, &static_dir, &config).unwrap();
        let r2 = m2.resolve("/a.css").to_string();

        assert_eq!(r1, r2, "same content should produce same hash");
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib build::content_hash -- --nocapture`

Expected: All new `build_manifest` tests pass along with the existing helper tests.

- [ ] **Step 4: Commit**

```bash
git add src/build/content_hash.rs
git commit -m "feat(content-hash): implement build_manifest (Phase 1 - hash and rename)"
```

---

## Task 4: Wire Phase 1 into `copy_static_assets` and update call sites

**Depends on:** Task 3 (needs `build_manifest`)

**Files:**
- Modify: `src/build/output.rs`
- Modify: `src/build/render.rs`
- Modify: `src/dev/rebuild.rs`

This task changes the signature of `copy_static_assets` to accept `ContentHashConfig` and return `AssetManifest`, then updates all three call sites (production build, dev full_build, dev rebuild).

- [ ] **Step 1: Update `copy_static_assets` in `src/build/output.rs`**

Change the function signature and add the manifest building:

Replace the existing function signature and body. The current signature on line 41 is:

```rust
pub fn copy_static_assets(project_root: &Path) -> Result<()> {
```

Replace the entire `copy_static_assets` function with:

```rust
/// Copy the `static/` directory contents into `dist/` recursively.
///
/// Preserves directory structure: `static/css/style.css` → `dist/css/style.css`.
/// If `static/` does not exist, this is a no-op.
///
/// When content hashing is enabled, files are renamed with content-based
/// hashes and the mapping is returned in the `AssetManifest`.
pub fn copy_static_assets(
    project_root: &Path,
    content_hash_config: &crate::config::ContentHashConfig,
) -> Result<crate::build::content_hash::AssetManifest> {
    let static_dir = project_root.join("static");
    let dist_dir = project_root.join("dist");

    if !static_dir.is_dir() {
        return Ok(crate::build::content_hash::AssetManifest::new());
    }

    for entry in WalkDir::new(&static_dir).into_iter() {
        let entry = entry.wrap_err("Failed to read entry while copying static assets")?;
        let src_path = entry.path();
        let rel_path = src_path
            .strip_prefix(&static_dir)
            .wrap_err("Static asset path is not inside static/ directory")?;
        let dst_path = dist_dir.join(rel_path);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dst_path)
                .wrap_err_with(|| format!("Failed to create directory {}", dst_path.display()))?;
        } else {
            // Ensure parent directory exists.
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!("Failed to create parent dir for {}", dst_path.display()))?;
            }
            std::fs::copy(src_path, &dst_path)
                .wrap_err_with(|| {
                    format!("Failed to copy {} → {}", src_path.display(), dst_path.display())
                })?;
        }
    }

    // Phase 1: Build content hash manifest if enabled.
    if content_hash_config.enabled {
        crate::build::content_hash::build_manifest(&dist_dir, &static_dir, content_hash_config)
    } else {
        Ok(crate::build::content_hash::AssetManifest::new())
    }
}
```

- [ ] **Step 2: Update existing tests in `src/build/output.rs`**

All existing tests call `copy_static_assets(root)` with one argument. Update each call to pass the disabled config:

Replace every occurrence of `copy_static_assets(root)` in the tests with:

```rust
copy_static_assets(root, &crate::config::ContentHashConfig::default())
```

There are 3 calls to update: in `test_copy_static_assets` (line 146), `test_copy_static_assets_no_static_dir` (line 163), and `test_copy_static_preserves_structure` (line 174). The return value is already handled (`.unwrap()` on the `Result` works because the return type changes from `Result<()>` to `Result<AssetManifest>` and `.unwrap()` works on both).

Each test currently has a line like:
```rust
        copy_static_assets(root).unwrap();
```

Change each to:
```rust
        copy_static_assets(root, &crate::config::ContentHashConfig::default()).unwrap();
```

- [ ] **Step 3: Update the production build in `src/build/render.rs`**

In the `build()` function, find the line (around line 72):

```rust
    output::copy_static_assets(project_root)?;
    tracing::info!("Copying static assets... ✓");
```

Replace with:

```rust
    // Phase 1: Copy static assets (with content hashing if enabled).
    let manifest = output::copy_static_assets(project_root, &config.build.content_hash)?;
    let manifest = std::sync::Arc::new(manifest);

    if !manifest.is_empty() {
        tracing::info!(
            "Content hashing: {} assets fingerprinted.",
            manifest.len(),
        );
    }
    tracing::info!("Copying static assets... ✓");
```

Add `use std::sync::Arc;` to the imports at the top of the file if not already present. The existing imports in `render.rs` do not include `Arc`, so add it after the other `std` imports:

```rust
use std::sync::Arc;
```

- [ ] **Step 4: Update dev server call sites in `src/dev/rebuild.rs`**

Find the call at line 128:

```rust
                    crate::build::output::copy_static_assets(&self.project_root)?;
```

Replace with:

```rust
                    let _ = crate::build::output::copy_static_assets(
                        &self.project_root,
                        &crate::config::ContentHashConfig::default(),
                    )?;
```

Find the call at line 165:

```rust
        crate::build::output::copy_static_assets(project_root)?;
```

Replace with:

```rust
        let _ = crate::build::output::copy_static_assets(
            project_root,
            &crate::config::ContentHashConfig::default(),
        )?;
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check`

Expected: No compilation errors. The `manifest` variable in `build()` is created but not yet used beyond the log line -- the compiler may warn about unused `Arc` import or the variable. That is acceptable at this stage; subsequent tasks will consume the manifest.

- [ ] **Step 6: Run the output tests**

Run: `cargo test --lib build::output -- --nocapture`

Expected: All existing output tests pass with the new signature.

- [ ] **Step 7: Commit**

```bash
git add src/build/output.rs src/build/render.rs src/dev/rebuild.rs
git commit -m "feat(content-hash): wire Phase 1 into copy_static_assets and update call sites"
```

---

## Task 5: Implement `rewrite_references` (Phase 3) and `rewrite_file_content`

**Depends on:** Task 2 (needs `AssetManifest`)

**Files:**
- Modify: `src/build/content_hash.rs`

This task implements the post-render rewrite pass that scans `.html`, `.css`, and `.js` files in `dist/` and replaces original asset paths with their hashed equivalents.

- [ ] **Step 1: Add `rewrite_file_content` helper**

Add the following private function to `src/build/content_hash.rs`, above the `build_manifest` function:

```rust
/// Rewrite asset references in a single file's content.
///
/// Performs string replacement of all original paths with their hashed
/// equivalents, using longest-match-first ordering to prevent partial
/// matches.
///
/// Returns the rewritten content, or `None` if no replacements were made
/// (to avoid unnecessary file writes).
fn rewrite_file_content(content: &str, manifest: &AssetManifest) -> Option<String> {
    let pairs = manifest.pairs_longest_first();
    let mut result = content.to_string();
    let mut changed = false;

    for (original, hashed) in &pairs {
        if result.contains(original) {
            result = result.replace(original, hashed);
            changed = true;
        }
    }

    if changed { Some(result) } else { None }
}
```

- [ ] **Step 2: Implement `rewrite_references`**

Replace the stub `rewrite_references` function with:

```rust
/// Rewrite all references to static assets in rendered HTML, CSS, and
/// JS files in dist_dir.
///
/// Walks all `.html`, `.css`, and `.js` files in dist_dir and replaces
/// occurrences of original asset paths with their hashed equivalents.
///
/// Fragment HTML files in `_fragments/` ARE rewritten.
pub fn rewrite_references(dist_dir: &Path, manifest: &AssetManifest) -> Result<()> {
    use walkdir::WalkDir;

    if manifest.is_empty() {
        return Ok(());
    }

    for entry in WalkDir::new(dist_dir).into_iter() {
        let entry = entry.wrap_err("Failed to read entry while rewriting asset references")?;

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Only process text files that may contain asset references.
        if ext != "html" && ext != "css" && ext != "js" {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Content hash rewrite: failed to read {}: {}",
                    path.display(),
                    e,
                );
                continue;
            }
        };

        if let Some(rewritten) = rewrite_file_content(&content, manifest) {
            std::fs::write(path, rewritten).wrap_err_with(|| {
                format!(
                    "Content hash rewrite: failed to write {}",
                    path.display(),
                )
            })?;
        }
    }

    Ok(())
}
```

- [ ] **Step 3: Write tests for `rewrite_file_content` and `rewrite_references`**

Add these tests to the `#[cfg(test)] mod tests` block in `src/build/content_hash.rs`:

```rust
    // --- rewrite_file_content ---

    #[test]
    fn test_rewrite_file_content_html() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());
        m.insert("/js/app.js".into(), "/js/app.def456.js".into());

        let html = r#"<link href="/css/style.css"><script src="/js/app.js"></script>"#;
        let result = rewrite_file_content(html, &m).unwrap();
        assert!(result.contains("/css/style.abc123.css"));
        assert!(result.contains("/js/app.def456.js"));
        assert!(!result.contains(r#""/css/style.css""#));
    }

    #[test]
    fn test_rewrite_file_content_css_url() {
        let mut m = AssetManifest::new();
        m.insert("/images/icon.png".into(), "/images/icon.abc123.png".into());

        let css = r#".icon { background-image: url('/images/icon.png'); }"#;
        let result = rewrite_file_content(css, &m).unwrap();
        assert!(result.contains("/images/icon.abc123.png"));
    }

    #[test]
    fn test_rewrite_file_content_no_match() {
        let m = AssetManifest::new();
        let html = "<h1>Hello</h1>";
        assert!(rewrite_file_content(html, &m).is_none());
    }

    #[test]
    fn test_rewrite_file_content_no_match_with_entries() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        let html = "<h1>Hello</h1>";
        assert!(rewrite_file_content(html, &m).is_none());
    }

    #[test]
    fn test_rewrite_file_content_multiple_refs() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        let html = r#"<link href="/css/style.css"><link href="/css/style.css">"#;
        let result = rewrite_file_content(html, &m).unwrap();
        // Both occurrences should be replaced.
        assert!(!result.contains(r#""/css/style.css""#));
        assert_eq!(result.matches("/css/style.abc123.css").count(), 2);
    }

    #[test]
    fn test_rewrite_file_content_idempotent() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        let html = r#"<link href="/css/style.css">"#;
        let first = rewrite_file_content(html, &m).unwrap();
        // Second pass should produce no changes.
        assert!(rewrite_file_content(&first, &m).is_none());
    }

    // --- rewrite_references ---

    #[test]
    fn test_rewrite_references_html() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "index.html", r#"<link href="/css/style.css">"#);

        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        rewrite_references(dist, &m).unwrap();

        let content = std::fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(content.contains("/css/style.abc123.css"));
    }

    #[test]
    fn test_rewrite_references_css() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", r#"body { background: url('/images/bg.png'); }"#);

        let mut m = AssetManifest::new();
        m.insert("/images/bg.png".into(), "/images/bg.abc123.png".into());

        rewrite_references(dist, &m).unwrap();

        let content = std::fs::read_to_string(dist.join("css/style.css")).unwrap();
        assert!(content.contains("/images/bg.abc123.png"));
    }

    #[test]
    fn test_rewrite_references_js() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "js/app.js", r#"const url = '/images/logo.png';"#);

        let mut m = AssetManifest::new();
        m.insert("/images/logo.png".into(), "/images/logo.abc123.png".into());

        rewrite_references(dist, &m).unwrap();

        let content = std::fs::read_to_string(dist.join("js/app.js")).unwrap();
        assert!(content.contains("/images/logo.abc123.png"));
    }

    #[test]
    fn test_rewrite_references_skips_binary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        // Write a .png file -- should NOT be processed.
        write_file(dist, "images/logo.png", "binary-data-/css/style.css");

        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        rewrite_references(dist, &m).unwrap();

        // The .png file should be untouched.
        let content = std::fs::read_to_string(dist.join("images/logo.png")).unwrap();
        assert!(content.contains("/css/style.css"));
    }

    #[test]
    fn test_rewrite_references_fragments() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "_fragments/about.html", r#"<img src="/images/photo.jpg">"#);

        let mut m = AssetManifest::new();
        m.insert("/images/photo.jpg".into(), "/images/photo.abc123.jpg".into());

        rewrite_references(dist, &m).unwrap();

        let content = std::fs::read_to_string(dist.join("_fragments/about.html")).unwrap();
        assert!(content.contains("/images/photo.abc123.jpg"));
    }

    #[test]
    fn test_rewrite_references_empty_manifest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "index.html", "<h1>Hello</h1>");

        let m = AssetManifest::new();
        // Should be a no-op, not an error.
        rewrite_references(dist, &m).unwrap();

        let content = std::fs::read_to_string(dist.join("index.html")).unwrap();
        assert_eq!(content, "<h1>Hello</h1>");
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib build::content_hash -- --nocapture`

Expected: All tests pass including the new rewrite tests.

- [ ] **Step 5: Commit**

```bash
git add src/build/content_hash.rs
git commit -m "feat(content-hash): implement rewrite_references (Phase 3 - post-render rewrite)"
```

---

## Task 6: Wire Phase 2 into the template engine (`asset()` function)

**Depends on:** Task 4 (needs manifest variable in `build()`)

**Files:**
- Modify: `src/template/functions.rs`
- Modify: `src/template/environment.rs`

This task updates the `asset()` template function to look up hashed paths from the manifest, and threads the manifest through `setup_environment` to `register_functions`.

- [ ] **Step 1: Update `register_functions` signature in `src/template/functions.rs`**

Add the import at the top of the file:

```rust
use std::sync::Arc;
use crate::build::content_hash::AssetManifest;
```

Change the function signature from:

```rust
pub fn register_functions(env: &mut Environment<'_>, config: &SiteConfig) {
```

To:

```rust
pub fn register_functions(
    env: &mut Environment<'_>,
    config: &SiteConfig,
    manifest: Option<Arc<AssetManifest>>,
) {
```

- [ ] **Step 2: Update the `asset()` closure**

Replace the current `asset()` registration (lines 54-60):

```rust
    // asset(path)
    // For now this is a simple pass-through; in the future it could add
    // cache-busting hashes.
    env.add_function("asset", |path: &str| -> String {
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        }
    });
```

With:

```rust
    // asset(path)
    // Returns the content-hashed path when a manifest is available,
    // otherwise passes through unchanged.
    let manifest_clone = manifest;
    env.add_function("asset", move |path: &str| -> String {
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        match &manifest_clone {
            Some(m) => m.resolve(&normalized).to_string(),
            None => normalized,
        }
    });
```

- [ ] **Step 3: Update `setup_environment` in `src/template/environment.rs`**

Add the import at the top:

```rust
use std::sync::Arc;
use crate::build::content_hash::AssetManifest;
```

Change the function signature from:

```rust
pub fn setup_environment(
    project_root: &Path,
    config: &SiteConfig,
    pages: &[PageDef],
    plugin_registry: Option<&PluginRegistry>,
) -> Result<Environment<'static>> {
```

To:

```rust
pub fn setup_environment(
    project_root: &Path,
    config: &SiteConfig,
    pages: &[PageDef],
    plugin_registry: Option<&PluginRegistry>,
    manifest: Option<Arc<AssetManifest>>,
) -> Result<Environment<'static>> {
```

Update the `register_functions` call (around line 89) from:

```rust
    functions::register_functions(&mut env, config);
```

To:

```rust
    functions::register_functions(&mut env, config, manifest);
```

- [ ] **Step 4: Update the production build call site in `src/build/render.rs`**

In `build()`, find the current `setup_environment` call (around line 76):

```rust
    let env = template::setup_environment(project_root, &config, &pages, Some(&plugin_registry))?;
```

Replace with:

```rust
    // Phase 2: Setup template engine (pass manifest for asset() function).
    let env = template::setup_environment(
        project_root,
        &config,
        &pages,
        Some(&plugin_registry),
        if config.build.content_hash.enabled {
            Some(manifest.clone())
        } else {
            None
        },
    )?;
```

- [ ] **Step 5: Update the dev server call site in `src/dev/rebuild.rs`**

Find the `setup_environment` call (around line 168):

```rust
        let env = template::setup_environment(project_root, config, &pages, Some(&self.plugin_registry))?;
```

Replace with:

```rust
        let env = template::setup_environment(
            project_root,
            config,
            &pages,
            Some(&self.plugin_registry),
            None, // No content hashing in dev mode.
        )?;
```

- [ ] **Step 6: Update existing tests in `src/template/functions.rs`**

All existing tests call `register_functions(&mut env, &config)` with two arguments. Update each call to pass `None` for the manifest:

Replace every occurrence of:
```rust
        register_functions(&mut env, &config);
```

With:
```rust
        register_functions(&mut env, &config, None);
```

There are 8 calls in the test module: in `test_link_to_default` (line 170), `test_link_to_custom_target` (line 187), `test_link_to_custom_block` (line 201), `test_link_to_no_fragments` (line 215), `test_current_year` (line 232), `test_asset_with_leading_slash` (line 249), `test_asset_without_leading_slash` (line 262), and `test_site_global` (line 277).

- [ ] **Step 7: Update existing tests in `src/template/environment.rs`**

All existing calls to `setup_environment` need the new manifest parameter. Update each call:

Replace every occurrence of:
```rust
        let env = setup_environment(root, &config, &pages, None).unwrap();
```

With:
```rust
        let env = setup_environment(root, &config, &pages, None, None).unwrap();
```

And:
```rust
        let env = setup_environment(root, &config, &pages, Some(&registry)).unwrap();
```

With:
```rust
        let env = setup_environment(root, &config, &pages, Some(&registry), None).unwrap();
```

There are 4 calls total in the test module: `test_setup_environment_basic` (line 232), `test_strict_undefined_behavior` (line 260), `test_setup_environment_with_plugin_registry` (line 305), and `test_setup_environment_without_plugin_registry` (line 328).

- [ ] **Step 8: Write new tests for `asset()` with manifest**

Add these tests to the existing `#[cfg(test)] mod tests` block in `src/template/functions.rs`:

```rust
    // --- asset with manifest ---

    #[test]
    fn test_asset_with_manifest() {
        let mut env = Environment::new();
        let config = test_config();
        let mut manifest = AssetManifest::new();
        manifest.insert("/css/style.css".into(), "/css/style.abc123.css".into());
        let manifest = Arc::new(manifest);

        register_functions(&mut env, &config, Some(manifest));

        env.add_template("test", "{{ asset('/css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.abc123.css");
    }

    #[test]
    fn test_asset_without_manifest() {
        let mut env = Environment::new();
        let config = test_config();
        register_functions(&mut env, &config, None);

        env.add_template("test", "{{ asset('/css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.css");
    }

    #[test]
    fn test_asset_unknown_path_with_manifest() {
        let mut env = Environment::new();
        let config = test_config();
        let manifest = Arc::new(AssetManifest::new());

        register_functions(&mut env, &config, Some(manifest));

        env.add_template("test", "{{ asset('/unknown.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/unknown.css");
    }

    #[test]
    fn test_asset_normalizes_then_resolves() {
        let mut env = Environment::new();
        let config = test_config();
        let mut manifest = AssetManifest::new();
        manifest.insert("/css/style.css".into(), "/css/style.abc123.css".into());
        let manifest = Arc::new(manifest);

        register_functions(&mut env, &config, Some(manifest));

        // Path without leading slash should be normalized and then resolved.
        env.add_template("test", "{{ asset('css/style.css') }}")
            .unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {}).unwrap();
        assert_eq!(result.trim(), "/css/style.abc123.css");
    }
```

- [ ] **Step 9: Run all template tests**

Run: `cargo test --lib template -- --nocapture`

Expected: All existing tests pass plus the 4 new manifest tests pass.

- [ ] **Step 10: Commit**

```bash
git add src/template/functions.rs src/template/environment.rs src/build/render.rs src/dev/rebuild.rs
git commit -m "feat(content-hash): wire Phase 2 into asset() template function"
```

---

## Task 7: Wire Phase 3 into the build pipeline

**Depends on:** Tasks 4 and 5 (needs manifest in `build()` and `rewrite_references` implemented)

**Files:**
- Modify: `src/build/render.rs`

This task adds the Phase 3 rewrite call to the end of the `build()` function, after post-build plugin hooks.

- [ ] **Step 1: Add Phase 3 call in `build()` function**

In `src/build/render.rs`, find the post-build hooks section (around line 172):

```rust
    // Run post-build hooks
    plugin_registry.post_build(&dist_dir, project_root)?;
```

Add Phase 3 immediately after:

```rust
    // Phase 3: Rewrite remaining asset references in HTML/CSS/JS.
    if config.build.content_hash.enabled && !manifest.is_empty() {
        content_hash::rewrite_references(&dist_dir, &manifest)?;
        tracing::info!("Asset references rewritten.");
    }
```

- [ ] **Step 2: Add the `content_hash` import**

At the top of `src/build/render.rs`, add `use super::content_hash;` to the `use super::` block (insert before `context` to maintain alphabetical order). The file uses `use super::` imports for all other build modules, so this is required for consistency:

```rust
use super::content_hash;
use super::context::{self, PageMeta};
use super::critical_css;
```

Then update the Phase 3 call from Step 1 to use the short form:

```rust
        content_hash::rewrite_references(&dist_dir, &manifest)?;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`

Expected: No compilation errors. The full pipeline now has all three phases wired in.

- [ ] **Step 4: Commit**

```bash
git add src/build/render.rs
git commit -m "feat(content-hash): wire Phase 3 rewrite into build pipeline"
```

---

## Task 8: Add manifest fallback to critical CSS

**Depends on:** Tasks 6 and 7 (needs manifest threaded through the pipeline and `content_hash` import in `render.rs`)

**Files:**
- Modify: `src/build/critical_css/mod.rs`
- Modify: `src/build/render.rs`

This task adds an optional `AssetManifest` parameter to `inline_critical_css`, `get_or_load`, and `load_stylesheet` so that critical CSS can find stylesheets that have been renamed by Phase 1 when templates hardcode the original path.

- [ ] **Step 1: Update `load_stylesheet` in `src/build/critical_css/mod.rs`**

Change the `load_stylesheet` function signature (around line 150) from:

```rust
fn load_stylesheet(href: &str, dist_dir: &Path) -> Result<String, String> {
```

To:

```rust
fn load_stylesheet(
    href: &str,
    dist_dir: &Path,
    manifest: Option<&crate::build::content_hash::AssetManifest>,
) -> Result<String, String> {
```

After the existing file-not-found check:

```rust
    if !css_path.exists() {
        return Err(format!("File not found: {}", css_path.display()));
    }
```

Replace with:

```rust
    if !css_path.exists() {
        // Try resolving through manifest (file may have been renamed by content hashing).
        if let Some(m) = manifest {
            let url_path = if href.starts_with('/') {
                href.to_string()
            } else {
                format!("/{}", href)
            };
            let resolved = m.resolve(&url_path);
            if resolved != url_path {
                let resolved_relative = resolved.trim_start_matches('/');
                let resolved_path = dist_dir.join(resolved_relative);
                if resolved_path.exists() {
                    let css = std::fs::read_to_string(&resolved_path)
                        .map_err(|e| format!("Failed to read {}: {e}", resolved_path.display()))?;
                    return resolve_imports(&css, &resolved_path, dist_dir, 0);
                }
            }
        }
        return Err(format!("File not found: {}", css_path.display()));
    }
```

- [ ] **Step 2: Update `get_or_load` in `StylesheetCache`**

Change the signature from:

```rust
    fn get_or_load(&mut self, href: &str, dist_dir: &Path) -> Option<&str> {
```

To:

```rust
    fn get_or_load(
        &mut self,
        href: &str,
        dist_dir: &Path,
        manifest: Option<&crate::build::content_hash::AssetManifest>,
    ) -> Option<&str> {
```

Update the `load_stylesheet` call inside from:

```rust
            match load_stylesheet(href, dist_dir) {
```

To:

```rust
            match load_stylesheet(href, dist_dir, manifest) {
```

- [ ] **Step 3: Update `inline_critical_css` signature**

Change the signature from:

```rust
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
    css_cache: &mut StylesheetCache,
) -> String {
```

To:

```rust
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
    css_cache: &mut StylesheetCache,
    manifest: Option<&crate::build::content_hash::AssetManifest>,
) -> String {
```

Update the `css_cache.get_or_load` call inside the function (around line 93) from:

```rust
        let css_content = match css_cache.get_or_load(href, dist_dir) {
```

To:

```rust
        let css_content = match css_cache.get_or_load(href, dist_dir, manifest) {
```

- [ ] **Step 4: Update critical CSS call sites in `src/build/render.rs`**

In `render_static_page`, find the critical CSS block (around lines 321-330):

```rust
    let full_html = if config.build.critical_css.enabled {
        critical_css::inline_critical_css(
            &full_html,
            &config.build.critical_css,
            dist_dir,
            css_cache,
        )
    } else {
        full_html
    };
```

Replace with:

```rust
    let full_html = if config.build.critical_css.enabled {
        critical_css::inline_critical_css(
            &full_html,
            &config.build.critical_css,
            dist_dir,
            css_cache,
            if manifest.is_empty() { None } else { Some(manifest.as_ref()) },
        )
    } else {
        full_html
    };
```

This requires `manifest` to be accessible in `render_static_page`. Add it as a parameter to the function. Change the signature from:

```rust
fn render_static_page(
    page: &PageDef,
    env: &minijinja::Environment<'_>,
    fetcher: &mut DataFetcher,
    global_data: &HashMap<String, serde_json::Value>,
    config: &SiteConfig,
    dist_dir: &Path,
    build_time: &str,
    output_paths: &mut HashSet<String>,
    data_query_count: &mut u32,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    plugin_registry: &PluginRegistry,
    image_cache: &ImageCache,
    css_cache: &mut critical_css::StylesheetCache,
) -> Result<RenderedPage> {
```

To:

```rust
fn render_static_page(
    page: &PageDef,
    env: &minijinja::Environment<'_>,
    fetcher: &mut DataFetcher,
    global_data: &HashMap<String, serde_json::Value>,
    config: &SiteConfig,
    dist_dir: &Path,
    build_time: &str,
    output_paths: &mut HashSet<String>,
    data_query_count: &mut u32,
    asset_cache: &mut AssetCache,
    asset_client: &reqwest::blocking::Client,
    plugin_registry: &PluginRegistry,
    image_cache: &ImageCache,
    css_cache: &mut critical_css::StylesheetCache,
    manifest: &Arc<content_hash::AssetManifest>,
) -> Result<RenderedPage> {
```

Do the same for `render_dynamic_page` (signature at line 411) -- add `manifest: &Arc<content_hash::AssetManifest>` as the last parameter (after `css_cache` on line 425), and update its critical CSS call (lines 585-591) identically:

```rust
    let full_html = if config.build.critical_css.enabled {
        critical_css::inline_critical_css(
            &full_html,
            &config.build.critical_css,
            dist_dir,
            css_cache,
            if manifest.is_empty() { None } else { Some(manifest.as_ref()) },
        )
    } else {
        full_html
    };
```

- [ ] **Step 5: Update the call sites for `render_static_page` and `render_dynamic_page` in `build()`**

In the `build()` function, update the calls to pass the manifest. In the static page branch (around line 127):

```rust
                let result = render_static_page(
                    page,
                    &env,
                    &mut fetcher,
                    &global_data,
                    &config,
                    &dist_dir,
                    &build_time,
                    &mut output_paths,
                    &mut data_query_count,
                    &mut asset_cache,
                    &asset_client,
                    &plugin_registry,
                    &image_cache,
                    &mut css_cache,
                    &manifest,
                )?;
```

And similarly for the dynamic page branch (around line 146):

```rust
                let results = render_dynamic_page(
                    page,
                    &env,
                    &mut fetcher,
                    &global_data,
                    &config,
                    &dist_dir,
                    &build_time,
                    &mut output_paths,
                    &mut data_query_count,
                    &mut asset_cache,
                    &asset_client,
                    &plugin_registry,
                    &image_cache,
                    &mut css_cache,
                    &manifest,
                )?;
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check`

Expected: No compilation errors.

- [ ] **Step 7: Update existing critical CSS tests for new signatures**

Existing tests in `src/build/critical_css/mod.rs` call `inline_critical_css` and `get_or_load` directly and need the new `manifest` parameter added (pass `None`).

There are **8 calls** to `inline_critical_css` in the test module (lines 258, 267, 277, 296, 321, 342, 363, 388). Add `, None` after `&mut cache` in each call.

There are **2 calls** to `get_or_load` in the test module (lines 406 and 412 in `test_stylesheet_cache_reuse`). Add `, None` after `dist` in each call.

- [ ] **Step 8: Write tests for manifest fallback in critical CSS**

Add these tests to the `#[cfg(test)]` block in `src/build/critical_css/mod.rs`:

```rust
    #[test]
    fn test_get_or_load_with_manifest_fallback() {
        use crate::build::content_hash::AssetManifest;

        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        // File exists at hashed path, not at original.
        let css = "body { color: red; }";
        let hashed_dir = dist.join("css");
        std::fs::create_dir_all(&hashed_dir).unwrap();
        std::fs::write(hashed_dir.join("style.abc123.css"), css).unwrap();

        let mut manifest = AssetManifest::new();
        manifest.insert(
            "/css/style.css".into(),
            "/css/style.abc123.css".into(),
        );

        let mut cache = StylesheetCache::new();
        let result = cache.get_or_load("/css/style.css", dist, Some(&manifest));
        assert!(result.is_some(), "should resolve via manifest fallback");
        assert_eq!(result.unwrap(), css);
    }

    #[test]
    fn test_get_or_load_without_manifest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        // File exists at original path.
        let css = "body { color: blue; }";
        let css_dir = dist.join("css");
        std::fs::create_dir_all(&css_dir).unwrap();
        std::fs::write(css_dir.join("style.css"), css).unwrap();

        let mut cache = StylesheetCache::new();
        let result = cache.get_or_load("/css/style.css", dist, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), css);
    }
```

- [ ] **Step 9: Run critical CSS tests**

Run: `cargo test --lib build::critical_css -- --nocapture`

Expected: All existing critical CSS tests pass plus the 2 new manifest fallback tests.

- [ ] **Step 10: Commit**

```bash
git add src/build/critical_css/mod.rs src/build/render.rs
git commit -m "feat(content-hash): add manifest fallback to critical CSS stylesheet loading"
```

---

## Task 9: Write feature documentation

**Depends on:** Tasks 1-8

**Files:**
- Create: `docs/content_hashing.md`

- [ ] **Step 1: Write the documentation**

Create `docs/content_hashing.md`:

```markdown
# Content Hashing for Static Assets

## Overview

Content hashing fingerprints static assets (CSS, JS, images, fonts) by
embedding a SHA-256 hash in the filename. This enables browsers and CDNs
to cache assets indefinitely with `Cache-Control: immutable`.

Example: `style.css` becomes `style.a1b2c3d4e5f67890.css`

## Configuration

Add to `site.toml`:

```toml
[build.content_hash]
enabled = true
```

### Custom exclusions

```toml
[build.content_hash]
enabled = true
exclude = [
    "favicon.ico",
    "robots.txt",
    "CNAME",
    "_headers",
    "_redirects",
    ".well-known/**",
    "sw.js",           # service workers need stable URLs
    "manifest.json",   # PWA manifests need stable URLs
]
```

## How It Works

### Three Phases

1. **Phase 1 (Build Manifest):** During `copy_static_assets`, files in
   `dist/` are hashed and renamed. A manifest maps original paths to
   hashed paths.

2. **Phase 2 (Template Resolution):** The `asset()` template function
   uses the manifest to return hashed paths at render time.

3. **Phase 3 (Post-Render Rewrite):** After all rendering, a final pass
   rewrites any remaining hardcoded asset references in `.html`, `.css`,
   and `.js` files.

### Template Usage

Always use the `asset()` function for static asset references:

```html
<link rel="stylesheet" href="{{ asset('/css/style.css') }}">
<script src="{{ asset('/js/app.js') }}"></script>
<img src="{{ asset('/images/logo.png') }}" alt="Logo">
```

Hardcoded paths also work (Phase 3 rewrites them), but `asset()` is
preferred because it gives templates the correct URL at render time.

## Default Exclusions

These files are excluded from hashing by default (they must keep stable
names for deployment platforms):

- `favicon.ico`
- `robots.txt`
- `CNAME`
- `_headers`
- `_redirects`
- `.well-known/**`

## Dev Server

Content hashing is always disabled during dev server operation. The
`asset()` function returns paths unchanged in dev mode.

## Interactions

- **Image optimization:** Works correctly. Use `asset()` for source
  image paths; image optimization reads the hashed filename from dist.
- **Critical CSS:** Enhanced to resolve hashed paths via manifest
  fallback when the original file path is not found.
- **Resource hints:** Preload/prefetch URLs naturally use hashed paths
  when templates use `asset()`.
- **Asset localization:** Downloaded assets are not hashed (they already
  have content-based filenames).

## Limitations

- Relative CSS `url()` paths (e.g., `../images/icon.png`) are not
  rewritten. Use absolute paths in CSS files.
- Dynamic JavaScript references constructed at runtime cannot be
  statically rewritten.
- Source map relative references may not be rewritten. Use absolute
  paths or exclude `*.map` files.

## Module Location

`src/build/content_hash.rs` -- single-file module containing
`AssetManifest`, `build_manifest()`, and `rewrite_references()`.
```

- [ ] **Step 2: Commit**

```bash
git add docs/content_hashing.md
git commit -m "docs: add content hashing feature documentation"
```

---

## Task 10: Integration test and final verification

**Depends on:** Tasks 1-9

**Files:**
- Modify: `src/build/content_hash.rs` (add integration-style test)

This task adds a comprehensive end-to-end test that exercises all three phases together, and runs the full test suite.

- [ ] **Step 1: Write end-to-end test**

Add this test to the `#[cfg(test)] mod tests` block in `src/build/content_hash.rs`:

```rust
    // --- end-to-end ---

    #[test]
    fn test_end_to_end_hash_and_rewrite() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let static_dir = root.join("static");
        let dist_dir = root.join("dist");

        // Set up static files.
        let css_content = "body { color: red; }";
        let js_content = "console.log('hello');";
        write_file(&static_dir, "css/style.css", css_content);
        write_file(&static_dir, "js/app.js", js_content);
        write_file(&static_dir, "favicon.ico", "icon-data");

        // Copy to dist (simulating copy_static_assets).
        write_file(&dist_dir, "css/style.css", css_content);
        write_file(&dist_dir, "js/app.js", js_content);
        write_file(&dist_dir, "favicon.ico", "icon-data");

        // Phase 1: Build manifest.
        let config = hash_config();
        let manifest = build_manifest(&dist_dir, &static_dir, &config).unwrap();

        // favicon.ico should be excluded.
        assert_eq!(manifest.len(), 2);
        assert!(dist_dir.join("favicon.ico").exists());

        // CSS and JS should be hashed.
        let css_hashed = manifest.resolve("/css/style.css");
        let js_hashed = manifest.resolve("/js/app.js");
        assert_ne!(css_hashed, "/css/style.css");
        assert_ne!(js_hashed, "/js/app.js");

        // Write an HTML file that references the original paths (simulating
        // a template that does not use asset()).
        let html = format!(
            r#"<html><head><link href="/css/style.css"><script src="/js/app.js"></script></head></html>"#
        );
        write_file(&dist_dir, "index.html", &html);

        // Phase 3: Rewrite references.
        rewrite_references(&dist_dir, &manifest).unwrap();

        // Verify HTML was rewritten.
        let result = std::fs::read_to_string(dist_dir.join("index.html")).unwrap();
        assert!(result.contains(css_hashed), "CSS reference should be rewritten");
        assert!(result.contains(js_hashed), "JS reference should be rewritten");
        assert!(!result.contains(r#"href="/css/style.css""#), "original CSS ref should be gone");
        assert!(!result.contains(r#"src="/js/app.js""#), "original JS ref should be gone");
    }
```

- [ ] **Step 2: Run the content_hash tests**

Run: `cargo test --lib build::content_hash -- --nocapture`

Expected: All tests pass, including the new end-to-end test.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test -- --nocapture`

Expected: All tests across all modules pass. No regressions from the signature changes.

- [ ] **Step 4: Commit**

```bash
git add src/build/content_hash.rs
git commit -m "test(content-hash): add end-to-end integration test"
```

---

## Final Verification

After completing all tasks:

- [ ] Run `cargo check` -- no compilation errors
- [ ] Run `cargo test` -- all tests pass
- [ ] Run `cargo clippy` -- no warnings (fix any that arise)
- [ ] Verify the feature is disabled by default: build the example site without `[build.content_hash]` in `site.toml` and confirm `dist/` contains files with original names
- [ ] Verify the feature works when enabled: add `[build.content_hash]\nenabled = true` to the example site's `site.toml`, run `cargo run -- build`, and confirm:
  - Files in `dist/` have hashed names (except excluded files like `favicon.ico`)
  - HTML files reference hashed asset paths
  - The `asset()` function in templates returns hashed paths
- [ ] Verify dev server still works: run `cargo run -- dev` and confirm no errors, assets load correctly without hashes
