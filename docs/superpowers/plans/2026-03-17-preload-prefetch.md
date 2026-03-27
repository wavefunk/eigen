# Preload / Prefetch Hints Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-generate `<link rel="preload">` hints for hero images and `<link rel="prefetch">` hints for navigation links, injecting them into `<head>` to improve LCP and make HTMX navigations feel instant.

**Architecture:** A new `build::hints` module with three files (`mod.rs`, `preload.rs`, `prefetch.rs`) slots into the existing build pipeline after critical CSS inlining and before HTML minification. It uses `lol_html` (already a dependency) for HTML scanning and injection. Hero images are identified via frontmatter `hero_image` or auto-detection of the first qualifying `<img>` tag. Navigation links are collected from `hx-get` attributes (or `<a href>` when fragments are disabled). The feature is enabled by default via `[build.hints]` in `site.toml` -- resource hints are purely additive and cannot break output.

**Tech Stack:** Rust, lol_html (`rewrite_str` API for HTML scan + inject, consistent with existing codebase usage), glob (exclude pattern matching), regex (variant filename parsing for dist scan fallback)

**IMPORTANT — lol_html API:** The codebase uses `lol_html::rewrite_str` with `lol_html::RewriteStrSettings` everywhere (see `src/assets/html_rewrite.rs`, `src/build/critical_css/rewrite.rs`). It does NOT use the streaming `HtmlRewriter` / `Settings` API. All code in this plan uses `rewrite_str` / `RewriteStrSettings` to be consistent. Closures that capture mutable state use `Rc<RefCell<>>` (matching the pattern in `critical_css/rewrite.rs`). CSS attribute selectors in `lol_html::element!()` use single quotes (e.g., `link[rel='preload']`), not escaped double quotes, matching existing codebase style.

**Design spec:** `docs/superpowers/specs/2026-03-17-preload-prefetch-design.md`

---

## File Structure

### New files to create

| File | Responsibility |
|------|---------------|
| `src/build/hints/mod.rs` | Public API (`inject_resource_hints`), orchestration, `ResourceHint` enum, `hints_to_html`, `inject_into_head`, hero image resolution dispatch |
| `src/build/hints/preload.rs` | Hero image detection: auto-detect first qualifying `<img>`, `<picture>`/`<source>` extraction, dist_dir variant scanning, MIME type inference, `HeroPreload` building |
| `src/build/hints/prefetch.rs` | Navigation link scanning: `hx-get`/`href` collection, URL deduplication, self-reference detection, exclude pattern matching, `max_prefetch` limiting |
| `docs/preload_prefetch.md` | Feature documentation for future reference |

### Existing files to modify

| File | Change |
|------|--------|
| `src/config/mod.rs` | Add `HintsConfig` struct, add `hints` field to `BuildConfig`, update `Default` impl, add `default_max_prefetch` and `default_image_sizes` functions |
| `src/frontmatter/mod.rs` | Add `hero_image: Option<String>` to `Frontmatter` and `RawFrontmatter`, update `parse_frontmatter` and `Default` impl |
| `src/build/mod.rs` | Add `pub mod hints;` declaration |
| `src/build/render.rs` | Insert hints step after critical CSS inlining, before minification, in both `render_static_page` and `render_dynamic_page` |

---

## Task 1: Add `HintsConfig` to configuration

**Depends on:** Nothing (starting point)

**Files:**
- Modify: `src/config/mod.rs`

- [ ] **Step 1: Add `HintsConfig` struct**

Add the following after the `CriticalCssConfig` struct and its `Default` impl (after line 216 in `src/config/mod.rs`). Reuse the existing `default_true` function (line 69).

```rust
/// Configuration for preload and prefetch resource hints.
///
/// Located under `[build.hints]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct HintsConfig {
    /// Master switch for resource hints. Default: true.
    /// When false, no preload or prefetch hints are generated.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Whether to auto-detect the hero image from rendered HTML when
    /// no `hero_image` is set in frontmatter. Default: true.
    #[serde(default = "default_true")]
    pub auto_detect_hero: bool,

    /// Whether to generate prefetch hints for navigation links.
    /// Default: true.
    #[serde(default = "default_true")]
    pub prefetch_links: bool,

    /// Maximum number of `<link rel="prefetch">` hints per page.
    /// Default: 5.
    #[serde(default = "default_max_prefetch")]
    pub max_prefetch: usize,

    /// Fallback value for the `imagesizes` attribute on hero image
    /// preload hints. Default: "100vw".
    #[serde(default = "default_image_sizes")]
    pub hero_image_sizes: String,

    /// Glob patterns for link hrefs to exclude from prefetching.
    #[serde(default)]
    pub exclude_prefetch: Vec<String>,
}

fn default_max_prefetch() -> usize {
    5
}

fn default_image_sizes() -> String {
    "100vw".to_string()
}

impl Default for HintsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_detect_hero: true,
            prefetch_links: true,
            max_prefetch: default_max_prefetch(),
            hero_image_sizes: default_image_sizes(),
            exclude_prefetch: Vec::new(),
        }
    }
}
```

- [ ] **Step 2: Add `hints` field to `BuildConfig`**

In `src/config/mod.rs`, add a new field to the `BuildConfig` struct (after the `critical_css` field, around line 53):

```rust
    /// Preload/prefetch resource hints configuration.
    #[serde(default)]
    pub hints: HintsConfig,
```

Update the `Default` impl for `BuildConfig` (around line 56) to include:

```rust
            hints: HintsConfig::default(),
```

(Add this line after the `critical_css: CriticalCssConfig::default(),` line.)

- [ ] **Step 3: Write tests for hints config**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/config/mod.rs`:

```rust
    // --- Hints config tests ---

    #[test]
    fn test_hints_config_defaults() {
        let toml_str = r#"
[site]
name = "Hints Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.hints.enabled);
        assert!(config.build.hints.auto_detect_hero);
        assert!(config.build.hints.prefetch_links);
        assert_eq!(config.build.hints.max_prefetch, 5);
        assert_eq!(config.build.hints.hero_image_sizes, "100vw");
        assert!(config.build.hints.exclude_prefetch.is_empty());
    }

    #[test]
    fn test_hints_config_custom() {
        let toml_str = r#"
[site]
name = "Hints Custom"
base_url = "https://example.com"

[build.hints]
enabled = true
auto_detect_hero = false
prefetch_links = true
max_prefetch = 3
hero_image_sizes = "(max-width: 1200px) 100vw, 1200px"
exclude_prefetch = ["**/archive/**"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.hints.enabled);
        assert!(!config.build.hints.auto_detect_hero);
        assert!(config.build.hints.prefetch_links);
        assert_eq!(config.build.hints.max_prefetch, 3);
        assert_eq!(
            config.build.hints.hero_image_sizes,
            "(max-width: 1200px) 100vw, 1200px"
        );
        assert_eq!(config.build.hints.exclude_prefetch.len(), 1);
    }

    #[test]
    fn test_hints_config_disabled() {
        let toml_str = r#"
[site]
name = "Hints Off"
base_url = "https://example.com"

[build.hints]
enabled = false
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.hints.enabled);
        // Other fields should have defaults.
        assert!(config.build.hints.auto_detect_hero);
        assert_eq!(config.build.hints.max_prefetch, 5);
    }
```

- [ ] **Step 4: Run tests to verify config changes compile and pass**

Run: `cargo test --lib config -- --nocapture`

Expected: All existing config tests pass, plus the 3 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat(hints): add HintsConfig and wire into BuildConfig

