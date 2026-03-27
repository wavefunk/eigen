# CSS/JS Bundling and Tree-Shaking Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge multiple CSS and JS files into single bundled files, tree-shake unused CSS selectors across the entire site, and rewrite all HTML references to point to the bundles -- reducing HTTP requests, improving compression, and eliminating dead CSS.

**Architecture:** A new `build::bundling` module with four submodules (`collect.rs`, `css.rs`, `js.rs`, `rewrite.rs`) slots into the build pipeline as Phase 2.5 -- after all pages are rendered and post-build plugins have run, but before content hashing. The module scans all rendered HTML files in `dist/` for `<link>` and `<script>` references, merges referenced local CSS/JS files into single bundles, tree-shakes CSS by matching selectors against the union of all rendered HTML DOMs (reusing public primitives from `critical_css/extract.rs`), and rewrites all HTML files to reference the bundled files. The feature is opt-in via `[build.bundling]` in `site.toml`, disabled by default. No new crate dependencies are needed -- all required libraries (`lightningcss`, `scraper`, `lol_html`, `walkdir`, `glob`) are already in `Cargo.toml`.

**Tech Stack:** Rust, lightningcss (CSS parsing/serialization/minification), scraper (HTML DOM + selector matching), lol_html (streaming HTML rewrite), walkdir (directory traversal), glob (exclude pattern matching)

**Design spec:** `docs/superpowers/specs/2026-03-17-css-js-bundling-design.md`

---

## File Structure

### New files to create

| File | Responsibility |
|------|---------------|
| `src/build/bundling/mod.rs` | Public API (`bundle_assets`), orchestration: config checks, call collect/merge/tree-shake/rewrite, write bundle files, return generated file list |
| `src/build/bundling/collect.rs` | Scan all HTML files in `dist/` with `lol_html`, extract `<link rel="stylesheet">` hrefs and `<script src>` srcs, deduplicate in first-encounter order, return `CollectedRefs` |
| `src/build/bundling/css.rs` | CSS merging (`merge_css_files`) and tree-shaking (`tree_shake_css`): concatenate CSS files with source comments, resolve `@import` chains, parse with lightningcss, match selectors against all HTML DOMs using shared primitives from `critical_css/extract.rs` |
| `src/build/bundling/js.rs` | JS concatenation (`merge_js_files`): wrap each file in IIFE, concatenate with source comments and semicolons |
| `src/build/bundling/rewrite.rs` | HTML rewriting (`rewrite_html_for_bundles`): use `lol_html` to replace first bundled `<link>`/`<script>` with bundle reference, remove subsequent ones, handle critical CSS preload pattern and `<noscript>` text replacement |
| `docs/css_js_bundling.md` | Feature documentation for future reference |

### Existing files to modify

| File | Change |
|------|--------|
| `src/config/mod.rs` | Add `BundlingConfig` struct with `enabled`, `css`, `tree_shake_css`, `js`, `css_output`, `js_output`, `exclude` fields; add `bundling` field to `BuildConfig`; update `Default` impl; add config tests |
| `src/build/mod.rs` | Add `pub mod bundling;` declaration |
| `src/build/render.rs` | Insert Phase 2.5 call to `bundling::bundle_assets` between post-build plugin hooks and content hashing; update content hashing integration to handle generated bundle files |
| `src/build/content_hash.rs` | Add `hash_additional_files` function; change `rewrite_references` signature to accept optional additional manifest |

---

## Task 1: Add `BundlingConfig` to configuration

**Depends on:** Nothing (starting point)

**Files:**
- Modify: `src/config/mod.rs`

This task adds the configuration struct and integrates it into `BuildConfig`. No new dependencies are needed.

- [ ] **Step 1: Add `BundlingConfig` struct to `src/config/mod.rs`**

Add the following after the `ContentHashConfig` struct and its `Default` impl (after line 316, before the `SourceConfig` struct). The `default_true` function already exists at line 77 and is reused.

```rust
/// Configuration for CSS/JS bundling and tree-shaking.
///
/// Located under `[build.bundling]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct BundlingConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Whether to bundle CSS files. Default: true (when bundling is enabled).
    #[serde(default = "default_true")]
    pub css: bool,

    /// Whether to tree-shake CSS (remove unused selectors). Default: true.
    /// Only applies when CSS bundling is enabled.
    /// When false, CSS files are merged but no selectors are removed.
    #[serde(default = "default_true")]
    pub tree_shake_css: bool,

    /// Whether to bundle JS files. Default: true (when bundling is enabled).
    #[serde(default = "default_true")]
    pub js: bool,

    /// Output filename for the bundled CSS file.
    /// Written to `dist/{css_output}`. Default: "css/bundle.css".
    #[serde(default = "default_css_output")]
    pub css_output: String,

    /// Output filename for the bundled JS file.
    /// Written to `dist/{js_output}`. Default: "js/bundle.js".
    #[serde(default = "default_js_output")]
    pub js_output: String,

    /// Glob patterns for stylesheet/script paths to exclude from bundling.
    /// Matched against the href/src value. Excluded files remain as
    /// separate `<link>`/`<script>` tags in the HTML.
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_css_output() -> String {
    "css/bundle.css".to_string()
}

fn default_js_output() -> String {
    "js/bundle.js".to_string()
}

impl Default for BundlingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            css: true,
            tree_shake_css: true,
            js: true,
            css_output: default_css_output(),
            js_output: default_js_output(),
            exclude: Vec::new(),
        }
    }
}
```

- [ ] **Step 2: Add `bundling` field to `BuildConfig`**

In `src/config/mod.rs`, add a new field to the `BuildConfig` struct (after the `content_hash` field, around line 59):

```rust
    /// CSS/JS bundling and tree-shaking configuration.
    #[serde(default)]
    pub bundling: BundlingConfig,
```

Update the `Default` impl for `BuildConfig` (around line 62) to include:

```rust
impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            fragments: true,
            fragment_dir: default_fragment_dir(),
            content_block: default_content_block(),
            oob_blocks: Vec::new(),
            minify: true,
            critical_css: CriticalCssConfig::default(),
            hints: HintsConfig::default(),
            content_hash: ContentHashConfig::default(),
            bundling: BundlingConfig::default(),
        }
    }
}
```

- [ ] **Step 3: Write tests for the new config types**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/config/mod.rs`:

```rust
    // --- Bundling config tests ---

    #[test]
    fn test_bundling_config_defaults() {
        let toml_str = r#"
[site]
name = "Bundle Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.bundling.enabled);
        assert!(config.build.bundling.css);
        assert!(config.build.bundling.tree_shake_css);
        assert!(config.build.bundling.js);
        assert_eq!(config.build.bundling.css_output, "css/bundle.css");
        assert_eq!(config.build.bundling.js_output, "js/bundle.js");
        assert!(config.build.bundling.exclude.is_empty());
    }

    #[test]
    fn test_bundling_config_enabled_only() {
        let toml_str = r#"
[site]
name = "Bundle Enabled"
base_url = "https://example.com"

[build.bundling]
enabled = true
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.bundling.enabled);
        // Other fields should have defaults.
        assert!(config.build.bundling.css);
        assert!(config.build.bundling.tree_shake_css);
        assert!(config.build.bundling.js);
        assert_eq!(config.build.bundling.css_output, "css/bundle.css");
    }

    #[test]
    fn test_bundling_config_custom() {
        let toml_str = r#"
[site]
name = "Bundle Custom"
base_url = "https://example.com"

[build.bundling]
enabled = true
css = true
tree_shake_css = false
js = false
css_output = "assets/styles.css"
js_output = "assets/scripts.js"
exclude = ["**/vendor/**", "**/print.css"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.bundling.enabled);
        assert!(config.build.bundling.css);
        assert!(!config.build.bundling.tree_shake_css);
        assert!(!config.build.bundling.js);
        assert_eq!(config.build.bundling.css_output, "assets/styles.css");
        assert_eq!(config.build.bundling.js_output, "assets/scripts.js");
        assert_eq!(config.build.bundling.exclude.len(), 2);
    }

    #[test]
    fn test_bundling_config_css_only() {
        let toml_str = r#"
[site]
name = "CSS Only"
base_url = "https://example.com"

[build.bundling]
enabled = true
js = false
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.bundling.enabled);
        assert!(config.build.bundling.css);
        assert!(!config.build.bundling.js);
    }
```

- [ ] **Step 4: Run tests to verify config changes compile and pass**

Run: `cargo test --lib config -- --nocapture`

Expected: All existing config tests pass, plus the 4 new bundling tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat(bundling): add BundlingConfig to site configuration

Add BundlingConfig struct with enabled, css, tree_shake_css, js,
css_output, js_output, and exclude fields. Wire into BuildConfig
with serde defaults. All fields opt-in (enabled defaults to false)."
```

---

## Task 2: Create `bundling/collect.rs` -- HTML scanning and reference collection

**Depends on:** Task 1 (config types must exist for compilation)

**Files:**
- Create: `src/build/bundling/collect.rs`
- Create: `src/build/bundling/mod.rs` (minimal, to make module compilable)
- Modify: `src/build/mod.rs`

This task implements scanning all HTML files in `dist/` and extracting `<link rel="stylesheet">` hrefs and `<script src>` srcs. References are deduplicated in first-encounter order. HTML files are sorted by path for deterministic ordering.

- [ ] **Step 1: Create `src/build/bundling/mod.rs` with module declarations**

```rust
//! CSS/JS bundling and tree-shaking.
//!
//! Merges multiple CSS/JS files into single bundles, tree-shakes unused
//! CSS selectors, and rewrites HTML references. Runs as Phase 2.5 in the
//! build pipeline (after rendering, before content hashing).

pub mod collect;
pub mod css;
pub mod js;
pub mod rewrite;

use std::path::Path;

use eyre::Result;

