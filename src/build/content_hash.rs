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
    pub fn resolve<'a>(&'a self, original: &'a str) -> &'a str {
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
            Some(parent) if !parent.as_os_str().is_empty() => {
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

/// Hash and rename additional generated files (e.g., CSS/JS bundles)
/// and return a manifest of their original-to-hashed path mappings.
///
/// These files exist only in dist/ (no static/ counterpart). The returned
/// manifest is used alongside the main manifest during reference rewriting.
pub fn hash_additional_files(
    dist_dir: &Path,
    relative_paths: &[String],
) -> Result<AssetManifest> {
    let mut manifest = AssetManifest::with_capacity(relative_paths.len());

    for rel_path in relative_paths {
        let file_path = dist_dir.join(rel_path);

        if !file_path.exists() {
            tracing::warn!(
                "Content hash: generated file '{}' not found, skipping",
                file_path.display()
            );
            continue;
        }

        let data = std::fs::read(&file_path)
            .wrap_err_with(|| format!(
                "Failed to read generated file '{}'",
                file_path.display()
            ))?;

        let hash = content_hash(&data);

        let filename = file_path
            .file_name()
            .and_then(|f| f.to_str())
            .ok_or_else(|| eyre::eyre!(
                "Invalid filename: {}", file_path.display()
            ))?;

        let hashed_name = hashed_filename(filename, &hash);
        let hashed_path = file_path.with_file_name(&hashed_name);

        // Rename the file.
        std::fs::rename(&file_path, &hashed_path)
            .wrap_err_with(|| format!(
                "Failed to rename '{}' to '{}'",
                file_path.display(), hashed_path.display()
            ))?;

        // Build URL paths with leading slash.
        let original_url = format!("/{rel_path}");
        let hashed_rel = rel_path.rsplit_once('/')
            .map(|(dir, _)| format!("{dir}/{hashed_name}"))
            .unwrap_or(hashed_name.clone());
        let hashed_url = format!("/{hashed_rel}");

        manifest.insert(original_url, hashed_url);
    }

    Ok(manifest)
}

/// Rewrite asset references in a single file's content.
///
/// Performs string replacement of all original paths with their hashed
/// equivalents, using longest-match-first ordering to prevent partial
/// matches.
///
/// Returns the rewritten content, or `None` if no replacements were made
/// (to avoid unnecessary file writes).
fn rewrite_file_content(content: &str, pairs: &[(&str, &str)]) -> Option<String> {
    let mut result = content.to_string();
    let mut changed = false;

    for (original, hashed) in pairs {
        if result.contains(original) {
            result = result.replace(original, hashed);
            changed = true;
        }
    }

    if changed { Some(result) } else { None }
}

/// Rewrite all references to static assets in rendered HTML, CSS, and
/// JS files in dist_dir.
///
/// Walks all `.html`, `.css`, and `.js` files in dist_dir and replaces
/// occurrences of original asset paths with their hashed equivalents.
///
/// Fragment HTML files in `_fragments/` ARE rewritten.
pub fn rewrite_references(
    dist_dir: &Path,
    manifest: &AssetManifest,
    additional: Option<&AssetManifest>,
) -> Result<()> {
    use walkdir::WalkDir;

    if manifest.is_empty() && additional.map_or(true, |a| a.is_empty()) {
        return Ok(());
    }

    // Build merged replacement pairs from both manifests, sorted longest-first.
    let mut all_pairs = manifest.pairs_longest_first();
    if let Some(add) = additional {
        all_pairs.extend(add.pairs_longest_first());
        all_pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
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

        if let Some(rewritten) = rewrite_file_content(&content, &all_pairs) {
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

    // --- rewrite_file_content ---

    #[test]
    fn test_rewrite_file_content_html() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());
        m.insert("/js/app.js".into(), "/js/app.def456.js".into());

        let pairs = m.pairs_longest_first();
        let html = r#"<link href="/css/style.css"><script src="/js/app.js"></script>"#;
        let result = rewrite_file_content(html, &pairs).unwrap();
        assert!(result.contains("/css/style.abc123.css"));
        assert!(result.contains("/js/app.def456.js"));
        assert!(!result.contains(r#""/css/style.css""#));
    }

    #[test]
    fn test_rewrite_file_content_css_url() {
        let mut m = AssetManifest::new();
        m.insert("/images/icon.png".into(), "/images/icon.abc123.png".into());

        let pairs = m.pairs_longest_first();
        let css = r#".icon { background-image: url('/images/icon.png'); }"#;
        let result = rewrite_file_content(css, &pairs).unwrap();
        assert!(result.contains("/images/icon.abc123.png"));
    }

    #[test]
    fn test_rewrite_file_content_no_match() {
        let m = AssetManifest::new();
        let pairs = m.pairs_longest_first();
        let html = "<h1>Hello</h1>";
        assert!(rewrite_file_content(html, &pairs).is_none());
    }

    #[test]
    fn test_rewrite_file_content_no_match_with_entries() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        let pairs = m.pairs_longest_first();
        let html = "<h1>Hello</h1>";
        assert!(rewrite_file_content(html, &pairs).is_none());
    }

    #[test]
    fn test_rewrite_file_content_multiple_refs() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        let pairs = m.pairs_longest_first();
        let html = r#"<link href="/css/style.css"><link href="/css/style.css">"#;
        let result = rewrite_file_content(html, &pairs).unwrap();
        // Both occurrences should be replaced.
        assert!(!result.contains(r#""/css/style.css""#));
        assert_eq!(result.matches("/css/style.abc123.css").count(), 2);
    }

    #[test]
    fn test_rewrite_file_content_idempotent() {
        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        let pairs = m.pairs_longest_first();
        let html = r#"<link href="/css/style.css">"#;
        let first = rewrite_file_content(html, &pairs).unwrap();
        // Second pass should produce no changes.
        assert!(rewrite_file_content(&first, &pairs).is_none());
    }

    // --- rewrite_references ---

    #[test]
    fn test_rewrite_references_html() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "index.html", r#"<link href="/css/style.css">"#);

        let mut m = AssetManifest::new();
        m.insert("/css/style.css".into(), "/css/style.abc123.css".into());

        rewrite_references(dist, &m, None).unwrap();

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

        rewrite_references(dist, &m, None).unwrap();

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

        rewrite_references(dist, &m, None).unwrap();

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

        rewrite_references(dist, &m, None).unwrap();

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

        rewrite_references(dist, &m, None).unwrap();

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
        rewrite_references(dist, &m, None).unwrap();

        let content = std::fs::read_to_string(dist.join("index.html")).unwrap();
        assert_eq!(content, "<h1>Hello</h1>");
    }

    // --- Hegel property-based tests ---

    use hegel::generators;

    #[hegel::test]
    fn prop_content_hash_determinism(tc: hegel::TestCase) {
        let data: Vec<u8> = tc.draw(generators::binary());
        assert_eq!(content_hash(&data), content_hash(&data));
    }

    #[hegel::test]
    fn prop_content_hash_format_invariant(tc: hegel::TestCase) {
        let data: Vec<u8> = tc.draw(generators::binary());
        let hash = content_hash(&data);
        assert_eq!(hash.len(), 16, "hash must be exactly 16 chars");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "hash must be all lowercase hex, got: {hash}"
        );
    }

    #[hegel::test]
    fn prop_content_hash_collision_resistance(tc: hegel::TestCase) {
        let a: Vec<u8> = tc.draw(generators::binary());
        let b: Vec<u8> = tc.draw(generators::binary());
        tc.assume(a != b);
        assert_ne!(content_hash(&a), content_hash(&b));
    }

    #[hegel::test]
    fn prop_hashed_filename_preserves_extension(tc: hegel::TestCase) {
        let filename: String = tc.draw(generators::from_regex(r"[a-z]{1,10}\.[a-z]{1,5}"));
        let hash: String = tc.draw(generators::from_regex(r"[0-9a-f]{16}"));
        let result = hashed_filename(&filename, &hash);
        let original_ext = filename.rsplit('.').next().unwrap();
        assert!(
            result.ends_with(&format!(".{original_ext}")),
            "expected extension '.{original_ext}' preserved, got: {result}"
        );
    }

    #[hegel::test]
    fn prop_hashed_filename_contains_hash(tc: hegel::TestCase) {
        let filename: String = tc.draw(generators::from_regex(r"[a-z]{1,10}\.[a-z]{1,5}"));
        let hash: String = tc.draw(generators::from_regex(r"[0-9a-f]{16}"));
        let result = hashed_filename(&filename, &hash);
        assert!(
            result.contains(&hash),
            "output '{result}' must contain hash '{hash}'"
        );
    }

    #[hegel::test]
    fn prop_rewrite_file_content_idempotence(tc: hegel::TestCase) {
        let key: String = tc.draw(generators::from_regex(r"/[a-z]{1,8}\.[a-z]{1,4}"));
        let value: String = tc.draw(generators::from_regex(r"/[a-z]{1,8}\.[0-9a-f]{8}\.[a-z]{1,4}"));
        tc.assume(key != value);
        tc.assume(!value.contains(&key));

        let mut manifest = AssetManifest::new();
        manifest.insert(key.clone(), value.clone());
        let pairs = manifest.pairs_longest_first();

        let content = format!("<link href=\"{key}\">");
        let first = rewrite_file_content(&content, &pairs).unwrap();
        // Second pass should find nothing to replace.
        assert!(
            rewrite_file_content(&first, &pairs).is_none(),
            "second rewrite pass should be a no-op"
        );
    }

    #[hegel::test]
    fn prop_asset_manifest_model(tc: hegel::TestCase) {
        let entries: std::collections::HashMap<String, String> = tc.draw(
            generators::hashmaps(
                generators::text().min_size(1).max_size(20),
                generators::text().min_size(1).max_size(20),
            )
            .max_size(20),
        );

        let mut manifest = AssetManifest::new();
        for (k, v) in &entries {
            manifest.insert(k.clone(), v.clone());
        }

        // resolve returns the mapped value for inserted keys.
        for (k, v) in &entries {
            assert_eq!(manifest.resolve(k), v.as_str());
        }

        // resolve returns the original for non-inserted keys.
        let missing = "___key_not_in_manifest___";
        assert_eq!(manifest.resolve(missing), missing);

        // len matches unique inserts.
        assert_eq!(manifest.len(), entries.len());
    }

    #[hegel::test]
    fn prop_pairs_longest_first_sorted(tc: hegel::TestCase) {
        let entries: std::collections::HashMap<String, String> = tc.draw(
            generators::hashmaps(
                generators::text().min_size(1).max_size(30),
                generators::text().min_size(1).max_size(30),
            )
            .max_size(20),
        );

        let mut manifest = AssetManifest::new();
        for (k, v) in &entries {
            manifest.insert(k.clone(), v.clone());
        }

        let pairs = manifest.pairs_longest_first();
        for window in pairs.windows(2) {
            assert!(
                window[0].0.len() >= window[1].0.len(),
                "pairs not sorted by decreasing key length: '{}' (len {}) before '{}' (len {})",
                window[0].0,
                window[0].0.len(),
                window[1].0,
                window[1].0.len(),
            );
        }
    }

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
        let html = r#"<html><head><link href="/css/style.css"><script src="/js/app.js"></script></head></html>"#;
        write_file(&dist_dir, "index.html", html);

        // Phase 3: Rewrite references.
        rewrite_references(&dist_dir, &manifest, None).unwrap();

        // Verify HTML was rewritten.
        let result = std::fs::read_to_string(dist_dir.join("index.html")).unwrap();
        assert!(result.contains(css_hashed), "CSS reference should be rewritten");
        assert!(result.contains(js_hashed), "JS reference should be rewritten");
        assert!(!result.contains(r#"href="/css/style.css""#), "original CSS ref should be gone");
        assert!(!result.contains(r#"src="/js/app.js""#), "original JS ref should be gone");
    }
}