Add HintsConfig struct with enabled, auto_detect_hero, prefetch_links,
max_prefetch, hero_image_sizes, and exclude_prefetch fields. Defaults
to enabled (resource hints are purely additive). Wire into BuildConfig
under [build.hints] in site.toml."
```

---

## Task 2: Add `hero_image` to frontmatter

**Depends on:** Nothing (independent of Task 1)

**Files:**
- Modify: `src/frontmatter/mod.rs`

- [ ] **Step 1: Add `hero_image` field to `Frontmatter`**

In `src/frontmatter/mod.rs`, add a new field to the `Frontmatter` struct (after `fragment_blocks`, around line 22):

```rust
    /// Path to the hero/LCP image for this page.
    ///
    /// When set, a `<link rel="preload">` hint is injected into `<head>`
    /// for this image, improving Largest Contentful Paint.
    /// The path should be relative to the site root (e.g. "/assets/hero.jpg").
    pub hero_image: Option<String>,
```

- [ ] **Step 2: Add `hero_image` to `RawFrontmatter`**

In `src/frontmatter/mod.rs`, add a new field to the `RawFrontmatter` struct (after `fragment_blocks`, around line 68):

```rust
    hero_image: Option<String>,
```

- [ ] **Step 3: Update `parse_frontmatter` to map the field**

In `src/frontmatter/mod.rs`, update the `Ok(Frontmatter { ... })` block in `parse_frontmatter` (around line 136) to include:

```rust
        hero_image: raw.hero_image,
```

(Add after the `fragment_blocks: raw.fragment_blocks,` line.)

- [ ] **Step 4: Update `Frontmatter::default()`**

In the `Default` impl for `Frontmatter` (around line 24), add after `fragment_blocks: None,`:

```rust
            hero_image: None,
```

- [ ] **Step 5: Write tests for hero_image frontmatter**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/frontmatter/mod.rs`:

```rust
    #[test]
    fn test_parse_hero_image_frontmatter() {
        let yaml = "hero_image: /assets/hero-banner.jpg\n";
        let fm = parse_frontmatter(yaml, "index.html").unwrap();
        assert_eq!(fm.hero_image.as_deref(), Some("/assets/hero-banner.jpg"));
    }

    #[test]
    fn test_parse_no_hero_image() {
        let yaml = "data:\n  nav:\n    file: \"nav.yaml\"\n";
        let fm = parse_frontmatter(yaml, "index.html").unwrap();
        assert!(fm.hero_image.is_none());
    }

    #[test]
    fn test_default_hero_image_is_none() {
        let fm = Frontmatter::default();
        assert!(fm.hero_image.is_none());
    }
```

- [ ] **Step 6: Run tests to verify frontmatter changes compile and pass**

Run: `cargo test --lib frontmatter -- --nocapture`

Expected: All existing frontmatter tests pass, plus the 3 new tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/frontmatter/mod.rs
git commit -m "feat(hints): add hero_image field to Frontmatter

Add optional hero_image field to Frontmatter and RawFrontmatter structs.
When set in template YAML frontmatter, this designates the LCP image
for preload hint generation."
```

---

## Task 3: Create `hints/prefetch.rs` -- navigation link scanning

**Depends on:** Task 1 (needs `HintsConfig` for exclude patterns and max_prefetch)

**Files:**
- Create: `src/build/hints/prefetch.rs`

This task builds the navigation prefetch URL collector. It is independent of the preload logic and can be developed and tested in isolation.

- [ ] **Step 1: Create `prefetch.rs` with `collect_prefetch_urls`**

Create the file `src/build/hints/prefetch.rs`:

```rust
//! Navigation prefetch: scan HTML for hx-get/href attributes, collect
//! unique local URLs, filter self-references and exclusions.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Scan HTML for navigation link URLs suitable for prefetching.
///
/// When `fragments_enabled` is true, scans `hx-get` attributes.
/// When false, scans `<a href>` attributes (excluding `target="_blank"`).
///
/// Excludes:
/// - External URLs (http:// or https://)
/// - Anchor-only links (#...)
/// - The current page's own URL (self-reference)
/// - URLs matching exclude patterns (glob)
/// - Duplicates (first occurrence wins)
///
/// Returns at most `max_prefetch` URLs in document order.
pub fn collect_prefetch_urls(
    html: &str,
    current_url: &str,
    fragment_dir: &str,
    fragments_enabled: bool,
    max_prefetch: usize,
    exclude_patterns: &[String],
) -> Vec<String> {
    if max_prefetch == 0 {
        return Vec::new();
    }

    // Compile exclude patterns, skipping invalid ones with a warning.
    let patterns: Vec<glob::Pattern> = exclude_patterns
        .iter()
        .filter_map(|p| {
            match glob::Pattern::new(p) {
                Ok(pat) => Some(pat),
                Err(e) => {
                    tracing::warn!(
                        "Invalid glob pattern in exclude_prefetch '{}': {}",
                        p, e
                    );
                    None
                }
            }
        })
        .collect();

    // Derive the self-reference URL to exclude.
    let self_url = compute_self_url(current_url, fragment_dir, fragments_enabled);

    // Collect URLs from HTML via lol_html.
    let urls = if fragments_enabled {
        collect_hx_get_urls(html)
    } else {
        collect_href_urls(html)
    };

    // Filter and deduplicate.
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for url in urls {
        if result.len() >= max_prefetch {
            break;
        }

        // Skip external URLs.
        if url.starts_with("http://") || url.starts_with("https://") {
            continue;
        }

        // Skip anchor-only links.
        if url.starts_with('#') {
            continue;
        }

        // Skip empty URLs.
        if url.is_empty() {
            continue;
        }

        // Normalize for self-reference check.
        let normalized = normalize_url(&url);
        if normalized == self_url {
            continue;
        }

        // Skip excluded patterns.
        if patterns.iter().any(|p| p.matches(&url)) {
            continue;
        }

        // Deduplicate.
        if !seen.insert(url.clone()) {
            continue;
        }

        result.push(url);
    }

    result
}

/// Compute the URL that represents "this page" for self-reference detection.
///
/// When fragments are enabled, the self URL is the fragment path
/// (e.g., `/_fragments/about.html`). When disabled, it is the page URL itself.
fn compute_self_url(current_url: &str, fragment_dir: &str, fragments_enabled: bool) -> String {
    let normalized = normalize_url(current_url);
    if fragments_enabled {
        let clean = normalized.trim_start_matches('/');
        format!("/{}/{}", fragment_dir, clean)
    } else {
        normalized
    }
}

/// Normalize a URL for comparison: treat trailing `/` as `/index.html`.
fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url == "/" || url.is_empty() {
        return "/index.html".to_string();
    }
    if url.ends_with('/') {
        return format!("{}index.html", url);
    }
    url.to_string()
}