use crate::config::BundlingConfig;
```

- [ ] **Step 2: Create placeholder submodule files**

Create `src/build/bundling/css.rs`:

```rust
//! CSS merging and tree-shaking.
```

Create `src/build/bundling/js.rs`:

```rust
//! JS concatenation with IIFE wrapping.
```

Create `src/build/bundling/rewrite.rs`:

```rust
//! HTML rewriting: replace <link> and <script> tags with bundle references.
```

- [ ] **Step 3: Register the module in `src/build/mod.rs`**

Add `pub mod bundling;` to `src/build/mod.rs` at the top of the module list (alphabetical order):

```rust
pub mod bundling;
pub mod content_hash;
pub mod context;
pub mod critical_css;
pub mod fragments;
pub mod hints;
pub mod minify;
pub mod output;
pub mod render;
pub mod sitemap;
```

- [ ] **Step 4: Create `src/build/bundling/collect.rs` with `CollectedRefs` and `collect_references`**

```rust
//! HTML scanning for CSS and JS references.
//!
//! Scans all HTML files in dist/ and extracts `<link rel="stylesheet">`
//! hrefs and `<script src>` srcs, deduplicating in first-encounter order.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eyre::{Result, WrapErr};
use walkdir::WalkDir;

/// Collected references from all rendered HTML files.
pub struct CollectedRefs {
    /// Ordered list of unique local CSS hrefs found across all HTML files,
    /// in first-encounter order.
    pub css_hrefs: Vec<String>,

    /// Ordered list of unique local JS srcs found across all HTML files,
    /// in first-encounter order.
    pub js_srcs: Vec<String>,

    /// Paths to all HTML files in dist/ (full pages and fragments).
    /// Used for rewriting and for tree-shaking DOM parsing.
    pub html_files: Vec<PathBuf>,
}

/// Scan all HTML files in dist/ and collect `<link rel="stylesheet">`
/// hrefs and `<script src="...">` srcs.
///
/// Returns deduplicated lists in first-encounter order across all pages.
/// Also returns the list of HTML file paths for rewriting.
///
/// Skips:
/// - External URLs (http://, https://)
/// - `<link>` tags with a `media` attribute (e.g. print stylesheets)
/// - `<script>` tags with `defer`, `async`, or `type="module"`
/// - `<script>` tags without a `src` attribute (inline scripts)
/// - Paths matching the `exclude` glob patterns
///
/// HTML files are sorted by path before processing to ensure
/// deterministic merge order.
pub fn collect_references(
    dist_dir: &Path,
    exclude: &[String],
) -> Result<CollectedRefs> {
    // Collect and sort all HTML file paths for deterministic ordering.
    let mut html_files: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(dist_dir)
        .sort_by_file_name()
        .into_iter()
    {
        let entry = entry.wrap_err("Failed to read directory entry during reference collection")?;

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("html") {
            html_files.push(path.to_path_buf());
        }
    }

    if html_files.is_empty() {
        return Ok(CollectedRefs {
            css_hrefs: Vec::new(),
            js_srcs: Vec::new(),
            html_files: Vec::new(),
        });
    }

    let mut css_hrefs: Vec<String> = Vec::new();
    let mut css_seen: HashSet<String> = HashSet::new();
    let mut js_srcs: Vec<String> = Vec::new();
    let mut js_seen: HashSet<String> = HashSet::new();

    // Compile exclude patterns once.
    let exclude_patterns: Vec<glob::Pattern> = exclude
        .iter()
        .filter_map(|p| match glob::Pattern::new(p) {
            Ok(pat) => Some(pat),
            Err(e) => {
                tracing::warn!("Invalid exclude pattern '{}': {}", p, e);
                None
            }
        })
        .collect();

    for html_path in &html_files {
        let html_content = match std::fs::read_to_string(html_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to read HTML file '{}': {}",
                    html_path.display(), e
                );
                continue;
            }
        };

        let page_css: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let page_js: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let page_css_clone = page_css.clone();
        let page_js_clone = page_js.clone();
        let exclude_patterns_css = exclude_patterns.clone();
        let exclude_patterns_js = exclude_patterns.clone();

        let _ = lol_html::rewrite_str(
            &html_content,
            lol_html::RewriteStrSettings {
                element_content_handlers: vec![
                    // Collect CSS hrefs.
                    lol_html::element!("link[rel='stylesheet']", move |el| {
                        // Skip links with media attribute.
                        if el.get_attribute("media").is_some() {
                            return Ok(());
                        }

                        let href = match el.get_attribute("href") {
                            Some(h) if !h.is_empty() => h,
                            _ => return Ok(()),
                        };

                        // Skip external URLs.
                        if href.starts_with("http://") || href.starts_with("https://") {
                            return Ok(());
                        }

                        // Check exclude patterns.
                        if exclude_patterns_css.iter().any(|p| p.matches(&href)) {
                            return Ok(());
                        }

                        page_css_clone.borrow_mut().push(href);
                        Ok(())
                    }),
                    // Collect JS srcs.
                    lol_html::element!("script", move |el| {
                        // Skip inline scripts (no src).
                        let src = match el.get_attribute("src") {
                            Some(s) if !s.is_empty() => s,
                            _ => return Ok(()),
                        };

                        // Skip external URLs.
                        if src.starts_with("http://") || src.starts_with("https://") {
                            return Ok(());
                        }

                        // Skip defer, async, type="module".
                        if el.get_attribute("defer").is_some()
                            || el.get_attribute("async").is_some()
                        {
                            return Ok(());
                        }
                        if let Some(type_attr) = el.get_attribute("type") {
                            if type_attr == "module" {
                                return Ok(());
                            }
                        }

                        // Check exclude patterns.
                        if exclude_patterns_js.iter().any(|p| p.matches(&src)) {
                            return Ok(());
                        }

                        page_js_clone.borrow_mut().push(src);
                        Ok(())
                    }),
                ],
                ..lol_html::RewriteStrSettings::new()
            },
        );

        // Deduplicate: add to ordered lists if not seen before.
        for href in page_css.borrow().iter() {
            if css_seen.insert(href.clone()) {
                css_hrefs.push(href.clone());
            }
        }
        for src in page_js.borrow().iter() {
            if js_seen.insert(src.clone()) {
                js_srcs.push(src.clone());
            }
        }
    }

    Ok(CollectedRefs {
        css_hrefs,
        js_srcs,
        html_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to write a file, creating parent dirs.
    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_collect_single_css() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
        assert!(refs.js_srcs.is_empty());
        assert_eq!(refs.html_files.len(), 1);
    }

    #[test]
    fn test_collect_multiple_css() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/reset.css", "/css/style.css"]);
    }

    #[test]
    fn test_collect_dedup_css() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#);
        write_file(dist, "about.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
        assert_eq!(refs.html_files.len(), 2);
    }

    #[test]
    fn test_collect_skips_external() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="https://cdn.example.com/lib.css">
            <link rel="stylesheet" href="/css/local.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/local.css"]);
    }

    #[test]
    fn test_collect_skips_media() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/print.css" media="print">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
    }

    #[test]
    fn test_collect_skips_excluded() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/vendor/bootstrap.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#);

        let refs = collect_references(dist, &["**/vendor/**".to_string()]).unwrap();
        assert_eq!(refs.css_hrefs, vec!["/css/style.css"]);
    }

    #[test]
    fn test_collect_single_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body><script src="/js/app.js"></script></body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert!(refs.css_hrefs.is_empty());
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_skips_module_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body>
            <script type="module" src="/js/mod.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_skips_async_defer() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body>
            <script defer src="/js/deferred.js"></script>
            <script async src="/js/async.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_skips_inline_script() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", r#"<html><body>
            <script>console.log("inline");</script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let refs = collect_references(dist, &[]).unwrap();
        assert_eq!(refs.js_srcs, vec!["/js/app.js"]);
    }

    #[test]
    fn test_collect_no_html_files() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        let refs = collect_references(dist, &[]).unwrap();
        assert!(refs.css_hrefs.is_empty());
        assert!(refs.js_srcs.is_empty());
        assert!(refs.html_files.is_empty());
    }

    #[test]
    fn test_collect_deterministic_order() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        // Two pages with different CSS sets -- order should be deterministic.
        write_file(dist, "a.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/a.css">
        </head><body></body></html>"#);
        write_file(dist, "b.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/b.css">
        </head><body></body></html>"#);

        let refs1 = collect_references(dist, &[]).unwrap();
        let refs2 = collect_references(dist, &[]).unwrap();
        assert_eq!(refs1.css_hrefs, refs2.css_hrefs);
        // reset.css should be first (encountered in a.html which sorts before b.html).
        assert_eq!(refs1.css_hrefs[0], "/css/reset.css");
    }
}
```

- [ ] **Step 5: Run the collect.rs tests**

Run: `cargo test --lib build::bundling::collect -- --nocapture`

Expected: All 12 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/build/bundling/ src/build/mod.rs
git commit -m "feat(bundling): add HTML scanning and reference collection

Implement collect_references to scan all HTML files in dist/ for
<link rel='stylesheet'> hrefs and <script src> srcs. Deduplicates
in first-encounter order. Skips external URLs, media attrs, defer,
async, type=module, and excluded patterns. Files sorted by path
for deterministic ordering."
```

---

## Task 3: Implement `bundling/js.rs` -- JS concatenation

**Depends on:** Task 2 (module structure must exist)

**Files:**
- Modify: `src/build/bundling/js.rs`

This task implements the simpler of the two merging operations: JS file concatenation with IIFE wrapping. Each file is wrapped in an IIFE to prevent variable scope pollution.

- [ ] **Step 1: Implement `merge_js_files` in `src/build/bundling/js.rs`**