/// Collect all `hx-get` attribute values from HTML elements.
fn collect_hx_get_urls(html: &str) -> Vec<String> {
    let urls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let urls_clone = urls.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("*[hx-get]", move |el| {
                    if let Some(val) = el.get_attribute("hx-get") {
                        urls_clone.borrow_mut().push(val);
                    }
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = urls.borrow().clone();
    result
}

/// Collect `href` attribute values from `<a>` elements, excluding
/// links with `target="_blank"`.
fn collect_href_urls(html: &str) -> Vec<String> {
    let urls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let urls_clone = urls.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("a[href]", move |el| {
                    // Skip target="_blank" links.
                    if let Some(target) = el.get_attribute("target") {
                        if target == "_blank" {
                            return Ok(());
                        }
                    }
                    if let Some(val) = el.get_attribute("href") {
                        urls_clone.borrow_mut().push(val);
                    }
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = urls.borrow().clone();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_hx_get_urls() {
        let html = r#"
            <a href="/about.html" hx-get="/_fragments/about.html" hx-target="#content">About</a>
            <a href="/blog.html" hx-get="/_fragments/blog.html" hx-target="#content">Blog</a>
        "#;
        let urls = collect_hx_get_urls(html);
        assert_eq!(urls, vec!["/_fragments/about.html", "/_fragments/blog.html"]);
    }

    #[test]
    fn test_collect_href_urls_no_fragments() {
        let html = r#"
            <a href="/about.html">About</a>
            <a href="/blog.html">Blog</a>
            <a href="https://external.com" target="_blank">Ext</a>
        "#;
        let urls = collect_href_urls(html);
        assert_eq!(urls, vec!["/about.html", "/blog.html"]);
    }

    #[test]
    fn test_excludes_external_urls() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="https://evil.com/steal">Bad</a>
        "#;
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", true, 10, &[],
        );
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_self_reference() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/index.html">Home</a>
        "#;
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", true, 10, &[],
        );
        // Self-reference to /index.html (-> /_fragments/index.html) should be excluded.
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_self_reference_index() {
        // Test that "/" and "/index.html" are recognized as the same page.
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/index.html">Home</a>
        "#;
        // current_url is "/" -- should still match /_fragments/index.html
        let result = collect_prefetch_urls(
            html, "/", "_fragments", true, 10, &[],
        );
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_pattern_match() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/archive/2023.html">Archive</a>
        "#;
        let result = collect_prefetch_urls(
            html,
            "/index.html",
            "_fragments",
            true,
            10,
            &["**/archive/**".to_string()],
        );
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_max_prefetch_limit() {
        let html = r#"
            <a hx-get="/_fragments/a.html">A</a>
            <a hx-get="/_fragments/b.html">B</a>
            <a hx-get="/_fragments/c.html">C</a>
            <a hx-get="/_fragments/d.html">D</a>
        "#;
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", true, 2, &[],
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result, vec!["/_fragments/a.html", "/_fragments/b.html"]);
    }

    #[test]
    fn test_deduplicates_urls() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="/_fragments/about.html">About Again</a>
            <a hx-get="/_fragments/blog.html">Blog</a>
        "#;
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", true, 10, &[],
        );
        assert_eq!(result, vec!["/_fragments/about.html", "/_fragments/blog.html"]);
    }

    #[test]
    fn test_excludes_anchor_only() {
        let html = r#"
            <a hx-get="/_fragments/about.html">About</a>
            <a hx-get="#section">Section</a>
        "#;
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", true, 10, &[],
        );
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_excludes_target_blank() {
        let html = r#"
            <a href="/about.html">About</a>
            <a href="/ext.html" target="_blank">External</a>
        "#;
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", false, 10, &[],
        );
        assert_eq!(result, vec!["/about.html"]);
    }

    #[test]
    fn test_max_prefetch_zero() {
        let html = r#"<a hx-get="/_fragments/about.html">About</a>"#;
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", true, 0, &[],
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_normalize_url_trailing_slash() {
        assert_eq!(normalize_url("/posts/"), "/posts/index.html");
        assert_eq!(normalize_url("/"), "/index.html");
        assert_eq!(normalize_url("/about.html"), "/about.html");
    }

    #[test]
    fn test_compute_self_url_fragments_enabled() {
        let result = compute_self_url("/about.html", "_fragments", true);
        assert_eq!(result, "/_fragments/about.html");
    }

    #[test]
    fn test_compute_self_url_fragments_disabled() {
        let result = compute_self_url("/about.html", "_fragments", false);
        assert_eq!(result, "/about.html");
    }

    #[test]
    fn test_glob_pattern_error_handled() {
        // An invalid glob pattern should not crash -- it should be skipped.
        let html = r#"<a hx-get="/_fragments/about.html">About</a>"#;
        let result = collect_prefetch_urls(
            html,
            "/index.html",
            "_fragments",
            true,
            10,
            &["[invalid".to_string()],
        );
        // The invalid pattern is skipped; the URL is still collected.
        assert_eq!(result, vec!["/_fragments/about.html"]);
    }

    #[test]
    fn test_no_links_in_html() {
        let html = "<html><head></head><body><p>No links here</p></body></html>";
        let result = collect_prefetch_urls(
            html, "/index.html", "_fragments", true, 10, &[],
        );
        assert!(result.is_empty());
    }
}
```

- [ ] **Step 2: Create a minimal `mod.rs` to make the module compilable**

Create `src/build/hints/mod.rs` with a minimal stub so the module compiles:

```rust
//! Preload/prefetch resource hints: inject `<link rel="preload">` for hero
//! images and `<link rel="prefetch">` for navigation links into `<head>`.

pub mod prefetch;
```

- [ ] **Step 3: Register the module in `src/build/mod.rs`**

Add `pub mod hints;` after the `pub mod fragments;` line in `src/build/mod.rs`:

```rust
pub mod hints;
```

- [ ] **Step 4: Run tests to verify prefetch module compiles and passes**

Run: `cargo test --lib build::hints::prefetch -- --nocapture`

Expected: All 16 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/build/hints/prefetch.rs src/build/hints/mod.rs src/build/mod.rs
git commit -m "feat(hints): add prefetch URL collector

Implement collect_prefetch_urls that scans HTML for hx-get (or href when
fragments disabled) attributes. Handles deduplication, self-reference
exclusion, external URL filtering, anchor-only skipping, glob-based
exclude patterns, and max_prefetch limiting."
```

---

## Task 4: Create `hints/preload.rs` -- hero image preload

**Depends on:** Task 1 (needs `HintsConfig`), Task 3 (needs `mod.rs` to exist)

**Files:**
- Create: `src/build/hints/preload.rs`
- Modify: `src/build/hints/mod.rs` (add module declaration)

This task builds the hero image preload hint generator. It handles auto-detection from HTML, `<picture>` element extraction, and the dist_dir variant scan fallback.

- [ ] **Step 1: Create `preload.rs` with hero image resolution and preload building**

Create the file `src/build/hints/preload.rs`:

```rust
//! Hero image preload: detect the hero image, extract responsive variant
//! information from `<picture>` elements or dist_dir scanning, and build
//! the preload `<link>` tag data.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Data needed to build a hero image `<link rel="preload">` tag.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroPreload {
    /// The image URL path (e.g. "/assets/hero.jpg").
    pub href: String,
    /// MIME type of the image (e.g. "image/avif").
    pub mime_type: String,
    /// If the image has responsive variants, the srcset string.
    pub imagesrcset: Option<String>,
    /// The `imagesizes` value (e.g. "100vw").
    pub imagesizes: Option<String>,
}

/// Determine the hero image URL for a page.
///
/// Priority:
/// 1. Explicit `hero_image` from frontmatter.
/// 2. Auto-detected first `<img>` without `loading="lazy"` (if enabled).
/// 3. None.
pub fn resolve_hero_image(
    html: &str,
    frontmatter_hero: Option<&str>,
    auto_detect: bool,
) -> Option<String> {
    // Priority 1: frontmatter.
    if let Some(hero) = frontmatter_hero {
        let hero = hero.trim();
        if !hero.is_empty() {
            return Some(hero.to_string());
        }
    }

    // Priority 2: auto-detect.
    if auto_detect {
        return auto_detect_hero_image(html);
    }

    None
}

/// Build the hero preload hint by extracting responsive variant
/// information from the rendered HTML's `<picture>` elements.
///
/// First checks for a `<picture>` element with a matching `<img src>`.
/// If found, extracts `srcset` and `type` from the first `<source>`.
/// If not found, falls back to scanning dist_dir for variant files.
pub fn build_hero_preload(
    hero_src: &str,
    html: &str,
    dist_dir: &Path,
    image_sizes_fallback: &str,
) -> HeroPreload {
    // Try to find a <picture> element containing this hero image.
    if let Some(preload) = extract_from_picture(hero_src, html, image_sizes_fallback) {
        return preload;
    }

    // Fallback: scan dist_dir for variant files.
    if let Some(preload) = scan_dist_for_variants(hero_src, dist_dir, image_sizes_fallback) {
        return preload;
    }

    // Last resort: simple preload with just href and inferred type.
    HeroPreload {
        href: hero_src.to_string(),
        mime_type: infer_mime_type(hero_src),
        imagesrcset: None,
        imagesizes: None,
    }
}

/// Auto-detect the hero image from rendered HTML.
///
/// Finds the first `<img>` that is not:
/// - `loading="lazy"`
/// - smaller than 100px in both width and height
/// - `role="presentation"` or empty `alt=""`
fn auto_detect_hero_image(html: &str) -> Option<String> {
    let found: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let found_clone = found.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("img[src]", move |el| {
                    // Only take the first qualifying image.
                    if found_clone.borrow().is_some() {
                        return Ok(());
                    }

                    // Skip lazy-loaded images.
                    if let Some(loading) = el.get_attribute("loading") {
                        if loading.eq_ignore_ascii_case("lazy") {
                            return Ok(());
                        }
                    }

                    // Skip small icons (both dimensions < 100px).
                    let width = el.get_attribute("width")
                        .and_then(|v| v.parse::<u32>().ok());
                    let height = el.get_attribute("height")
                        .and_then(|v| v.parse::<u32>().ok());
                    if let (Some(w), Some(h)) = (width, height) {
                        if w < 100 && h < 100 {
                            return Ok(());
                        }
                    }

                    // Skip decorative images.
                    if let Some(role) = el.get_attribute("role") {
                        if role == "presentation" {
                            return Ok(());
                        }
                    }
                    if let Some(alt) = el.get_attribute("alt") {
                        if alt.is_empty() {
                            return Ok(());
                        }
                    }

                    if let Some(src) = el.get_attribute("src") {
                        if !src.is_empty() {
                            *found_clone.borrow_mut() = Some(src);
                        }
                    }

                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = found.borrow().clone();
    result
}

/// Extract preload data from a `<picture>` element whose `<img>` has a
/// matching `src`. Returns the srcset and type from the first `<source>`.
fn extract_from_picture(hero_src: &str, html: &str, image_sizes_fallback: &str) -> Option<HeroPreload> {
    // State tracking for the streaming parser.
    // We track whether we are inside a <picture> that contains our hero <img>.
    let in_picture: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_match: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let first_source: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
    let img_sizes: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Since lol_html processes elements in document order, we track
    // picture/source/img elements as we encounter them.

    let hero_src_owned = hero_src.to_string();

    let in_picture_c1 = in_picture.clone();
    let first_source_c1 = first_source.clone();
    let img_sizes_c1 = img_sizes.clone();
    let in_picture_c2 = in_picture.clone();
    let first_source_c2 = first_source.clone();
    let in_picture_c3 = in_picture.clone();
    let found_match_c = found_match.clone();
    let img_sizes_c2 = img_sizes.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("picture", move |_el| {
                    *in_picture_c1.borrow_mut() = true;
                    *first_source_c1.borrow_mut() = None;
                    *img_sizes_c1.borrow_mut() = None;
                    Ok(())
                }),
                lol_html::element!("source[srcset]", move |el| {
                    if *in_picture_c2.borrow() && first_source_c2.borrow().is_none() {
                        let srcset = el.get_attribute("srcset").unwrap_or_default();
                        let source_type = el.get_attribute("type").unwrap_or_default();
                        if !srcset.is_empty() && !source_type.is_empty() {
                            *first_source_c2.borrow_mut() = Some((srcset, source_type));
                        }
                    }
                    Ok(())
                }),
                lol_html::element!("img[src]", move |el| {
                    if *in_picture_c3.borrow() {
                        if let Some(src) = el.get_attribute("src") {
                            if src == hero_src_owned {
                                *found_match_c.borrow_mut() = true;
                                if let Some(sizes) = el.get_attribute("sizes") {
                                    *img_sizes_c2.borrow_mut() = Some(sizes);
                                }
                            }
                        }
                        // Reset picture tracking (each picture is independent).
                        *in_picture_c3.borrow_mut() = false;
                    }
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    if !*found_match.borrow() {
        return None;
    }

    let source_data = first_source.borrow().clone();
    match source_data {
        Some((srcset, source_type)) => {
            let sizes = img_sizes.borrow().clone()
                .unwrap_or_else(|| image_sizes_fallback.to_string());
            Some(HeroPreload {
                href: hero_src.to_string(),
                mime_type: source_type,
                imagesrcset: Some(srcset),
                imagesizes: Some(sizes),
            })
        }
        None => {
            // Picture found but no <source> with srcset/type -- fallback to simple preload.
            None
        }
    }
}

/// Scan dist_dir for responsive variants of an image.
///
/// Looks for files matching `{stem}-*w-*.{format}` in the same directory.
/// Groups by format, prefers AVIF, then WebP, then original.
fn scan_dist_for_variants(
    hero_src: &str,
    dist_dir: &Path,
    image_sizes_fallback: &str,
) -> Option<HeroPreload> {
    // Extract the directory and stem from the hero image path.
    let hero_path = Path::new(hero_src);
    let stem = hero_path.file_stem()?.to_str()?;
    let parent = hero_path.parent()?;

    // Build the dist directory path for this image's directory.
    let search_dir = dist_dir.join(parent.to_str()?.trim_start_matches('/'));

    if !search_dir.is_dir() {
        return None;
    }

    // Scan for variant files matching {stem}-{width}w-{hash}.{format}
    let variant_pattern = format!("{}-*w-*.*", stem);
    let variant_re = match regex::Regex::new(&format!(
        r"^{}-(\d+)w-[a-f0-9]+\.(\w+)$",
        regex::escape(stem)
    )) {
        Ok(re) => re,
        Err(_) => return None,
    };

    let entries: Vec<_> = match std::fs::read_dir(&search_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                glob::Pattern::new(&variant_pattern)
                    .map(|p| p.matches(&name))
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            tracing::debug!("Failed to read dir for variant scan: {}", e);
            return None;
        }
    };

    if entries.is_empty() {
        return None;
    }

    // Group variants by format, preferring AVIF > WebP > original.
    let mut avif_variants: Vec<(String, u32)> = Vec::new();
    let mut webp_variants: Vec<(String, u32)> = Vec::new();
    let mut other_variants: Vec<(String, u32)> = Vec::new();

    let parent_str = parent.to_str()?;

    for entry in &entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(caps) = variant_re.captures(&name_str) {
            let width: u32 = match caps[1].parse() {
                Ok(w) => w,
                Err(_) => continue,
            };
            let format = &caps[2];
            let url = format!("{}/{}", parent_str, name_str);

            match format {
                "avif" => avif_variants.push((url, width)),
                "webp" => webp_variants.push((url, width)),
                _ => other_variants.push((url, width)),
            }
        }
    }

    // Pick best format group.
    let (variants, mime_type): (Vec<(String, u32)>, String) = if !avif_variants.is_empty() {
        (avif_variants, "image/avif".to_string())
    } else if !webp_variants.is_empty() {
        (webp_variants, "image/webp".to_string())
    } else if !other_variants.is_empty() {
        let mime = infer_mime_type(&other_variants[0].0);
        (other_variants, mime)
    } else {
        return None;
    };

    // Build srcset string: "url1 480w, url2 768w, ..."
    let mut sorted = variants;
    sorted.sort_by_key(|(_, w)| *w);
    let srcset: String = sorted
        .iter()
        .map(|(url, w)| format!("{} {}w", url, w))
        .collect::<Vec<_>>()
        .join(", ");

    Some(HeroPreload {
        href: hero_src.to_string(),
        mime_type,
        imagesrcset: Some(srcset),
        imagesizes: Some(image_sizes_fallback.to_string()),
    })
}

/// Infer MIME type from a file path's extension.
pub fn infer_mime_type(path: &str) -> String {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        _ => "image/jpeg", // fallback
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_hero_frontmatter_priority() {
        let html = r#"<img src="/assets/auto.jpg">"#;
        let result = resolve_hero_image(html, Some("/assets/hero.jpg"), true);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_resolve_hero_auto_detect() {
        let html = r#"<html><body><img src="/assets/banner.jpg" alt="Hero"></body></html>"#;
        let result = resolve_hero_image(html, None, true);
        assert_eq!(result, Some("/assets/banner.jpg".to_string()));
    }

    #[test]
    fn test_resolve_hero_auto_detect_disabled() {
        let html = r#"<img src="/assets/banner.jpg" alt="Hero">"#;
        let result = resolve_hero_image(html, None, false);
        assert!(result.is_none());
    }

    #[test]
    fn test_auto_detect_skips_lazy() {
        let html = r#"
            <img src="/assets/lazy.jpg" loading="lazy" alt="Lazy">
            <img src="/assets/hero.jpg" alt="Hero">
        "#;
        let result = auto_detect_hero_image(html);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_auto_detect_skips_small_icons() {
        let html = r#"
            <img src="/assets/icon.png" width="32" height="32" alt="Icon">
            <img src="/assets/hero.jpg" alt="Hero">
        "#;
        let result = auto_detect_hero_image(html);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_auto_detect_skips_decorative() {
        let html = r#"
            <img src="/assets/spacer.png" role="presentation">
            <img src="/assets/decorative.png" alt="">
            <img src="/assets/hero.jpg" alt="Hero">
        "#;
        let result = auto_detect_hero_image(html);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_auto_detect_no_qualifying_img() {
        let html = r#"
            <img src="/assets/icon.png" width="16" height="16" alt="x">
            <img src="/assets/lazy.jpg" loading="lazy" alt="Lazy">
        "#;
        let result = auto_detect_hero_image(html);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_from_picture() {
        let html = r#"
            <picture>
                <source srcset="/assets/hero-480w-abc.avif 480w, /assets/hero-768w-abc.avif 768w" type="image/avif">
                <source srcset="/assets/hero-480w-abc.webp 480w, /assets/hero-768w-abc.webp 768w" type="image/webp">
                <img src="/assets/hero.jpg" alt="Hero" sizes="100vw">
            </picture>
        "#;
        let result = extract_from_picture("/assets/hero.jpg", html, "100vw");
        assert!(result.is_some());
        let preload = result.unwrap();
        assert_eq!(preload.href, "/assets/hero.jpg");
        assert_eq!(preload.mime_type, "image/avif");
        assert!(preload.imagesrcset.unwrap().contains("480w"));
        assert_eq!(preload.imagesizes, Some("100vw".to_string()));
    }

    #[test]
    fn test_extract_from_picture_avif_preferred() {
        // The first <source> is AVIF (best format), so preload uses it.
        let html = r#"
            <picture>
                <source srcset="/hero-480w.avif 480w" type="image/avif">
                <source srcset="/hero-480w.webp 480w" type="image/webp">
                <img src="/hero.jpg" alt="Hero">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        let preload = result.unwrap();
        assert_eq!(preload.mime_type, "image/avif");
    }

    #[test]
    fn test_extract_from_picture_sizes_from_img() {
        let html = r#"
            <picture>
                <source srcset="/hero-480w.avif 480w" type="image/avif">
                <img src="/hero.jpg" sizes="(max-width: 600px) 100vw, 50vw" alt="Hero">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        let preload = result.unwrap();
        assert_eq!(
            preload.imagesizes,
            Some("(max-width: 600px) 100vw, 50vw".to_string())
        );
    }

    #[test]
    fn test_extract_from_picture_sizes_fallback() {
        let html = r#"
            <picture>
                <source srcset="/hero-480w.avif 480w" type="image/avif">
                <img src="/hero.jpg" alt="Hero">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        let preload = result.unwrap();
        // No sizes on <img>, so fallback is used.
        assert_eq!(preload.imagesizes, Some("100vw".to_string()));
    }

    #[test]
    fn test_extract_no_matching_picture() {
        let html = r#"
            <picture>
                <source srcset="/other-480w.avif 480w" type="image/avif">
                <img src="/other.jpg" alt="Other">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        assert!(result.is_none());
    }

    #[test]
    fn test_build_hero_preload_simple_no_variants() {
        // When there's no <picture> and no variants on disk, generate simple preload.
        let html = r#"<img src="/assets/hero.jpg" alt="Hero">"#;
        let dir = tempfile::tempdir().unwrap();
        let preload = build_hero_preload("/assets/hero.jpg", html, dir.path(), "100vw");
        assert_eq!(preload.href, "/assets/hero.jpg");
        assert_eq!(preload.mime_type, "image/jpeg");
        assert!(preload.imagesrcset.is_none());
        assert!(preload.imagesizes.is_none());
    }

    #[test]
    fn test_build_hero_preload_from_picture() {
        let html = r#"
            <picture>
                <source srcset="/assets/hero-480w-abc.avif 480w" type="image/avif">
                <img src="/assets/hero.jpg" alt="Hero">
            </picture>
        "#;
        let dir = tempfile::tempdir().unwrap();
        let preload = build_hero_preload("/assets/hero.jpg", html, dir.path(), "100vw");
        assert_eq!(preload.mime_type, "image/avif");
        assert!(preload.imagesrcset.is_some());
    }

    #[test]
    fn test_infer_mime_type() {
        assert_eq!(infer_mime_type("/img.jpg"), "image/jpeg");
        assert_eq!(infer_mime_type("/img.jpeg"), "image/jpeg");
        assert_eq!(infer_mime_type("/img.png"), "image/png");
        assert_eq!(infer_mime_type("/img.webp"), "image/webp");
        assert_eq!(infer_mime_type("/img.avif"), "image/avif");
        assert_eq!(infer_mime_type("/img.gif"), "image/gif");
        assert_eq!(infer_mime_type("/img.svg"), "image/svg+xml");
    }

    #[test]
    fn test_scan_dist_for_variants_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();

        // Create variant files matching the pattern.
        std::fs::write(assets_dir.join("hero-480w-abcdef12.avif"), b"fake").unwrap();
        std::fs::write(assets_dir.join("hero-768w-abcdef12.avif"), b"fake").unwrap();
        std::fs::write(assets_dir.join("hero-480w-abcdef12.webp"), b"fake").unwrap();

        let result = scan_dist_for_variants(
            "/assets/hero.jpg",
            dir.path(),
            "100vw",
        );
        assert!(result.is_some());
        let preload = result.unwrap();
        // Should prefer AVIF.
        assert_eq!(preload.mime_type, "image/avif");
        let srcset = preload.imagesrcset.unwrap();
        assert!(srcset.contains("480w"));
        assert!(srcset.contains("768w"));
        // Should not contain webp variants in the AVIF srcset.
        assert!(!srcset.contains("webp"));
    }

    #[test]
    fn test_scan_dist_no_variants() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();

        let result = scan_dist_for_variants(
            "/assets/hero.jpg",
            dir.path(),
            "100vw",
        );
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Add preload module declaration to `mod.rs`**

In `src/build/hints/mod.rs`, add the module declaration:

```rust
pub mod preload;
```

(Add after the existing `pub mod prefetch;` line.)

- [ ] **Step 3: Run tests to verify preload module compiles and passes**

Run: `cargo test --lib build::hints::preload -- --nocapture`

Expected: All 17 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/build/hints/preload.rs src/build/hints/mod.rs
git commit -m "feat(hints): add hero image preload builder

Implement hero image resolution (frontmatter priority, auto-detection
fallback), <picture>/<source> extraction for responsive variants,
dist_dir variant scanning fallback, and MIME type inference."
```