```rust
//! JS concatenation with IIFE wrapping.
//!
//! Each file is wrapped in an Immediately Invoked Function Expression (IIFE)
//! to prevent variable scope pollution between files.

use std::path::Path;

use eyre::Result;

/// Read and concatenate multiple JS files into a single string.
///
/// Files are concatenated in order, each wrapped in an IIFE:
/// ```js
/// // Source: /js/utils.js
/// ;(function(){
/// /* original file content */
/// })();
/// ```
///
/// The leading semicolon is defensive -- it prevents issues if a previous
/// file's content ends unexpectedly. The IIFE prevents `var` and function
/// declarations from leaking into the global scope.
///
/// Missing files are skipped with a warning logged.
pub fn merge_js_files(
    srcs: &[String],
    dist_dir: &Path,
) -> Result<String> {
    let mut merged = String::new();
    let mut files_merged = 0u32;

    for src in srcs {
        let relative = src.trim_start_matches('/');
        let file_path = dist_dir.join(relative);

        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "JS bundling: file '{}' not found, skipping: {}",
                    file_path.display(), e
                );
                continue;
            }
        };

        if !merged.is_empty() {
            merged.push('\n');
        }

        // Source comment for traceability.
        merged.push_str("// Source: ");
        merged.push_str(src);
        merged.push('\n');

        // IIFE wrapping.
        merged.push_str(";(function(){\n");
        merged.push_str(&content);
        merged.push_str("\n})();\n");

        files_merged += 1;
    }

    if files_merged == 0 && !srcs.is_empty() {
        tracing::warn!("JS bundling: all {} JS file(s) failed to load", srcs.len());
    }

    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_merge_single_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/app.js", "var x = 1;");

        let result = merge_js_files(&["/js/app.js".to_string()], dist).unwrap();
        assert!(result.contains("// Source: /js/app.js"));
        assert!(result.contains(";(function(){"));
        assert!(result.contains("var x = 1;"));
        assert!(result.contains("})();"));
    }

    #[test]
    fn test_merge_multiple_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/utils.js", "function util() {}");
        write_file(dist, "js/app.js", "util();");

        let result = merge_js_files(
            &["/js/utils.js".to_string(), "/js/app.js".to_string()],
            dist,
        ).unwrap();

        // Both files should be wrapped in IIFEs.
        assert_eq!(result.matches(";(function(){").count(), 2);
        assert_eq!(result.matches("})();").count(), 2);
        assert!(result.contains("// Source: /js/utils.js"));
        assert!(result.contains("// Source: /js/app.js"));

        // utils.js should appear before app.js.
        let utils_pos = result.find("// Source: /js/utils.js").unwrap();
        let app_pos = result.find("// Source: /js/app.js").unwrap();
        assert!(utils_pos < app_pos);
    }

    #[test]
    fn test_merge_missing_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/exists.js", "var y = 2;");

        let result = merge_js_files(
            &["/js/missing.js".to_string(), "/js/exists.js".to_string()],
            dist,
        ).unwrap();

        // Missing file is skipped, existing file is included.
        assert!(!result.contains("missing.js"));
        assert!(result.contains("var y = 2;"));
    }

    #[test]
    fn test_iife_wrapping() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/test.js", "console.log('hello');");

        let result = merge_js_files(&["/js/test.js".to_string()], dist).unwrap();

        // Verify IIFE structure.
        assert!(result.contains(";(function(){\nconsole.log('hello');\n})();"));
    }
}
```

- [ ] **Step 2: Run the js.rs tests**

Run: `cargo test --lib build::bundling::js -- --nocapture`

Expected: All 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/build/bundling/js.rs
git commit -m "feat(bundling): implement JS concatenation with IIFE wrapping

Each JS file is wrapped in ;(function(){ ... })(); to prevent scope
pollution. Files concatenated in order with source comments for
traceability. Missing files skipped with warnings."
```

---

## Task 4: Implement `bundling/css.rs` -- CSS merging and tree-shaking

**Depends on:** Task 2 (module structure must exist)

**Files:**
- Modify: `src/build/bundling/css.rs`

This task implements CSS file merging with `@import` resolution and tree-shaking using selector matching against all rendered HTML DOMs. It reuses public primitives from `critical_css/extract.rs` (`strip_pseudo_for_matching`, `selector_matches`, `collect_global_deps`, `GlobalDependencies`).

- [ ] **Step 1: Implement `merge_css_files` in `src/build/bundling/css.rs`**