---

## Task 5: Complete `hints/mod.rs` -- orchestration and injection

**Depends on:** Tasks 1, 2, 3, 4 (needs config, frontmatter, prefetch, and preload)

**Files:**
- Modify: `src/build/hints/mod.rs`

This task wires everything together: the public `inject_resource_hints` function, `ResourceHint` enum, `hints_to_html` serializer, `inject_into_head` via lol_html, and duplicate preload detection.

- [ ] **Step 1: Rewrite `mod.rs` with full orchestration**

Replace the contents of `src/build/hints/mod.rs` with:

```rust
//! Preload/prefetch resource hints: inject `<link rel="preload">` for hero
//! images and `<link rel="prefetch">` for navigation links into `<head>`.
//!
//! This module provides `inject_resource_hints`, the main entry point called
//! from the build pipeline. It:
//! 1. Resolves the hero image (frontmatter or auto-detection).
//! 2. Builds a preload hint with responsive variant info.
//! 3. Collects navigation link URLs for prefetching.
//! 4. Injects all hints as `<link>` tags early in `<head>`.

pub mod prefetch;
pub mod preload;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::config::HintsConfig;

/// A resource hint to be injected into the page's `<head>`.
#[derive(Debug, Clone, PartialEq)]
enum ResourceHint {
    /// `<link rel="preload">` for the hero image.
    HeroPreload(preload::HeroPreload),
    /// `<link rel="prefetch">` for a likely next navigation.
    NavigationPrefetch {
        /// The fragment or page URL to prefetch.
        href: String,
        /// Whether this targets a full document (vs a fragment).
        is_document: bool,
    },
}

/// Inject preload and prefetch resource hints into HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
pub fn inject_resource_hints(
    html: &str,
    config: &HintsConfig,
    dist_dir: &Path,
    hero_image: Option<&str>,
    current_url: &str,
    fragment_dir: &str,
    fragments_enabled: bool,
) -> String {
    if !config.enabled {
        return html.to_string();
    }

    let mut hints: Vec<ResourceHint> = Vec::new();

    // 1. Hero image preload.
    if let Some(hero_src) = preload::resolve_hero_image(
        html,
        hero_image,
        config.auto_detect_hero,
    ) {
        // Check if there's already a preload hint for this image.
        if !has_existing_preload(html, &hero_src) {
            let hero_preload = preload::build_hero_preload(
                &hero_src,
                html,
                dist_dir,
                &config.hero_image_sizes,
            );
            hints.push(ResourceHint::HeroPreload(hero_preload));
        }
    }

    // 2. Navigation prefetch.
    if config.prefetch_links {
        let urls = prefetch::collect_prefetch_urls(
            html,
            current_url,
            fragment_dir,
            fragments_enabled,
            config.max_prefetch,
            &config.exclude_prefetch,
        );
        for url in urls {
            hints.push(ResourceHint::NavigationPrefetch {
                href: url,
                is_document: !fragments_enabled,
            });
        }
    }

    // 3. Generate and inject HTML.
    if hints.is_empty() {
        return html.to_string();
    }

    let hint_html = hints_to_html(&hints);

    match inject_into_head(html, &hint_html) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("Failed to inject resource hints: {}", e);
            html.to_string()
        }
    }
}

/// Check if the HTML already has a `<link rel="preload">` for the given href.
fn has_existing_preload(html: &str, href: &str) -> bool {
    let found: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_clone = found.clone();
    let href_owned = href.to_string();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("link[rel='preload']", move |el| {
                    if let Some(existing_href) = el.get_attribute("href") {
                        if existing_href == href_owned {
                            *found_clone.borrow_mut() = true;
                        }
                    }
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    *found.borrow()
}

/// Serialize a list of ResourceHint values into HTML `<link>` tag strings.
fn hints_to_html(hints: &[ResourceHint]) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match hint {
            ResourceHint::HeroPreload(p) => {
                let _ = write!(out, r#"<link rel="preload" as="image" href="{}""#, p.href);
                if let Some(ref srcset) = p.imagesrcset {
                    let _ = write!(out, r#" imagesrcset="{}""#, srcset);
                }
                if let Some(ref sizes) = p.imagesizes {
                    let _ = write!(out, r#" imagesizes="{}""#, sizes);
                }
                let _ = write!(out, r#" type="{}">"#, p.mime_type);
            }
            ResourceHint::NavigationPrefetch { href, is_document } => {
                let _ = write!(out, r#"<link rel="prefetch" href="{}""#, href);
                if *is_document {
                    out.push_str(r#" as="document""#);
                }
                out.push('>');
            }
        }
    }

    out
}

/// Inject hint `<link>` tags into the HTML `<head>` as early children.
///
/// Uses lol_html to find the opening `<head>` tag and prepend the
/// hint tags. Returns the original HTML if no `<head>` is found.
fn inject_into_head(html: &str, hint_html: &str) -> Result<String, String> {
    let hint_html_owned = hint_html.to_string();

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("head", move |el| {
                    el.prepend(
                        &format!("\n{}\n", hint_html_owned),
                        lol_html::html_content::ContentType::Html,
                    );
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| format!("lol_html rewrite error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HintsConfig {
        HintsConfig {
            enabled: true,
            auto_detect_hero: true,
            prefetch_links: true,
            max_prefetch: 5,
            hero_image_sizes: "100vw".to_string(),
            exclude_prefetch: Vec::new(),
        }
    }

    #[test]
    fn test_inject_hints_disabled() {
        let html = r#"<html><head><title>T</title></head><body><img src="/hero.jpg" alt="H"></body></html>"#;
        let config = HintsConfig { enabled: false, ..test_config() };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", true,
        );
        assert_eq!(result, html);
    }

    #[test]
    fn test_inject_hero_from_frontmatter() {
        let html = r#"<html><head><title>T</title></head><body><p>No img</p></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html,
            &config,
            dir.path(),
            Some("/assets/hero.jpg"),
            "/index.html",
            "_fragments",
            true,
        );
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains(r#"href="/assets/hero.jpg""#));
        assert!(result.contains(r#"as="image""#));
    }

    #[test]
    fn test_inject_hero_auto_detect() {
        let html = r#"<html><head><title>T</title></head><body><img src="/assets/banner.jpg" alt="Banner"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", true,
        );
        assert!(result.contains(r#"href="/assets/banner.jpg""#));
        assert!(result.contains(r#"rel="preload""#));
    }

    #[test]
    fn test_auto_detect_skips_lazy() {
        let html = r#"<html><head></head><body><img src="/lazy.jpg" loading="lazy" alt="L"><img src="/hero.jpg" alt="H"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", true,
        );
        assert!(result.contains(r#"href="/hero.jpg""#));
        assert!(!result.contains(r#"href="/lazy.jpg""#));
    }

    #[test]
    fn test_inject_prefetch_links() {
        let html = r#"<html><head></head><body>
            <a href="/about.html" hx-get="/_fragments/about.html" hx-target="#content">About</a>
            <a href="/blog.html" hx-get="/_fragments/blog.html" hx-target="#content">Blog</a>
        </body></html>"#;
        let config = HintsConfig { auto_detect_hero: false, ..test_config() };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", true,
        );
        assert!(result.contains(r#"rel="prefetch" href="/_fragments/about.html""#));
        assert!(result.contains(r#"rel="prefetch" href="/_fragments/blog.html""#));
    }

    #[test]
    fn test_inject_combined() {
        let html = r#"<html><head></head><body>
            <img src="/hero.jpg" alt="Hero">
            <a hx-get="/_fragments/about.html">About</a>
        </body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", true,
        );
        // Both preload and prefetch should be present.
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains(r#"rel="prefetch""#));
    }

    #[test]
    fn test_no_head_element() {
        let html = r#"<div>No head element here</div>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        // Should not crash; hint_html has nowhere to go, but inject_into_head
        // will still return the HTML (lol_html won't match the `head` selector).
        let result = inject_resource_hints(
            html, &config, dir.path(), Some("/hero.jpg"), "/index.html", "_fragments", true,
        );
        // The function should return something (possibly unchanged if injection
        // had no effect, or with hints if lol_html just prepends to nothing).
        assert!(!result.is_empty());
    }

    #[test]
    fn test_skip_existing_preload() {
        let html = r#"<html><head>
            <link rel="preload" as="image" href="/hero.jpg">
        </head><body><img src="/hero.jpg" alt="Hero"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), Some("/hero.jpg"), "/index.html", "_fragments", true,
        );
        // Should not duplicate the preload.
        let preload_count = result.matches(r#"rel="preload""#).count();
        assert_eq!(preload_count, 1);
    }

    #[test]
    fn test_hints_injected_early_in_head() {
        let html = r#"<html><head><meta charset="UTF-8"><title>T</title></head><body><img src="/hero.jpg" alt="H"></body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), Some("/hero.jpg"), "/index.html", "_fragments", true,
        );
        // Hints should appear before the <meta> tag.
        let head_pos = result.find("<head>").unwrap();
        let preload_pos = result.find(r#"rel="preload""#).unwrap();
        let meta_pos = result.find(r#"<meta"#).unwrap();
        assert!(preload_pos > head_pos);
        assert!(preload_pos < meta_pos);
    }

    #[test]
    fn test_prefetch_is_document_when_no_fragments() {
        let html = r#"<html><head></head><body>
            <a href="/about.html">About</a>
        </body></html>"#;
        let config = HintsConfig { auto_detect_hero: false, ..test_config() };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", false,
        );
        assert!(result.contains(r#"as="document""#));
    }

    #[test]
    fn test_prefetch_no_as_document_with_fragments() {
        let html = r#"<html><head></head><body>
            <a hx-get="/_fragments/about.html">About</a>
        </body></html>"#;
        let config = HintsConfig { auto_detect_hero: false, ..test_config() };
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", true,
        );
        assert!(!result.contains(r#"as="document""#));
    }

    #[test]
    fn test_hints_to_html_preload() {
        let hints = vec![ResourceHint::HeroPreload(preload::HeroPreload {
            href: "/hero.jpg".to_string(),
            mime_type: "image/jpeg".to_string(),
            imagesrcset: None,
            imagesizes: None,
        })];
        let html = hints_to_html(&hints);
        assert_eq!(html, r#"<link rel="preload" as="image" href="/hero.jpg" type="image/jpeg">"#);
    }

    #[test]
    fn test_hints_to_html_preload_responsive() {
        let hints = vec![ResourceHint::HeroPreload(preload::HeroPreload {
            href: "/hero.jpg".to_string(),
            mime_type: "image/avif".to_string(),
            imagesrcset: Some("/hero-480w.avif 480w, /hero-768w.avif 768w".to_string()),
            imagesizes: Some("100vw".to_string()),
        })];
        let html = hints_to_html(&hints);
        assert!(html.contains(r#"imagesrcset="/hero-480w.avif 480w, /hero-768w.avif 768w""#));
        assert!(html.contains(r#"imagesizes="100vw""#));
        assert!(html.contains(r#"type="image/avif""#));
    }

    #[test]
    fn test_hints_to_html_prefetch() {
        let hints = vec![ResourceHint::NavigationPrefetch {
            href: "/_fragments/about.html".to_string(),
            is_document: false,
        }];
        let html = hints_to_html(&hints);
        assert_eq!(html, r#"<link rel="prefetch" href="/_fragments/about.html">"#);
    }

    #[test]
    fn test_hints_to_html_prefetch_document() {
        let hints = vec![ResourceHint::NavigationPrefetch {
            href: "/about.html".to_string(),
            is_document: true,
        }];
        let html = hints_to_html(&hints);
        assert_eq!(html, r#"<link rel="prefetch" href="/about.html" as="document">"#);
    }

    #[test]
    fn test_inject_into_head() {
        let html = r#"<html><head><title>T</title></head></html>"#;
        let hint = r#"<link rel="preload" as="image" href="/hero.jpg" type="image/jpeg">"#;
        let result = inject_into_head(html, hint).unwrap();
        assert!(result.contains(hint));
        // Hint should be before <title>
        let hint_pos = result.find(r#"rel="preload""#).unwrap();
        let title_pos = result.find("<title>").unwrap();
        assert!(hint_pos < title_pos);
    }

    #[test]
    fn test_has_existing_preload() {
        let html = r#"<head><link rel="preload" as="image" href="/hero.jpg"></head>"#;
        assert!(has_existing_preload(html, "/hero.jpg"));
        assert!(!has_existing_preload(html, "/other.jpg"));
    }

    #[test]
    fn test_hero_from_picture_element() {
        let html = r#"<html><head></head><body>
            <picture>
                <source srcset="/hero-480w.avif 480w, /hero-768w.avif 768w" type="image/avif">
                <img src="/hero.jpg" alt="Hero">
            </picture>
        </body></html>"#;
        let config = test_config();
        let dir = tempfile::tempdir().unwrap();
        let result = inject_resource_hints(
            html, &config, dir.path(), None, "/index.html", "_fragments", true,
        );
        assert!(result.contains(r#"imagesrcset="#));
        assert!(result.contains(r#"type="image/avif""#));
    }
}
```