```rust
//! CSS merging and tree-shaking.
//!
//! Merges multiple CSS files into a single string, resolving @import chains.
//! Tree-shakes by matching selectors against all rendered HTML DOMs --
//! a selector is kept if it matches in ANY page.
//!
//! Reuses public primitives from `critical_css/extract.rs`:
//! - `strip_pseudo_for_matching` for pseudo-class/element stripping
//! - `selector_matches` for DOM matching via scraper
//! - `collect_global_deps` and `GlobalDependencies` for transitive
//!   dependency tracking (@font-face, @keyframes, custom properties)

use std::path::{Path, PathBuf};

use eyre::Result;
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;

use crate::build::critical_css::extract::{
    collect_global_deps, selector_matches, strip_pseudo_for_matching,
    GlobalDependencies,
};

/// Read and merge multiple CSS files into a single string.
///
/// Files are concatenated in the order given (first-encounter order from
/// HTML scanning). Each file's content is preceded by a source comment.
///
/// `@import` directives within each file are resolved by reusing the
/// `resolve_imports` logic from the critical_css module. Since bundling
/// runs before content hashing, no manifest resolution is needed.
///
/// Missing files are skipped with a warning logged.
pub fn merge_css_files(
    hrefs: &[String],
    dist_dir: &Path,
) -> Result<String> {
    let mut merged = String::new();
    let mut files_merged = 0u32;

    for href in hrefs {
        let relative = href.trim_start_matches('/');
        let file_path = dist_dir.join(relative);

        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "CSS bundling: file '{}' not found, skipping: {}",
                    file_path.display(), e
                );
                continue;
            }
        };

        // Resolve @import directives.
        let resolved = resolve_imports(&content, &file_path, dist_dir, 0);

        if !merged.is_empty() {
            merged.push('\n');
        }

        // Source comment for traceability (stripped by lightningcss minifier).
        merged.push_str("/* Source: ");
        merged.push_str(href);
        merged.push_str(" */\n");
        merged.push_str(&resolved);

        files_merged += 1;
    }

    if files_merged == 0 && !hrefs.is_empty() {
        tracing::warn!("CSS bundling: all {} CSS file(s) failed to load", hrefs.len());
    }

    Ok(merged)
}

/// Matches `@import` directives in CSS (both `url()` and string forms).
static IMPORT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"@import\s+(?:url\(\s*['"]?([^'")]+)['"]?\s*\)|['"]([^'"]+)['"]);?"#
    ).expect("import regex is valid")
});

/// Recursively resolve `@import` directives in CSS content.
///
/// This is equivalent to the `resolve_imports` in `critical_css/mod.rs`
/// but without manifest-based path resolution (bundling runs before
/// content hashing, so all files are at their original paths).
fn resolve_imports(
    css: &str,
    css_path: &Path,
    dist_dir: &Path,
    depth: usize,
) -> String {
    const MAX_IMPORT_DEPTH: usize = 10;

    if depth > MAX_IMPORT_DEPTH {
        tracing::warn!(
            "Circular or deeply nested @import detected in {}",
            css_path.display()
        );
        return css.to_string();
    }

    let css_dir = css_path.parent().unwrap_or(dist_dir);
    let mut result = css.to_string();

    let captures: Vec<_> = IMPORT_RE.captures_iter(css).collect();

    for cap in captures {
        let import_path_str = cap.get(1).or(cap.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");

        // Skip external imports.
        if import_path_str.starts_with("http://") || import_path_str.starts_with("https://") {
            continue;
        }

        if import_path_str.is_empty() {
            continue;
        }

        // Resolve relative to the importing file's directory.
        let import_path = if import_path_str.starts_with('/') {
            dist_dir.join(import_path_str.trim_start_matches('/'))
        } else {
            css_dir.join(import_path_str)
        };

        if import_path.exists() {
            match std::fs::read_to_string(&import_path) {
                Ok(imported_css) => {
                    let resolved = resolve_imports(
                        &imported_css, &import_path, dist_dir, depth + 1
                    );
                    if let Some(full_match) = cap.get(0) {
                        result = result.replace(full_match.as_str(), &resolved);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to read @import '{}': {e}",
                        import_path.display(),
                    );
                }
            }
        } else {
            tracing::warn!(
                "File not found for @import '{}' in {}",
                import_path_str, css_path.display()
            );
        }
    }

    result
}

/// Tree-shake CSS: remove selectors that do not match any element in any
/// rendered HTML page.
///
/// Steps:
/// 1. Parse every HTML file into a `scraper::Html` DOM.
/// 2. Parse the merged CSS with lightningcss (error recovery enabled).
/// 3. Walk all CSS rules. For each style rule, test its selectors against
///    every DOM. If a selector matches in at least one DOM, keep the rule.
/// 4. Include transitively referenced @font-face, @keyframes, and custom
///    property rules (same dependency tracking as critical_css/extract.rs).
/// 5. Serialize the surviving rules to a string.
///
/// Returns the tree-shaken CSS string.
pub fn tree_shake_css(
    merged_css: &str,
    html_files: &[PathBuf],
    minify: bool,
) -> Result<String> {
    // Step 1: Parse all HTML files into DOMs.
    let documents: Vec<scraper::Html> = html_files
        .iter()
        .filter_map(|path| {
            match std::fs::read_to_string(path) {
                Ok(html) => Some(scraper::Html::parse_document(&html)),
                Err(e) => {
                    tracing::warn!(
                        "Tree-shaking: failed to read HTML '{}': {}",
                        path.display(), e
                    );
                    None
                }
            }
        })
        .collect();

    if documents.is_empty() {
        return Ok(merged_css.to_string());
    }

    // Step 2: Parse CSS.
    let options = ParserOptions {
        error_recovery: true,
        ..ParserOptions::default()
    };
    let stylesheet = StyleSheet::parse(merged_css, options)
        .map_err(|e| eyre::eyre!("CSS parse error during tree-shaking: {e}"))?;

    // Step 3: Walk rules, matching against all DOMs.
    let mut matched_rules: Vec<String> = Vec::new();
    let mut global_deps = GlobalDependencies::default();

    let mut font_face_rules: Vec<String> = Vec::new();
    let mut keyframe_rules: Vec<String> = Vec::new();
    let mut custom_prop_rules: Vec<(String, String)> = Vec::new();

    walk_rules(
        &stylesheet.rules.0,
        &documents,
        &mut matched_rules,
        &mut global_deps,
        &mut font_face_rules,
        &mut keyframe_rules,
        &mut custom_prop_rules,
        minify,
    );

    if matched_rules.is_empty() {
        tracing::info!("Tree-shaking: no CSS selectors matched any page");
        return Ok(String::new());
    }

    // Step 4: Include transitively referenced global rules.
    let mut critical_parts: Vec<String> = Vec::new();

    // Include referenced @font-face rules.
    for rule_css in &font_face_rules {
        for family in &global_deps.font_families {
            if rule_css.contains(family.as_str()) {
                critical_parts.push(rule_css.clone());
                break;
            }
        }
    }

    // Include referenced @keyframes rules.
    for rule_css in &keyframe_rules {
        for anim_name in &global_deps.animation_names {
            if rule_css.contains(anim_name.as_str()) {
                critical_parts.push(rule_css.clone());
                break;
            }
        }
    }

    // Include referenced custom property definitions.
    for (prop_name, rule_css) in &custom_prop_rules {
        if global_deps.custom_properties.contains(prop_name) {
            critical_parts.push(rule_css.clone());
        }
    }

    // Add matched style rules.
    critical_parts.extend(matched_rules);

    Ok(critical_parts.join("\n"))
}

/// Create printer options with optional minification.
fn printer(minify: bool) -> PrinterOptions<'static> {
    PrinterOptions {
        minify,
        ..PrinterOptions::default()
    }
}

/// Recursively walk CSS rules, matching style rules against ALL DOMs.
///
/// This is the bundling variant of the walk_rules in critical_css/extract.rs.
/// The key difference: selectors are tested against multiple documents (the
/// union of all pages), not a single document.
fn walk_rules(
    rules: &[CssRule],
    documents: &[scraper::Html],
    matched_rules: &mut Vec<String>,
    global_deps: &mut GlobalDependencies,
    font_face_rules: &mut Vec<String>,
    keyframe_rules: &mut Vec<String>,
    custom_prop_rules: &mut Vec<(String, String)>,
    minify: bool,
) {
    let opts = printer(minify);

    for rule in rules {
        match rule {
            CssRule::Style(style_rule) => {
                handle_style_rule(
                    style_rule, documents, matched_rules,
                    global_deps, custom_prop_rules, minify,
                );
            }
            CssRule::Media(media_rule) => {
                let mut media_matched: Vec<String> = Vec::new();
                walk_rules(
                    &media_rule.rules.0, documents, &mut media_matched,
                    global_deps, font_face_rules, keyframe_rules,
                    custom_prop_rules, minify,
                );
                if !media_matched.is_empty() {
                    let query = media_rule.query
                        .to_css_string(opts)
                        .unwrap_or_default();
                    let inner = media_matched.join("\n");
                    matched_rules.push(format!("@media {query} {{\n{inner}\n}}"));
                }
            }
            CssRule::Supports(supports_rule) => {
                let mut supports_matched: Vec<String> = Vec::new();
                walk_rules(
                    &supports_rule.rules.0, documents, &mut supports_matched,
                    global_deps, font_face_rules, keyframe_rules,
                    custom_prop_rules, minify,
                );
                if !supports_matched.is_empty() {
                    let condition = supports_rule.condition
                        .to_css_string(opts)
                        .unwrap_or_default();
                    let inner = supports_matched.join("\n");
                    matched_rules.push(format!("@supports {condition} {{\n{inner}\n}}"));
                }
            }
            CssRule::LayerBlock(layer_rule) => {
                let mut layer_matched: Vec<String> = Vec::new();
                walk_rules(
                    &layer_rule.rules.0, documents, &mut layer_matched,
                    global_deps, font_face_rules, keyframe_rules,
                    custom_prop_rules, minify,
                );
                if !layer_matched.is_empty() {
                    let name = layer_rule.name.as_ref()
                        .and_then(|n| n.to_css_string(opts).ok())
                        .unwrap_or_default();
                    let inner = layer_matched.join("\n");
                    if name.is_empty() {
                        matched_rules.push(format!("@layer {{\n{inner}\n}}"));
                    } else {
                        matched_rules.push(format!("@layer {name} {{\n{inner}\n}}"));
                    }
                }
            }
            CssRule::LayerStatement(_) => {
                if let Ok(css) = rule.to_css_string(opts) {
                    matched_rules.push(css);
                }
            }
            CssRule::FontFace(_) => {
                if let Ok(css) = rule.to_css_string(opts) {
                    font_face_rules.push(css);
                }
            }
            CssRule::Keyframes(_) => {
                if let Ok(css) = rule.to_css_string(opts) {
                    keyframe_rules.push(css);
                }
            }
            CssRule::Import(_) | CssRule::Namespace(_) => {
                if let Ok(css) = rule.to_css_string(opts) {
                    matched_rules.push(css);
                }
            }
            _ => {
                // Unknown at-rules: include by default.
                if let Ok(css) = rule.to_css_string(opts) {
                    matched_rules.push(css);
                }
            }
        }
    }
}

/// Handle a single style rule: check if its selectors match ANY DOM,
/// and if so, serialize it and collect its global dependencies.
fn handle_style_rule(
    style_rule: &lightningcss::rules::style::StyleRule,
    documents: &[scraper::Html],
    matched_rules: &mut Vec<String>,
    global_deps: &mut GlobalDependencies,
    custom_prop_rules: &mut Vec<(String, String)>,
    minify: bool,
) {
    let opts = printer(minify);

    let selector_list = style_rule.selectors
        .to_css_string(opts)
        .unwrap_or_default();

    let css_text = match style_rule.to_css_string(opts) {
        Ok(css) => css,
        Err(_) => return,
    };

    // Collect custom property definitions from this rule regardless of
    // whether it matches (needed for var() dependency resolution).
    static CUSTOM_PROP_DEF_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(--[a-zA-Z0-9_-]+)\s*:").expect("custom prop regex is valid")
    });

    for cap in CUSTOM_PROP_DEF_RE.captures_iter(&css_text) {
        custom_prop_rules.push((cap[1].to_string(), css_text.clone()));
    }

    // Check each selector in the comma-separated list against ALL documents.
    let mut any_match = false;
    for selector in selector_list.split(',') {
        let selector = selector.trim();
        if selector.is_empty() {
            continue;
        }

        match strip_pseudo_for_matching(selector) {
            None => {
                // Pseudo-only selector (e.g. ::selection) -> include unconditionally.
                any_match = true;
                break;
            }
            Some(stripped) => {
                // Match against ANY document.
                for doc in documents {
                    if selector_matches(&stripped, doc) {
                        any_match = true;
                        break;
                    }
                }
                if any_match {
                    break;
                }
            }
        }
    }

    if any_match {
        let deps = collect_global_deps(&css_text);
        global_deps.font_families.extend(deps.font_families);
        global_deps.animation_names.extend(deps.animation_names);
        global_deps.custom_properties.extend(deps.custom_properties);

        matched_rules.push(css_text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // --- merge_css_files tests ---

    #[test]
    fn test_merge_single_file() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/style.css", "body { color: red; }");

        let result = merge_css_files(&["/css/style.css".to_string()], dist).unwrap();
        assert!(result.contains("/* Source: /css/style.css */"));
        assert!(result.contains("body { color: red; }"));
    }

    #[test]
    fn test_merge_multiple_files() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/reset.css", "* { margin: 0; }");
        write_file(dist, "css/style.css", "body { color: red; }");

        let result = merge_css_files(
            &["/css/reset.css".to_string(), "/css/style.css".to_string()],
            dist,
        ).unwrap();

        assert!(result.contains("/* Source: /css/reset.css */"));
        assert!(result.contains("/* Source: /css/style.css */"));

        // reset.css should appear before style.css.
        let reset_pos = result.find("* { margin: 0; }").unwrap();
        let style_pos = result.find("body { color: red; }").unwrap();
        assert!(reset_pos < style_pos);
    }

    #[test]
    fn test_merge_resolves_import() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/base.css", ".base { margin: 0; }");
        write_file(dist, "css/style.css", r#"@import "base.css";
.main { color: red; }"#);

        let result = merge_css_files(&["/css/style.css".to_string()], dist).unwrap();

        // @import should be resolved (base.css content inlined).
        assert!(result.contains(".base { margin: 0; }"));
        assert!(result.contains(".main { color: red; }"));
        assert!(!result.contains("@import"));
    }

    #[test]
    fn test_merge_missing_file() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/exists.css", ".ok { color: green; }");

        let result = merge_css_files(
            &["/css/missing.css".to_string(), "/css/exists.css".to_string()],
            dist,
        ).unwrap();

        // Missing file skipped, existing file included.
        assert!(!result.contains("missing"));
        assert!(result.contains(".ok { color: green; }"));
    }

    // --- tree_shake_css tests ---

    #[test]
    fn test_tree_shake_removes_unused() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><div class="used">Hello</div></body></html>"#);

        let css = ".used { color: red; } .unused { color: blue; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains(".used"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_tree_shake_keeps_used() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><p class="hero">Hi</p></body></html>"#);

        let css = ".hero { color: red; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains(".hero"));
        assert!(result.contains("color"));
    }

    #[test]
    fn test_tree_shake_keeps_pseudo() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><a class="link">Click</a></body></html>"#);

        let css = ".link { color: blue; } .link:hover { color: red; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        // Both the base rule and :hover rule should be kept.
        assert!(result.contains(".link"));
        assert!(result.contains("blue") || result.contains("red"));
    }

    #[test]
    fn test_tree_shake_media_query() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><div class="card">Card</div></body></html>"#);

        let css = r#"
            @media (max-width: 768px) {
                .card { width: 100%; }
                .sidebar { display: none; }
            }
        "#;
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains("@media"));
        assert!(result.contains(".card"));
        assert!(!result.contains(".sidebar"));
    }

    #[test]
    fn test_tree_shake_font_face() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><h1 class="heading">Title</h1></body></html>"#);

        let css = r#"
            @font-face {
                font-family: "Inter";
                src: url("/fonts/inter.woff2") format("woff2");
            }
            .heading { font-family: "Inter", sans-serif; }
            .unused { color: green; }
        "#;
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains("@font-face"));
        assert!(result.contains("Inter"));
        assert!(result.contains(".heading"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_tree_shake_keyframes() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><div class="spinner">Loading</div></body></html>"#);

        let css = r#"
            @keyframes spin {
                from { transform: rotate(0deg); }
                to { transform: rotate(360deg); }
            }
            .spinner { animation-name: spin; }
            .unused { color: green; }
        "#;
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains("@keyframes"));
        assert!(result.contains("spin"));
        assert!(result.contains(".spinner"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_tree_shake_empty_result() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", "<html><body><div>No matching classes</div></body></html>");

        let css = ".missing-a { color: red; } .missing-b { color: blue; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn test_tree_shake_multiple_docs() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "a.html", r#"<html><body><div class="a-only">A</div></body></html>"#);
        write_file(dist, "b.html", r#"<html><body><div class="b-only">B</div></body></html>"#);

        let css = ".a-only { color: red; } .b-only { color: blue; } .neither { color: green; }";
        let html_files = vec![dist.join("a.html"), dist.join("b.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        // Selectors used in either doc should be kept.
        assert!(result.contains(".a-only"));
        assert!(result.contains(".b-only"));
        // Selector used in neither should be removed.
        assert!(!result.contains(".neither"));
    }
}
```

- [ ] **Step 2: Run the css.rs tests**

Run: `cargo test --lib build::bundling::css -- --nocapture`

Expected: All 12 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/build/bundling/css.rs
git commit -m "feat(bundling): implement CSS merging and tree-shaking

CSS merging: concatenate files with source comments, resolve @import
chains. Tree-shaking: parse all HTML DOMs, walk CSS rules recursively,
keep selectors matching any page. Reuses strip_pseudo_for_matching,
selector_matches, and collect_global_deps from critical_css/extract.rs
for consistent selector handling."
```

---

## Task 5: Implement `bundling/rewrite.rs` -- HTML rewriting

**Depends on:** Task 2 (module structure), conceptually after Tasks 3-4 (but no code dependency)

**Files:**
- Modify: `src/build/bundling/rewrite.rs`

This task implements `lol_html`-based HTML rewriting: replace the first `<link>`/`<script>` referencing a bundled file with the bundle path, remove subsequent ones, and handle the critical CSS preload pattern (both `<link rel="preload">` and `<noscript>` text replacement).

- [ ] **Step 1: Implement `rewrite_html_for_bundles` in `src/build/bundling/rewrite.rs`**

```rust
//! HTML rewriting: replace <link> and <script> tags with bundle references.
//!
//! Uses `lol_html` for streaming HTML rewriting. Handles:
//! - Replacing first bundled `<link>`/`<script>` with bundle path
//! - Removing subsequent bundled `<link>`/`<script>` tags
//! - Rewriting critical CSS preload pattern (`<link rel="preload" as="style">`)
//! - Rewriting `<noscript>` fallback links (via text replacement, since
//!   lol_html treats noscript content as raw text)

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eyre::{Result, WrapErr};

/// Rewrite all HTML files in dist/ to reference bundled CSS/JS files.
///
/// For each HTML file:
/// 1. Replace the FIRST `<link>` / `<script>` that references a bundled
///    file with a reference to the bundle.
/// 2. Remove subsequent `<link>` / `<script>` tags that reference other
///    bundled files (they are now in the bundle).
/// 3. Handle the critical CSS preload pattern: rewrite both the preload
///    `<link>` and the `<noscript>` fallback `<link>`.
pub fn rewrite_html_for_bundles(
    html_files: &[PathBuf],
    dist_dir: &Path,
    css_bundle_href: Option<&str>,
    js_bundle_src: Option<&str>,
    css_hrefs: &[String],
    js_srcs: &[String],
) -> Result<()> {
    let css_set: HashSet<&str> = css_hrefs.iter().map(|s| s.as_str()).collect();
    let js_set: HashSet<&str> = js_srcs.iter().map(|s| s.as_str()).collect();

    for html_path in html_files {
        let html_content = std::fs::read_to_string(html_path)
            .wrap_err_with(|| format!(
                "Bundling rewrite: failed to read '{}'", html_path.display()
            ))?;

        let rewritten = rewrite_single_html(
            &html_content,
            css_bundle_href,
            js_bundle_src,
            &css_set,
            &js_set,
        ).wrap_err_with(|| format!(
            "Bundling rewrite: failed to rewrite '{}'", html_path.display()
        ))?;

        std::fs::write(html_path, rewritten)
            .wrap_err_with(|| format!(
                "Bundling rewrite: failed to write '{}'", html_path.display()
            ))?;
    }

    Ok(())
}