- [ ] **Step 2: Run tests to verify orchestration module compiles and passes**

Run: `cargo test --lib build::hints -- --nocapture`

Expected: All tests across `mod.rs`, `preload.rs`, and `prefetch.rs` pass.

- [ ] **Step 3: Commit**

```bash
git add src/build/hints/mod.rs
git commit -m "feat(hints): add orchestration, serialization, and injection

Wire together hero preload and navigation prefetch into the public
inject_resource_hints function. Add ResourceHint enum, hints_to_html
serializer, inject_into_head via lol_html, and duplicate preload
detection."
```

---

## Task 6: Integrate hints into the build pipeline

**Depends on:** Tasks 1, 2, 5 (needs config, frontmatter, and the complete hints module)

**Files:**
- Modify: `src/build/render.rs`

- [ ] **Step 1: Add the hints import to render.rs**

In `src/build/render.rs`, add an import for the hints module. After the existing `use super::critical_css;` line (around line 21), add:

```rust
use super::hints;
```

- [ ] **Step 2: Add hints step to `render_static_page`**

In `render_static_page`, insert the hints step after the critical CSS block (after the `};` that closes the critical CSS `if` around line 325) and before the minify step (before the `let full_html = if config.build.minify {` line):

```rust
    // 4d. Preload/prefetch hints (after critical CSS, before minify).
    // NOTE: This shifts the existing "4d. Minify" comment to "4e.".
    let full_html = if config.build.hints.enabled {
        hints::inject_resource_hints(
            &full_html,
            &config.build.hints,
            dist_dir,
            page.frontmatter.hero_image.as_deref(),
            &url_path,
            &config.build.fragment_dir,
            config.build.fragments,
        )
    } else {
        full_html
    };
```