/// Rewrite a single HTML string to reference bundled CSS/JS files.
///
/// All four handlers (stylesheet, preload, noscript, script) are always
/// registered. Handlers for disabled bundle types (css_bundle_href=None
/// or js_bundle_src=None) are no-ops because the bundled set will be
/// empty and no elements will match.
fn rewrite_single_html(
    html: &str,
    css_bundle_href: Option<&str>,
    js_bundle_src: Option<&str>,
    css_set: &HashSet<&str>,
    js_set: &HashSet<&str>,
) -> Result<String> {
    let css_injected = Rc::new(RefCell::new(false));
    let js_injected = Rc::new(RefCell::new(false));

    let css_injected_link = css_injected.clone();
    let css_injected_preload = css_injected.clone();
    let js_injected_clone = js_injected.clone();

    let css_set_link: Rc<HashSet<String>> = Rc::new(css_set.iter().map(|s| s.to_string()).collect());
    let css_set_preload = css_set_link.clone();
    let js_set_rc: Rc<HashSet<String>> = Rc::new(js_set.iter().map(|s| s.to_string()).collect());

    let css_bundle = css_bundle_href.map(|s| s.to_string());
    let css_bundle_link = css_bundle.clone();
    let css_bundle_preload = css_bundle.clone();
    let js_bundle = js_bundle_src.map(|s| s.to_string());

    let noscript_css_bundle = css_bundle.clone();
    let noscript_css_set: Rc<HashSet<String>> = css_set_link.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                // CSS <link rel="stylesheet"> handler.
                lol_html::element!("link[rel='stylesheet']", move |el| {
                    let href = match el.get_attribute("href") {
                        Some(h) => h,
                        None => return Ok(()),
                    };

                    if !css_set_link.contains(&href) {
                        return Ok(());
                    }

                    if !*css_injected_link.borrow() {
                        if let Some(ref bundle) = css_bundle_link {
                            el.set_attribute("href", bundle)?;
                        }
                        *css_injected_link.borrow_mut() = true;
                    } else {
                        el.remove();
                    }

                    Ok(())
                }),
                // CSS <link rel="preload" as="style"> handler (critical CSS pattern).
                lol_html::element!("link[rel='preload'][as='style']", move |el| {
                    let href = match el.get_attribute("href") {
                        Some(h) => h,
                        None => return Ok(()),
                    };

                    if !css_set_preload.contains(&href) {
                        return Ok(());
                    }

                    if !*css_injected_preload.borrow() {
                        if let Some(ref bundle) = css_bundle_preload {
                            el.set_attribute("href", bundle)?;
                        }
                        *css_injected_preload.borrow_mut() = true;
                    } else {
                        el.remove();
                    }

                    Ok(())
                }),
                // Noscript text handler: rewrite <link> hrefs inside <noscript>.
                // lol_html treats noscript content as raw text, so we use string
                // replacement rather than element matching.
                lol_html::text!("noscript", move |text| {
                    let content = text.as_str().to_string();

                    if let Some(ref bundle) = noscript_css_bundle {
                        let mut modified = content;
                        let mut first = true;

                        for href in noscript_css_set.iter() {
                            if modified.contains(href.as_str()) {
                                if first {
                                    modified = modified.replace(href.as_str(), bundle);
                                    first = false;
                                } else {
                                    let pattern = format!(
                                        r#"<link rel="stylesheet" href="{}">"#,
                                        href
                                    );
                                    modified = modified.replace(&pattern, "");
                                }
                            }
                        }

                        if modified != content {
                            text.replace(&modified, lol_html::html_content::ContentType::Html);
                        }
                    }

                    Ok(())
                }),
                // JS <script> handler.
                lol_html::element!("script", move |el| {
                    let src = match el.get_attribute("src") {
                        Some(s) if !s.is_empty() => s,
                        _ => return Ok(()),
                    };

                    // Skip defer, async, type="module".
                    if el.get_attribute("defer").is_some()
                        || el.get_attribute("async").is_some()
                    {
                        return Ok(());
                    }
                    if let Some(type_attr) = el.get_attribute("type") {
                        if type_attr == "module" {
                            return Ok(());
                        }
                    }

                    if !js_set_rc.contains(&src) {
                        return Ok(());
                    }

                    if !*js_injected_clone.borrow() {
                        if let Some(ref bundle) = js_bundle {
                            el.set_attribute("src", bundle)?;
                        }
                        *js_injected_clone.borrow_mut() = true;
                    } else {
                        el.remove();
                    }

                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| eyre::eyre!("lol_html rewrite error: {e}"))?;

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn css_set(hrefs: &[&str]) -> HashSet<&str> {
        hrefs.iter().copied().collect()
    }

    fn js_set(srcs: &[&str]) -> HashSet<&str> {
        srcs.iter().copied().collect()
    }

    #[test]
    fn test_rewrite_css_single_link() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body></body></html>"#;
        let css = css_set(&["/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        assert!(result.contains(r#"href="/css/bundle.css""#));
        assert!(!result.contains(r#"href="/css/style.css""#));
    }

    #[test]
    fn test_rewrite_css_multiple_links() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body></body></html>"#;
        let css = css_set(&["/css/reset.css", "/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        // First link rewritten to bundle, second removed.
        assert!(result.contains(r#"href="/css/bundle.css""#));
        assert!(!result.contains(r#"href="/css/reset.css""#));
        assert!(!result.contains(r#"href="/css/style.css""#));
        // Only one link tag should remain.
        assert_eq!(result.matches(r#"rel="stylesheet""#).count(), 1);
    }

    #[test]
    fn test_rewrite_css_preload_pattern() {
        let html = r#"<html><head>
            <style>.hero { color: red; }</style>
            <link rel="preload" href="/css/style.css" as="style" onload="this.onload=null;this.rel='stylesheet'">
            <noscript><link rel="stylesheet" href="/css/style.css"></noscript>
        </head><body></body></html>"#;
        let css = css_set(&["/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        // Preload link should be rewritten.
        assert!(result.contains(r#"href="/css/bundle.css""#));
        assert!(!result.contains(r#"href="/css/style.css""#));
        // Inline style should be untouched.
        assert!(result.contains("<style>.hero { color: red; }</style>"));
    }

    #[test]
    fn test_rewrite_css_noscript_pattern() {
        let html = r#"<html><head>
            <noscript><link rel="stylesheet" href="/css/style.css"></noscript>
        </head><body></body></html>"#;
        let css = css_set(&["/css/style.css"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), None, &css, &HashSet::new()).unwrap();

        // Noscript link should be rewritten via text replacement.
        assert!(result.contains("/css/bundle.css"));
        assert!(!result.contains("/css/style.css"));
    }

    #[test]
    fn test_rewrite_js_single_script() {
        let html = r#"<html><body><script src="/js/app.js"></script></body></html>"#;
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        assert!(result.contains(r#"src="/js/bundle.js""#));
        assert!(!result.contains(r#"src="/js/app.js""#));
    }

    #[test]
    fn test_rewrite_js_multiple_scripts() {
        let html = r#"<html><body>
            <script src="/js/utils.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let js = js_set(&["/js/utils.js", "/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        // First script rewritten, second removed.
        assert!(result.contains(r#"src="/js/bundle.js""#));
        assert!(!result.contains(r#"src="/js/utils.js""#));
        assert!(!result.contains(r#"src="/js/app.js""#));
    }

    #[test]
    fn test_rewrite_preserves_external() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="https://cdn.example.com/lib.css">
            <link rel="stylesheet" href="/css/local.css">
        </head><body>
            <script src="https://cdn.example.com/lib.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let css = css_set(&["/css/local.css"]);
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, Some("/css/bundle.css"), Some("/js/bundle.js"), &css, &js).unwrap();

        // External links should be untouched.
        assert!(result.contains("https://cdn.example.com/lib.css"));
        assert!(result.contains("https://cdn.example.com/lib.js"));
    }

    #[test]
    fn test_rewrite_preserves_module() {
        let html = r#"<html><body>
            <script type="module" src="/js/mod.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        // Module script untouched.
        assert!(result.contains(r#"type="module" src="/js/mod.js""#));
        // Regular script rewritten.
        assert!(result.contains(r#"src="/js/bundle.js""#));
    }

    #[test]
    fn test_rewrite_preserves_async_defer() {
        let html = r#"<html><body>
            <script defer src="/js/deferred.js"></script>
            <script async src="/js/async.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#;
        let js = js_set(&["/js/app.js"]);
        let result = rewrite_single_html(html, None, Some("/js/bundle.js"), &HashSet::new(), &js).unwrap();

        // Defer and async scripts untouched.
        assert!(result.contains(r#"src="/js/deferred.js""#));
        assert!(result.contains(r#"src="/js/async.js""#));
        // Regular script rewritten.
        assert!(result.contains(r#"src="/js/bundle.js""#));
    }

    #[test]
    fn test_rewrite_no_bundled_refs() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/other.css"></head><body><script src="/js/other.js"></script></body></html>"#;
        let result = rewrite_single_html(html, Some("/css/bundle.css"), Some("/js/bundle.js"), &HashSet::new(), &HashSet::new()).unwrap();

        // No bundled refs -> no changes.
        assert!(result.contains(r#"href="/css/other.css""#));
        assert!(result.contains(r#"src="/js/other.js""#));
    }
}
```

- [ ] **Step 2: Run the rewrite.rs tests**

Run: `cargo test --lib build::bundling::rewrite -- --nocapture`

Expected: All 10 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/build/bundling/rewrite.rs
git commit -m "feat(bundling): implement HTML rewriting for bundle references

Use lol_html to replace first bundled <link>/<script> with bundle
path and remove subsequent ones. Handles critical CSS preload pattern
(link rel=preload as=style) and noscript fallback text replacement.
Preserves external, module, async, and defer scripts."
```

---

## Task 6: Implement `bundling/mod.rs` -- orchestration and public API

**Depends on:** Tasks 2-5 (all submodules must be implemented)

**Files:**
- Modify: `src/build/bundling/mod.rs`

This task implements the `bundle_assets` public function that orchestrates the full bundling pipeline: collect references, merge CSS/JS, tree-shake, write output files, and rewrite HTML.

- [ ] **Step 1: Implement `bundle_assets` in `src/build/bundling/mod.rs`**

Replace the placeholder content with:

```rust
//! CSS/JS bundling and tree-shaking.
//!
//! Merges multiple CSS/JS files into single bundles, tree-shakes unused
//! CSS selectors, and rewrites HTML references. Runs as Phase 2.5 in the
//! build pipeline (after rendering, before content hashing).

pub mod collect;
pub mod css;
pub mod js;
pub mod rewrite;

use std::path::Path;

use eyre::{Result, WrapErr};

use crate::config::BundlingConfig;

/// Bundle CSS and JS files in dist/ and rewrite HTML references.
///
/// This is the main entry point called from the build pipeline after all
/// pages have been rendered and written to disk.
///
/// Steps:
/// 1. Scan all HTML files in dist/ for <link> and <script> tags.
/// 2. Collect and deduplicate referenced local CSS/JS files.
/// 3. For CSS: merge, tree-shake, write bundled file.
/// 4. For JS: concatenate with IIFE wrapping, write bundled file.
/// 5. Rewrite all HTML files to reference bundled files.
///
/// `minify_css` controls whether the bundled CSS output is minified
/// via lightningcss's printer. Typically set to `config.build.minify`.
///
/// Returns the list of generated file paths (relative to dist/)
/// for content hashing integration.
pub fn bundle_assets(
    dist_dir: &Path,
    config: &BundlingConfig,
    minify_css: bool,
) -> Result<Vec<String>> {
    if !config.enabled {
        return Ok(Vec::new());
    }

    // Step 1-2: Collect references from all HTML files.
    let refs = collect::collect_references(dist_dir, &config.exclude)
        .wrap_err("Failed to collect CSS/JS references from HTML files")?;

    if refs.css_hrefs.is_empty() && refs.js_srcs.is_empty() {
        tracing::debug!("Bundling: no local CSS/JS references found in HTML files");
        return Ok(Vec::new());
    }

    tracing::info!(
        "Bundling: found {} CSS file(s), {} JS file(s) across {} HTML page(s)",
        refs.css_hrefs.len(),
        refs.js_srcs.len(),
        refs.html_files.len(),
    );

    let mut generated_files: Vec<String> = Vec::new();
    let mut css_bundle_href: Option<String> = None;
    let mut js_bundle_src: Option<String> = None;

    // Step 3: CSS bundling.
    if config.css && !refs.css_hrefs.is_empty() {
        let merged = css::merge_css_files(&refs.css_hrefs, dist_dir)
            .wrap_err("Failed to merge CSS files")?;

        if merged.is_empty() {
            tracing::warn!("Bundling: merged CSS is empty (all files failed to load)");
        } else {
            let output_css = if config.tree_shake_css {
                css::tree_shake_css(&merged, &refs.html_files, minify_css)
                    .wrap_err("CSS tree-shaking failed")?
            } else if minify_css {
                // Minify without tree-shaking: parse and re-serialize.
                minify_css_string(&merged)?
            } else {
                merged
            };

            // Write bundled CSS file.
            let output_path = dist_dir.join(&config.css_output);

            // Warn if output path already exists.
            if output_path.exists() {
                tracing::warn!(
                    "Bundling: overwriting existing file at '{}'",
                    config.css_output,
                );
            }

            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!(
                        "Failed to create directory for CSS bundle: {}",
                        parent.display()
                    ))?;
            }

            std::fs::write(&output_path, &output_css)
                .wrap_err_with(|| format!(
                    "Failed to write CSS bundle to '{}'",
                    output_path.display()
                ))?;

            let href = format!("/{}", config.css_output);
            css_bundle_href = Some(href);
            generated_files.push(config.css_output.clone());

            tracing::info!(
                "Bundling: wrote CSS bundle ({} bytes) to {}",
                output_css.len(),
                config.css_output,
            );
        }
    }

    // Step 4: JS bundling.
    if config.js && !refs.js_srcs.is_empty() {
        let merged = js::merge_js_files(&refs.js_srcs, dist_dir)
            .wrap_err("Failed to merge JS files")?;

        if merged.is_empty() {
            tracing::warn!("Bundling: merged JS is empty (all files failed to load)");
        } else {
            // Write bundled JS file.
            let output_path = dist_dir.join(&config.js_output);

            if output_path.exists() {
                tracing::warn!(
                    "Bundling: overwriting existing file at '{}'",
                    config.js_output,
                );
            }

            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!(
                        "Failed to create directory for JS bundle: {}",
                        parent.display()
                    ))?;
            }

            std::fs::write(&output_path, &merged)
                .wrap_err_with(|| format!(
                    "Failed to write JS bundle to '{}'",
                    output_path.display()
                ))?;

            let src = format!("/{}", config.js_output);
            js_bundle_src = Some(src);
            generated_files.push(config.js_output.clone());

            tracing::info!(
                "Bundling: wrote JS bundle ({} bytes) to {}",
                merged.len(),
                config.js_output,
            );
        }
    }

    // Step 5: Rewrite all HTML files.
    if css_bundle_href.is_some() || js_bundle_src.is_some() {
        rewrite::rewrite_html_for_bundles(
            &refs.html_files,
            dist_dir,
            css_bundle_href.as_deref(),
            js_bundle_src.as_deref(),
            &refs.css_hrefs,
            &refs.js_srcs,
        ).wrap_err("Failed to rewrite HTML files for bundles")?;

        tracing::info!(
            "Bundling: rewrote {} HTML file(s)",
            refs.html_files.len(),
        );
    }

    Ok(generated_files)
}

/// Minify a CSS string using lightningcss without tree-shaking.
fn minify_css_string(css: &str) -> Result<String> {
    use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};

    let options = ParserOptions {
        error_recovery: true,
        ..ParserOptions::default()
    };
    let stylesheet = StyleSheet::parse(css, options)
        .map_err(|e| eyre::eyre!("CSS parse error during minification: {e}"))?;

    let result = stylesheet.to_css(PrinterOptions {
        minify: true,
        ..PrinterOptions::default()
    }).map_err(|e| eyre::eyre!("CSS serialization error: {e}"))?;

    Ok(result.code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_bundle_assets_disabled() {
        let tmp = TempDir::new().unwrap();
        let config = BundlingConfig::default(); // enabled: false
        let result = bundle_assets(tmp.path(), &config, false).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_bundle_assets_no_refs() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", "<html><body>No CSS or JS</body></html>");

        let config = BundlingConfig { enabled: true, ..Default::default() };
        let result = bundle_assets(dist, &config, false).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_bundle_assets_css_only() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", ".hero { color: red; }");
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hi</div></body></html>"#);

        let config = BundlingConfig {
            enabled: true,
            js: false,
            ..Default::default()
        };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result, vec!["css/bundle.css"]);
        assert!(dist.join("css/bundle.css").exists());

        // HTML should reference the bundle.
        let html = fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(html.contains("/css/bundle.css"));
    }

    #[test]
    fn test_bundle_assets_js_only() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "js/app.js", "var x = 1;");
        write_file(dist, "index.html", r#"<html><body><script src="/js/app.js"></script></body></html>"#);

        let config = BundlingConfig {
            enabled: true,
            css: false,
            ..Default::default()
        };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result, vec!["js/bundle.js"]);
        assert!(dist.join("js/bundle.js").exists());

        // HTML should reference the bundle.
        let html = fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(html.contains("/js/bundle.js"));
    }

    #[test]
    fn test_bundle_assets_end_to_end() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/reset.css", "* { margin: 0; }");
        write_file(dist, "css/style.css", ".hero { color: red; } .unused { display: none; }");
        write_file(dist, "js/utils.js", "function util() {}");
        write_file(dist, "js/app.js", "util();");
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body>
            <div class="hero">Hello</div>
            <script src="/js/utils.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let config = BundlingConfig { enabled: true, ..Default::default() };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains(&"css/bundle.css".to_string()));
        assert!(result.contains(&"js/bundle.js".to_string()));

        // Verify CSS bundle: tree-shaking should keep .hero and * but not .unused.
        let css_bundle = fs::read_to_string(dist.join("css/bundle.css")).unwrap();
        assert!(css_bundle.contains(".hero"));
        assert!(!css_bundle.contains(".unused"));

        // Verify JS bundle: both files wrapped in IIFEs.
        let js_bundle = fs::read_to_string(dist.join("js/bundle.js")).unwrap();
        assert!(js_bundle.contains("function util()"));
        assert!(js_bundle.contains("util();"));
        assert_eq!(js_bundle.matches(";(function(){").count(), 2);

        // Verify HTML rewriting.
        let html = fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(html.contains("/css/bundle.css"));
        assert!(html.contains("/js/bundle.js"));
        assert!(!html.contains("/css/reset.css"));
        assert!(!html.contains("/css/style.css"));
        assert!(!html.contains("/js/utils.js"));
        assert!(!html.contains("/js/app.js"));
    }

    #[test]
    fn test_bundle_assets_output_path_conflict() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        // Pre-existing file at bundle output path.
        write_file(dist, "css/bundle.css", "/* old content */");
        write_file(dist, "css/style.css", ".hero { color: red; }");
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hi</div></body></html>"#);

        let config = BundlingConfig {
            enabled: true,
            js: false,
            ..Default::default()
        };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result, vec!["css/bundle.css"]);

        // Old content should be overwritten.
        let bundle = fs::read_to_string(dist.join("css/bundle.css")).unwrap();
        assert!(!bundle.contains("old content"));
        assert!(bundle.contains(".hero"));
    }
}
```

- [ ] **Step 2: Run the mod.rs tests**

Run: `cargo test --lib build::bundling -- --nocapture`

Expected: All tests across all bundling submodules pass (collect: 12, js: 4, css: 12, rewrite: 10, mod: 6 = 44 total).

- [ ] **Step 3: Commit**

```bash
git add src/build/bundling/mod.rs
git commit -m "feat(bundling): implement bundle_assets orchestration

Public API: bundle_assets(dist_dir, config, minify_css) -> Vec<String>.
Orchestrates collect -> merge -> tree-shake -> write -> rewrite pipeline.
Returns list of generated files for content hashing integration.
Handles disabled state, CSS-only, JS-only, and full bundling modes."
```

---

## Task 7: Integrate bundling into the build pipeline

**Depends on:** Tasks 1 and 6 (config and bundling module complete)

**Files:**
- Modify: `src/build/render.rs`

This task wires `bundle_assets` into the `build()` function as Phase 2.5, between post-build plugin hooks and content hashing.

- [ ] **Step 1: Add the `bundling` import to `render.rs`**

In `src/build/render.rs`, add the import after `use super::content_hash;` (around line 20):

```rust
use super::bundling;
use super::content_hash;
```

- [ ] **Step 2: Add bundling log line in the build setup section**

In `src/build/render.rs`, after the content hash enabled log (around line 131, after the hints enabled log):

```rust
    if config.build.bundling.enabled {
        tracing::info!("CSS/JS bundling enabled.");
    }
```

- [ ] **Step 3: Insert Phase 2.5 bundling call**

In `src/build/render.rs`, after `plugin_registry.post_build(&dist_dir, project_root)?;` and before the Phase 3 content hashing block (around line 195-198), insert the Phase 2.5 bundling call:

Find the existing code:

```rust
    // Run post-build hooks
    plugin_registry.post_build(&dist_dir, project_root)?;

    // Phase 3: Rewrite remaining asset references in HTML/CSS/JS.
    if config.build.content_hash.enabled && !manifest.is_empty() {
        content_hash::rewrite_references(&dist_dir, &manifest)?;
        tracing::info!("Asset references rewritten.");
    }
```

Replace with:

```rust
    // Run post-build hooks
    plugin_registry.post_build(&dist_dir, project_root)?;

    // Phase 2.5: CSS/JS bundling and tree-shaking.
    // Note: `bundled_files` is used in Phase 3 (Task 8) for content hashing.
    // Until Task 8 is applied, prefix with `_` to suppress the unused warning,
    // then remove the underscore when Task 8 wires up the content hash integration.
    let _bundled_files = if config.build.bundling.enabled {
        let files = bundling::bundle_assets(
            &dist_dir, &config.build.bundling, config.build.minify,
        ).wrap_err("CSS/JS bundling failed")?;
        if !files.is_empty() {
            tracing::info!(
                "Bundling: {} file(s) generated.",
                files.len(),
            );
        }
        files
    } else {
        Vec::new()
    };

    // Phase 3: Rewrite remaining asset references in HTML/CSS/JS.
    if config.build.content_hash.enabled && !manifest.is_empty() {
        content_hash::rewrite_references(&dist_dir, &manifest)?;
        tracing::info!("Asset references rewritten.");
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`

Expected: Compiles without errors. The bundling phase is now wired into the pipeline but content hashing integration is not yet complete (Task 8).

- [ ] **Step 5: Commit**

```bash
git add src/build/render.rs
git commit -m "feat(bundling): wire Phase 2.5 into build pipeline

Insert bundle_assets call between post-build plugin hooks and content
hashing. Bundling runs only when config.build.bundling.enabled is true.
Logs the number of generated bundle files."
```

---

## Task 8: Integrate bundling with content hashing

**Depends on:** Task 7 (bundling must be wired into the pipeline)

**Files:**
- Modify: `src/build/content_hash.rs`
- Modify: `src/build/render.rs`

Bundled files exist only in `dist/` (no `static/` counterpart). The existing `build_manifest` only hashes files from `static/`. This task adds a `hash_additional_files` function and updates `rewrite_references` to accept an optional additional manifest.

- [ ] **Step 1: Add `hash_additional_files` to `src/build/content_hash.rs`**

Add the following public function after `build_manifest` (before `rewrite_references`):

```rust
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
```

- [ ] **Step 2: Update `rewrite_references` signature**

Change the `rewrite_references` function signature to accept an optional additional manifest:

Find:

```rust
pub fn rewrite_references(dist_dir: &Path, manifest: &AssetManifest) -> Result<()> {
```

Replace with:

```rust
pub fn rewrite_references(
    dist_dir: &Path,
    manifest: &AssetManifest,
    additional: Option<&AssetManifest>,
) -> Result<()> {
```

- [ ] **Step 3: Update `rewrite_references` and `rewrite_file_content` to use additional manifest**

The current `rewrite_file_content` calls `manifest.pairs_longest_first()` internally. To support an additional manifest, change `rewrite_file_content` to accept pre-built pairs instead, and build merged pairs in `rewrite_references`.

First, update `rewrite_file_content` (around line 256). Find:

```rust
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

Replace with:

```rust
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
```

Then update `rewrite_references` to build merged pairs and pass them. Find:

```rust
pub fn rewrite_references(dist_dir: &Path, manifest: &AssetManifest) -> Result<()> {
    use walkdir::WalkDir;

    if manifest.is_empty() {
        return Ok(());
    }
```

Replace with:

```rust
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
```

Then update the call to `rewrite_file_content` inside the loop (around line 315). Find:

```rust
        if let Some(rewritten) = rewrite_file_content(&content, manifest) {
```

Replace with:

```rust
        if let Some(rewritten) = rewrite_file_content(&content, &all_pairs) {
```

- [ ] **Step 4: Update existing `rewrite_file_content` tests**

The signature change to `rewrite_file_content` breaks existing tests that pass `&AssetManifest`. Update each test call to build pairs first. In the `#[cfg(test)] mod tests` block of `src/build/content_hash.rs`, update the following tests:

```rust
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
```

- [ ] **Step 5: Update existing `rewrite_references` test calls**

The signature change to `rewrite_references` (new third parameter `additional: Option<&AssetManifest>`) breaks existing test calls that pass only 2 arguments. Add `None` as the third argument to each call. In the `#[cfg(test)] mod tests` block of `src/build/content_hash.rs`, update the following calls:

- `rewrite_references(dist, &m).unwrap();` becomes `rewrite_references(dist, &m, None).unwrap();` (6 occurrences)
- `rewrite_references(&dist_dir, &manifest).unwrap();` becomes `rewrite_references(&dist_dir, &manifest, None).unwrap();` (1 occurrence in end-to-end test)

- [ ] **Step 6: Update the call site in `render.rs`**

In `src/build/render.rs`, first rename `_bundled_files` back to `bundled_files` (the underscore prefix was added in Task 7 to suppress the unused variable warning; now that this step uses it, the prefix must be removed).

Then update the Phase 3 content hashing block to use the new signature and handle bundled files. Replace the current Phase 3 block:

Find:

```rust
    // Phase 3: Rewrite remaining asset references in HTML/CSS/JS.
    if config.build.content_hash.enabled && !manifest.is_empty() {
        content_hash::rewrite_references(&dist_dir, &manifest)?;
        tracing::info!("Asset references rewritten.");
    }
```

Replace with:

```rust
    // Phase 3: Content hash rewrite.
    if config.build.content_hash.enabled {
        // Hash bundled files (generated, not from static/).
        let bundle_manifest = if !bundled_files.is_empty() {
            Some(content_hash::hash_additional_files(
                &dist_dir, &bundled_files,
            ).wrap_err("Failed to hash bundled files")?)
        } else {
            None
        };

        if !manifest.is_empty() || bundle_manifest.is_some() {
            content_hash::rewrite_references(
                &dist_dir,
                &manifest,
                bundle_manifest.as_ref(),
            )?;
            tracing::info!("Asset references rewritten.");
        }
    }
```

Also rename `_bundled_files` to `bundled_files` in the Phase 2.5 block above:

Find:
```rust
    let _bundled_files = if config.build.bundling.enabled {
```

Replace with:
```rust
    let bundled_files = if config.build.bundling.enabled {
```

- [ ] **Step 7: Run full test suite to verify nothing is broken**

Run: `cargo test -- --nocapture`

Expected: All tests pass, including existing content_hash tests (with updated signatures) and all bundling tests.

- [ ] **Step 8: Commit**

```bash
git add src/build/content_hash.rs src/build/render.rs
git commit -m "feat(bundling): integrate with content hashing

Add hash_additional_files() for hashing generated bundle files that
have no static/ counterpart. Update rewrite_references() signature
to accept optional additional manifest. Update rewrite_file_content
to accept pre-built pairs. Update all existing tests for new
signatures. Update build() to hash bundled files and merge manifests
during Phase 3 rewriting."
```

---

## Task 9: Write feature documentation

**Depends on:** Tasks 1-8 (feature must be fully implemented)

**Files:**
- Create: `docs/css_js_bundling.md`

- [ ] **Step 1: Create `docs/css_js_bundling.md`**

```markdown
# CSS/JS Bundling and Tree-Shaking

## Overview

CSS/JS bundling merges multiple CSS and JS files into single bundled files,
reduces HTTP requests, and removes unused CSS selectors via tree-shaking.

## Configuration

Add to `site.toml`:

```toml
[build.bundling]
enabled = true
```

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `enabled` | `false` | Master switch (opt-in) |
| `css` | `true` | Bundle CSS files |
| `tree_shake_css` | `true` | Remove unused CSS selectors |
| `js` | `true` | Bundle JS files |
| `css_output` | `"css/bundle.css"` | Output path for CSS bundle |
| `js_output` | `"js/bundle.js"` | Output path for JS bundle |
| `exclude` | `[]` | Glob patterns to exclude from bundling |

### Examples

CSS only (no JS bundling):
```toml
[build.bundling]
enabled = true
js = false
```

Exclude vendor files:
```toml
[build.bundling]
enabled = true
exclude = ["**/vendor/**", "**/print.css"]
```

## Pipeline Position

Runs as Phase 2.5 in the build pipeline:
1. Phase 1: Copy static assets (+ content hash manifest)
2. Phase 2: Render all pages (per-page pipeline)
3. Phase 2 (cont): Sitemap, post-build plugins
4. **Phase 2.5: CSS/JS bundling and tree-shaking** <-- this feature
5. Phase 3: Content hash rewrite

## How It Works

### CSS Bundling
1. Scans all HTML files in `dist/` for `<link rel="stylesheet">` tags
2. Deduplicates hrefs in first-encounter order (sorted by file path)
3. Reads and concatenates CSS files, resolving `@import` chains
4. Tree-shakes: parses all HTML DOMs, keeps selectors matching any page
5. Writes bundled CSS to `dist/{css_output}`

### JS Bundling
1. Scans all HTML files for `<script src="...">` tags
2. Skips: `defer`, `async`, `type="module"`, inline scripts
3. Wraps each file in IIFE: `;(function(){ ... })();`
4. Writes bundled JS to `dist/{js_output}`

### HTML Rewriting
- First `<link>`/`<script>` referencing a bundled file: rewritten to bundle
- Subsequent references: removed
- Critical CSS preload pattern: both preload `<link>` and `<noscript>` fallback rewritten

## Interactions

- **Critical CSS**: Runs before bundling (Phase 2). Bundling rewrites the preload links.
- **Content Hashing**: Runs after bundling (Phase 3). Hashes the bundle files.
- **Dev Server**: Bundling is disabled during dev.

## What Is Skipped
- External URLs (http/https)
- `<link>` with `media` attribute
- `<script>` with `defer`, `async`, or `type="module"`
- Inline `<style>` and `<script>` blocks
- Paths matching `exclude` patterns

## Module Structure
```
src/build/bundling/
  mod.rs      -- public API (bundle_assets), orchestration
  collect.rs  -- HTML scanning, reference collection
  css.rs      -- CSS merging, tree-shaking
  js.rs       -- JS concatenation with IIFE wrapping
  rewrite.rs  -- HTML rewriting
```
```

- [ ] **Step 2: Commit**

```bash
git add docs/css_js_bundling.md
git commit -m "docs(bundling): add CSS/JS bundling feature documentation

Document configuration options, pipeline position, how bundling works,
interactions with critical CSS and content hashing, and module structure."
```

---

## Task 10: Run full test suite and verify end-to-end

**Depends on:** Tasks 1-9

**Files:** No new files

This task runs the complete test suite and verifies the feature works end-to-end.

- [ ] **Step 1: Run all unit tests**

Run: `cargo test --lib -- --nocapture`

Expected: All tests pass. Count total bundling tests: collect (12) + js (4) + css (12) + rewrite (10) + mod (6) + config (4) = 48 bundling-specific tests.

- [ ] **Step 2: Run all tests including integration tests**

Run: `cargo test -- --nocapture`

Expected: All tests pass, including existing integration tests.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -- -D warnings`

Expected: No warnings.

- [ ] **Step 4: Verify a build with bundling enabled (if example site exists)**

Run: `cargo run -- build` from the example site directory (if available), after adding `[build.bundling] enabled = true` to the example site's `site.toml`.

Expected: Build succeeds, `dist/css/bundle.css` and `dist/js/bundle.js` are created, HTML files reference the bundles.

- [ ] **Step 5: Final commit (if any fixes were needed)**

If any fixes were made during this verification pass, commit them:

```bash
git add -A
git commit -m "fix(bundling): address issues found during verification"
```