- [ ] **Step 3: Add hints step to `render_dynamic_page`**

In `render_dynamic_page`, insert the same hints block after the critical CSS block (after the `};` around line 574) and before the minify step:

```rust
        // Preload/prefetch hints (after critical CSS, before minify).
        let full_html = if config.build.hints.enabled {
            hints::inject_resource_hints(
                &full_html,
                &config.build.hints,
                dist_dir,
                page.frontmatter.hero_image.as_deref(),
                &url_path,
                &config.build.fragment_dir,
                config.build.fragments,
            )
        } else {
            full_html
        };
```

- [ ] **Step 4: Add logging for hints**

In the `build()` function, after the critical CSS logging block (after line 108 `tracing::info!("Critical CSS inlining enabled.");`), add:

```rust
    if config.build.hints.enabled {
        tracing::info!("Resource hints enabled (preload + prefetch).");
    }
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build`

Expected: Successful compilation with no errors. There may be unused import warnings during development which will be resolved as the code is used.

- [ ] **Step 6: Commit**

```bash
git add src/build/render.rs
git commit -m "feat(hints): integrate resource hints into build pipeline

Insert inject_resource_hints call after critical CSS inlining and before
HTML minification in both render_static_page and render_dynamic_page.
Add logging for hints enabled state."
```

---

## Task 7: Write feature documentation

**Depends on:** Tasks 1-6 (feature must be complete)

**Files:**
- Create: `docs/preload_prefetch.md`

- [ ] **Step 1: Write feature documentation**

Create `docs/preload_prefetch.md`:

```markdown
# Preload / Prefetch Resource Hints

## Overview

Eigen automatically injects `<link>` resource hints into `<head>` to improve page load performance:

- **`<link rel="preload">`** for hero images -- tells the browser to start fetching the LCP image immediately, before the HTML parser reaches the `<img>` tag. Reduces Largest Contentful Paint.
- **`<link rel="prefetch">`** for navigation links -- tells the browser to fetch HTMX fragment files at idle time, making partial-page transitions feel instant.

Resource hints are enabled by default. They are purely additive (they add `<link>` tags but never modify or remove existing content) and cannot break output.

## Configuration

### site.toml

```toml
[build.hints]
# Master switch. Default: true.
enabled = true

# Auto-detect hero image from first qualifying <img>. Default: true.
auto_detect_hero = true

# Generate prefetch hints for navigation links. Default: true.
prefetch_links = true

# Maximum prefetch hints per page. Default: 5.
max_prefetch = 5

# Fallback imagesizes for hero preload. Default: "100vw".
hero_image_sizes = "100vw"

# Glob patterns to exclude from prefetching.
exclude_prefetch = ["**/archive/**"]
```

### Frontmatter

Designate a specific hero image in template frontmatter:

```yaml
---
hero_image: /assets/hero-banner.jpg
data:
  nav:
    file: "nav.yaml"
---
```

When `hero_image` is set, it takes priority over auto-detection.

## Hero Image Preload

### Resolution order

1. Frontmatter `hero_image` field (highest priority).
2. Auto-detected first `<img>` without `loading="lazy"` (if `auto_detect_hero` is enabled).
3. No preload hint generated.

### Responsive variants

When image optimization is active and the hero image has a `<picture>` element with `<source srcset>`, the preload uses `imagesrcset` and `imagesizes` to let the browser preload the correct size:

```html
<link rel="preload" as="image" href="/assets/hero.jpg"
      imagesrcset="/assets/hero-480w-abc.avif 480w, /assets/hero-768w-abc.avif 768w"
      imagesizes="100vw"
      type="image/avif">
```

The preload targets the best format (AVIF if available). Browsers that don't support AVIF ignore the hint safely.

### CSS background images

If the hero is a CSS `background-image`, auto-detection won't find it. Use the frontmatter `hero_image` field. The module will scan `dist/` for variant files as a fallback.

## Navigation Prefetch

### How it works

- When fragments are enabled: scans `hx-get` attributes to prefetch fragment files.
- When fragments are disabled: scans `<a href>` attributes to prefetch full pages (adds `as="document"`).

### Filtering

- External URLs (http/https) are excluded.
- Self-referencing links (current page) are excluded.
- Anchor-only links (#...) are excluded.
- Links with `target="_blank"` are excluded (href mode only).
- URLs matching `exclude_prefetch` glob patterns are excluded.
- Duplicate URLs are deduplicated (first occurrence wins).
- At most `max_prefetch` URLs are included (document order).

## Pipeline position

```
render template
  -> strip fragment markers
  -> localize assets
  -> optimize images (img -> picture)
  -> rewrite CSS background images
  -> plugin post_render_html
  -> critical CSS inlining
  -> preload/prefetch hints  <-- THIS FEATURE
  -> minify HTML
  -> write to disk
```

Fragments do NOT get resource hints (they lack a `<head>` element).
Dev server builds also skip resource hints.

## Error handling

All errors are non-fatal. If anything goes wrong (missing `<head>`, IO errors, parse failures), the original HTML is returned unchanged with a warning logged.
```

- [ ] **Step 2: Commit**

```bash
git add docs/preload_prefetch.md
git commit -m "docs: add preload/prefetch resource hints documentation

Document configuration, hero image resolution, responsive variants,
navigation prefetch behavior, filtering rules, and pipeline position."
```

---

## Final Verification

After all tasks are complete, run the following verification steps:

- [ ] **Full test suite**

Run: `cargo test -- --nocapture`

Expected: All tests pass (existing and new).

- [ ] **Compilation check**

Run: `cargo build`

Expected: Clean build with no warnings.

- [ ] **Verify feature structure**

```bash
ls -la src/build/hints/
```

Expected output shows three files: `mod.rs`, `preload.rs`, `prefetch.rs`.

- [ ] **Verify config integration**

Run: `cargo test --lib config::tests::test_hints -- --nocapture`

Expected: All 3 hints config tests pass.

- [ ] **Verify frontmatter integration**

Run: `cargo test --lib frontmatter::tests::test_parse_hero -- --nocapture`

Expected: hero_image parsing test passes.

- [ ] **Review test count**

Run: `cargo test --lib build::hints 2>&1 | tail -1`

Expected: ~51 tests across the hints module (16 prefetch + 17 preload + 18 mod).

- [ ] **Manual smoke test (optional)**

If an example site is available, run `cargo run -- build` against it and inspect the generated HTML in `dist/` for `<link rel="preload">` and `<link rel="prefetch">` tags in `<head>`.
