# Open Graph / Twitter Card Meta Tags Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-inject Open Graph and Twitter Card meta tags into `<head>` during the build pipeline, using per-page frontmatter `[seo]` fields with site-level defaults from `[site.seo]` in `site.toml`, requiring zero template changes from users.

**Architecture:** A new `build::seo` module (single file `src/build/seo.rs`) slots into the existing per-page build pipeline after `build::hints` (preload/prefetch) and before `build::minify`. It resolves SEO field values from a three-layer cascade (auto-derived defaults < site config < frontmatter), detects existing OG/Twitter meta tags in the rendered HTML to avoid duplicates, generates missing meta tag HTML, and injects it into `<head>` using `lol_html::rewrite_str`. For dynamic pages, frontmatter SEO fields containing minijinja template expressions (e.g. `{{ post.title }}`) are resolved per-item using `minijinja::Environment::render_str` before injection. The feature is always-on with no `enabled` flag -- SEO tags are purely additive and cost nothing.

**Tech Stack:** Rust, lol_html (`rewrite_str` API for HTML scan + inject, consistent with existing codebase), minijinja (`render_str` for template expression resolution in dynamic page SEO fields)

**IMPORTANT -- lol_html API:** The codebase uses `lol_html::rewrite_str` with `lol_html::RewriteStrSettings` everywhere (see `src/assets/html_rewrite.rs`, `src/build/critical_css/rewrite.rs`, `src/build/hints/mod.rs`). It does NOT use the streaming `HtmlRewriter` / `Settings` API. All code in this plan uses `rewrite_str` / `RewriteStrSettings` to be consistent. Closures that capture mutable state use `Rc<RefCell<>>` (matching the pattern in `critical_css/rewrite.rs` and `hints/mod.rs`). CSS attribute selectors in `lol_html::element!()` use single quotes (e.g., `meta[property='og:title']`), not escaped double quotes, matching existing codebase style.

**Design spec:** `docs/superpowers/specs/2026-03-17-og-twitter-meta-design.md`

---

## File Structure

### New files to create

| File | Responsibility |
|------|---------------|
| `src/build/seo.rs` | Public API (`inject_seo_tags`, `resolve_seo_expressions`), SEO field resolution (`resolve_seo`), existing tag detection (`detect_existing_tags`, `has_canonical_link`), meta tag HTML generation (`push_meta`, `generate_meta_html`), HTML injection (`inject_into_head`), attribute escaping (`escape_attr`), URL resolution (`make_absolute_url`), `ResolvedSeo` struct |
| `docs/og_twitter_meta.md` | Feature documentation for future reference |

### Existing files to modify

| File | Change |
|------|--------|
| `src/config/mod.rs` | Add `SiteSeoConfig` struct with `title`, `description`, `image`, `og_type`, `twitter_site`, `twitter_card` fields. Add `seo: SiteSeoConfig` field to `SiteMeta`. Add `default_og_type` and `default_twitter_card` helper functions. Manual `Default` impl. Add config parsing tests. |
| `src/frontmatter/mod.rs` | Add `SeoMeta` struct with `title`, `description`, `image`, `og_type`, `twitter_card`, `canonical_url` fields. Add `seo: SeoMeta` to `Frontmatter` and `RawFrontmatter`. Update `parse_frontmatter` and `Default` impl. Add frontmatter parsing tests. |
| `src/build/mod.rs` | Add `pub mod seo;` declaration |
| `src/build/render.rs` | Insert `resolve_seo_expressions` call after context build, insert `inject_seo_tags` call after hints and before minify, in both `render_static_page` and `render_dynamic_page`. Add `use super::seo;` import. |
| `src/build/context.rs` | Update `test_config()` helper to include `seo: SiteSeoConfig::default()` in `SiteMeta` struct literal (test code). |
| `src/build/sitemap.rs` | Update `test_config()` helper to include `seo: SiteSeoConfig::default()` in `SiteMeta` struct literal (test code). |
| `src/discovery/mod.rs` | Update `test_config()` helper to include `seo: SiteSeoConfig::default()` in `SiteMeta` struct literal (test code). |
| `src/template/environment.rs` | Update test helper to include `seo: SiteSeoConfig::default()` in `SiteMeta` struct literal (test code). |
| `src/template/functions.rs` | Update `test_config()` and `test_config_no_fragments()` helpers to include `seo: SiteSeoConfig::default()` in `SiteMeta` struct literal (test code). |
| `src/template/filters.rs` | Update test helper to include `seo: SiteSeoConfig::default()` in `SiteMeta` struct literal (test code). |

---

## Task 1: Add `SiteSeoConfig` to site configuration

**Depends on:** Nothing (starting point)

**Files:**
- Modify: `src/config/mod.rs`
- Modify (test helpers only): `src/build/context.rs`, `src/build/sitemap.rs`, `src/discovery/mod.rs`, `src/template/environment.rs`, `src/template/functions.rs`, `src/template/filters.rs`

- [ ] **Step 1: Add `SiteSeoConfig` struct and default helper functions**

In `src/config/mod.rs`, add the following after the `BundlingConfig` struct and its `Default` impl (after the `impl Default for BundlingConfig` block, around line 380):

```rust
/// Site-level SEO defaults for Open Graph and Twitter Card meta tags.
///
/// Located under `[site.seo]` in site.toml. These provide fallback
/// values for pages that do not set explicit `[seo]` in frontmatter.
#[derive(Debug, Clone, Deserialize)]
pub struct SiteSeoConfig {
    /// Default page title for `og:title` / `twitter:title`.
    /// Falls back to `site.name` if not set.
    pub title: Option<String>,

    /// Default meta description for pages without one.
    pub description: Option<String>,

    /// Default share image URL (absolute or site-relative path).
    /// Used when a page has no `seo.image` in frontmatter.
    pub image: Option<String>,

    /// Default `og:type`. Default: "website".
    #[serde(default = "default_og_type")]
    pub og_type: String,

    /// Twitter/X @handle for `twitter:site`.
    /// Example: "@mysite"
    pub twitter_site: Option<String>,

    /// Default `twitter:card` type when an image IS available.
    /// Default: "summary_large_image".
    ///
    /// When no image is available (neither from frontmatter nor from
    /// this config), `twitter:card` is forced to `"summary"` regardless
    /// of this setting.
    #[serde(default = "default_twitter_card")]
    pub twitter_card: String,
}

fn default_og_type() -> String {
    "website".to_string()
}

fn default_twitter_card() -> String {
    "summary_large_image".to_string()
}

impl Default for SiteSeoConfig {
    fn default() -> Self {
        Self {
            title: None,
            description: None,
            image: None,
            og_type: default_og_type(),
            twitter_site: None,
            twitter_card: default_twitter_card(),
        }
    }
}
```

> **NOTE:** `SiteSeoConfig` must NOT use `#[derive(Default)]` because the
> derived `Default` would set `og_type` and `twitter_card` to empty
> strings. The `#[serde(default = "...")]` attributes only apply during
> TOML deserialization, not during `Default::default()`. A manual
> `Default` impl is required, matching the pattern used by
> `CriticalCssConfig`, `HintsConfig`, `ContentHashConfig`, and
> `BundlingConfig` in this file.

- [ ] **Step 2: Add `seo` field to `SiteMeta`**

In `src/config/mod.rs`, modify the `SiteMeta` struct (around line 26) to add the `seo` field:

```rust
/// Metadata about the site itself.
#[derive(Debug, Clone, Deserialize)]
pub struct SiteMeta {
    pub name: String,
    pub base_url: String,
    /// Site-level SEO defaults for Open Graph and Twitter Card tags.
    #[serde(default)]
    pub seo: SiteSeoConfig,
}
```

- [ ] **Step 3: Update all `SiteMeta` struct literals in test helpers**

Adding `seo: SiteSeoConfig` to `SiteMeta` breaks every test helper that constructs `SiteMeta { name: ..., base_url: ... }` without the new field (Rust requires all fields in a struct literal). Add `seo: SiteSeoConfig::default()` to each of these locations:

| File | Line(s) | What to add |
|------|---------|-------------|
| `src/build/context.rs` | ~92 | `seo: SiteSeoConfig::default(),` after `base_url` in `test_config()` |
| `src/build/sitemap.rs` | ~81 | `seo: SiteSeoConfig::default(),` after `base_url` in `test_config()` |
| `src/discovery/mod.rs` | ~220 | `seo: SiteSeoConfig::default(),` after `base_url` in `test_config()` |
| `src/template/environment.rs` | ~177 | `seo: SiteSeoConfig::default(),` after `base_url` |
| `src/template/functions.rs` | ~119, ~137 | `seo: SiteSeoConfig::default(),` after `base_url` in both `test_config()` and `test_config_no_fragments()` |
| `src/template/filters.rs` | ~243 | `seo: SiteSeoConfig::default(),` after `base_url` |

Each file's test module needs to import `SiteSeoConfig` alongside its existing `SiteMeta` import. For example, change `use crate::config::{BuildConfig, SiteMeta};` to `use crate::config::{BuildConfig, SiteMeta, SiteSeoConfig};`.

> **Why these need updating:** Unlike `Frontmatter` (which has a manual `Default` impl and test code uses `..Default::default()`), `SiteMeta` does not implement `Default` and all test helpers use exhaustive struct literals. Adding a new field without updating them causes a compilation error.

- [ ] **Step 4: Write tests for `SiteSeoConfig` parsing**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/config/mod.rs`:

```rust
    // --- Site SEO config tests ---

    #[test]
    fn test_site_seo_config_defaults() {
        let toml_str = r#"
[site]
name = "SEO Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.site.seo.title.is_none());
        assert!(config.site.seo.description.is_none());
        assert!(config.site.seo.image.is_none());
        assert_eq!(config.site.seo.og_type, "website");
        assert!(config.site.seo.twitter_site.is_none());
        assert_eq!(config.site.seo.twitter_card, "summary_large_image");
    }

    #[test]
    fn test_site_seo_config_custom() {
        let toml_str = r#"
[site]
name = "SEO Custom"
base_url = "https://example.com"

[site.seo]
title = "My Custom Title"
description = "A description of the site"
image = "/assets/default-share.jpg"
og_type = "article"
twitter_site = "@mysite"
twitter_card = "summary"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.site.seo.title.as_deref(), Some("My Custom Title"));
        assert_eq!(config.site.seo.description.as_deref(), Some("A description of the site"));
        assert_eq!(config.site.seo.image.as_deref(), Some("/assets/default-share.jpg"));
        assert_eq!(config.site.seo.og_type, "article");
        assert_eq!(config.site.seo.twitter_site.as_deref(), Some("@mysite"));
        assert_eq!(config.site.seo.twitter_card, "summary");
    }

    #[test]
    fn test_site_seo_config_partial() {
        let toml_str = r#"
[site]
name = "SEO Partial"
base_url = "https://example.com"

[site.seo]
description = "Only description set"
twitter_site = "@partial"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.site.seo.title.is_none());
        assert_eq!(config.site.seo.description.as_deref(), Some("Only description set"));
        assert!(config.site.seo.image.is_none());
        assert_eq!(config.site.seo.og_type, "website");
        assert_eq!(config.site.seo.twitter_site.as_deref(), Some("@partial"));
        assert_eq!(config.site.seo.twitter_card, "summary_large_image");
    }
```

- [ ] **Step 5: Run tests to verify config changes compile and pass**

Run: `cargo test -- --nocapture`

Expected: All existing tests pass (including tests in `context.rs`, `sitemap.rs`, `discovery/mod.rs`, `template/environment.rs`, `template/functions.rs`, `template/filters.rs` which now compile with the updated `SiteMeta` struct), plus the 3 new SEO config tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/config/mod.rs src/build/context.rs src/build/sitemap.rs src/discovery/mod.rs src/template/environment.rs src/template/functions.rs src/template/filters.rs
git commit -m "feat(seo): add SiteSeoConfig and wire into SiteMeta

Add SiteSeoConfig struct with title, description, image, og_type,
twitter_site, and twitter_card fields. Wire into SiteMeta under
[site.seo] in site.toml. All fields are optional with sensible
defaults (og_type defaults to 'website', twitter_card defaults to
'summary_large_image'). No enabled flag -- SEO tags are always
beneficial and cost nothing.

Update all test helpers that construct SiteMeta struct literals to
include the new seo field."
```

---

## Task 2: Add `SeoMeta` to frontmatter

**Depends on:** Nothing (independent of Task 1)

**Files:**
- Modify: `src/frontmatter/mod.rs`

- [ ] **Step 1: Add `SeoMeta` struct**

In `src/frontmatter/mod.rs`, add the following after the `DataQuery` struct (after line 62):

```rust
/// Per-page SEO metadata for Open Graph and Twitter Card tags.
///
/// All fields are optional. When absent, site-level defaults from
/// `[site.seo]` in site.toml are used.
///
/// For dynamic pages, field values may contain minijinja template
/// expressions (e.g. `{{ post.title }}`) which are resolved per-item
/// during rendering.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeoMeta {
    /// Page title for `og:title` and `twitter:title`.
    /// Falls back to `site.seo.title`, then `site.name`.
    pub title: Option<String>,

    /// Page description for `og:description` and `twitter:description`.
    /// Falls back to `site.seo.description`.
    pub description: Option<String>,

    /// Share image URL for `og:image` and `twitter:image`.
    /// Can be a site-relative path (e.g. "/assets/hero.jpg") or
    /// absolute URL. Relative paths are resolved to absolute URLs
    /// using `site.base_url` during injection.
    /// Falls back to `site.seo.image`.
    pub image: Option<String>,

    /// Open Graph type for `og:type`.
    /// Falls back to `site.seo.og_type`, then "website".
    pub og_type: Option<String>,

    /// Twitter card type for `twitter:card`.
    /// Falls back to `site.seo.twitter_card`, then
    /// "summary_large_image". Forced to "summary" when no image
    /// is available at any level.
    pub twitter_card: Option<String>,

    /// Override the canonical URL. By default, this is auto-generated
    /// from `site.base_url` + page URL path.
    pub canonical_url: Option<String>,
}
```

- [ ] **Step 2: Add `seo` field to `Frontmatter`**

In `src/frontmatter/mod.rs`, add a new field to the `Frontmatter` struct (after `hero_image`, around line 28):

```rust
    /// SEO metadata for Open Graph and Twitter Card tags.
    pub seo: SeoMeta,
```

- [ ] **Step 3: Add `seo` field to `RawFrontmatter`**

In `src/frontmatter/mod.rs`, add a new field to the `RawFrontmatter` struct (after `hero_image`, around line 77):

```rust
    #[serde(default)]
    seo: SeoMeta,
```

- [ ] **Step 4: Update `parse_frontmatter` to map the field**

In `src/frontmatter/mod.rs`, update the `Ok(Frontmatter { ... })` block in `parse_frontmatter` (around line 144) to include:

```rust
        seo: raw.seo,
```

(Add after the `hero_image: raw.hero_image,` line.)

- [ ] **Step 5: Update `Frontmatter::default()`**

In the `Default` impl for `Frontmatter` (around line 30), add after `hero_image: None,`:

```rust
            seo: SeoMeta::default(),
```

- [ ] **Step 6: Write tests for `SeoMeta` frontmatter parsing**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/frontmatter/mod.rs`:

```rust
    // --- SEO frontmatter tests ---

    #[test]
    fn test_parse_seo_frontmatter_full() {
        let yaml = concat!(
            "seo:\n",
            "  title: \"About Us\"\n",
            "  description: \"Learn about our team\"\n",
            "  image: /assets/about-hero.jpg\n",
            "  og_type: website\n",
            "  twitter_card: summary_large_image\n",
            "  canonical_url: https://example.com/about\n",
        );
        let fm = parse_frontmatter(yaml, "about.html").unwrap();
        assert_eq!(fm.seo.title.as_deref(), Some("About Us"));
        assert_eq!(fm.seo.description.as_deref(), Some("Learn about our team"));
        assert_eq!(fm.seo.image.as_deref(), Some("/assets/about-hero.jpg"));
        assert_eq!(fm.seo.og_type.as_deref(), Some("website"));
        assert_eq!(fm.seo.twitter_card.as_deref(), Some("summary_large_image"));
        assert_eq!(fm.seo.canonical_url.as_deref(), Some("https://example.com/about"));
    }

    #[test]
    fn test_parse_seo_frontmatter_partial() {
        let yaml = concat!(
            "seo:\n",
            "  title: \"My Page\"\n",
            "  description: \"A description\"\n",
        );
        let fm = parse_frontmatter(yaml, "page.html").unwrap();
        assert_eq!(fm.seo.title.as_deref(), Some("My Page"));
        assert_eq!(fm.seo.description.as_deref(), Some("A description"));
        assert!(fm.seo.image.is_none());
        assert!(fm.seo.og_type.is_none());
        assert!(fm.seo.twitter_card.is_none());
        assert!(fm.seo.canonical_url.is_none());
    }

    #[test]
    fn test_parse_seo_frontmatter_absent() {
        let yaml = "data:\n  nav:\n    file: \"nav.yaml\"\n";
        let fm = parse_frontmatter(yaml, "index.html").unwrap();
        assert!(fm.seo.title.is_none());
        assert!(fm.seo.description.is_none());
        assert!(fm.seo.image.is_none());
        assert!(fm.seo.og_type.is_none());
        assert!(fm.seo.twitter_card.is_none());
        assert!(fm.seo.canonical_url.is_none());
    }

    #[test]
    fn test_parse_seo_with_template_expressions() {
        let yaml = concat!(
            "seo:\n",
            "  title: \"{{ post.title }} | My Blog\"\n",
            "  description: \"{{ post.excerpt }}\"\n",
            "  image: \"{{ post.cover_image }}\"\n",
        );
        let fm = parse_frontmatter(yaml, "post.html").unwrap();
        // Expressions are stored as literal strings, not evaluated at parse time.
        assert_eq!(fm.seo.title.as_deref(), Some("{{ post.title }} | My Blog"));
        assert_eq!(fm.seo.description.as_deref(), Some("{{ post.excerpt }}"));
        assert_eq!(fm.seo.image.as_deref(), Some("{{ post.cover_image }}"));
    }

    #[test]
    fn test_default_seo_is_empty() {
        let fm = Frontmatter::default();
        assert!(fm.seo.title.is_none());
        assert!(fm.seo.description.is_none());
        assert!(fm.seo.image.is_none());
        assert!(fm.seo.og_type.is_none());
        assert!(fm.seo.twitter_card.is_none());
        assert!(fm.seo.canonical_url.is_none());
    }
```

- [ ] **Step 7: Run tests to verify frontmatter changes compile and pass**

Run: `cargo test --lib frontmatter -- --nocapture`

Expected: All existing frontmatter tests pass, plus the 5 new SEO tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/frontmatter/mod.rs
git commit -m "feat(seo): add SeoMeta struct to frontmatter

Add SeoMeta struct with title, description, image, og_type,
twitter_card, and canonical_url fields. Wire into Frontmatter and
RawFrontmatter. All fields are optional. For dynamic pages, values
may contain minijinja template expressions that are resolved later
in the build pipeline."
```

---

## Task 3: Create `build::seo` module -- core resolution and generation logic

**Depends on:** Task 1 (SiteSeoConfig), Task 2 (SeoMeta)

**Files:**
- Create: `src/build/seo.rs`
- Modify: `src/build/mod.rs`

This is the main module. It contains the internal types, resolution logic, existing tag detection, meta tag HTML generation, HTML injection, and public API. We build and test incrementally: first the internal helpers, then the public orchestration functions.

- [ ] **Step 1: Create `src/build/seo.rs` with module declaration and imports**

Create `src/build/seo.rs` with the module-level doc comment, imports, and the `ResolvedSeo` struct:

```rust
//! Open Graph / Twitter Card meta tag injection.
//!
//! Auto-injects `<meta property="og:*">`, `<meta name="twitter:*">`, and
//! `<link rel="canonical">` tags into `<head>` during the build pipeline.
//!
//! Two public functions:
//! - `inject_seo_tags`: the pipeline step (after hints, before minify).
//! - `resolve_seo_expressions`: template expression evaluation for dynamic
//!   page SEO fields (called earlier in the render flow).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::config::SiteMeta;
use crate::frontmatter::SeoMeta;

/// Fully resolved SEO metadata ready for injection into HTML.
///
/// All template expressions have been evaluated. All fallbacks have
/// been applied. URLs are absolute.
struct ResolvedSeo {
    /// Page title. Never empty -- falls back chain:
    /// frontmatter.seo.title > site.seo.title > site.name.
    title: String,
    /// Meta description. None if no description is available at any level.
    description: Option<String>,
    /// Absolute URL to the share image. None if no image is available.
    image: Option<String>,
    /// Open Graph type (e.g. "website", "article").
    og_type: String,
    /// Twitter card type. Forced to "summary" when image is None.
    twitter_card: String,
    /// Twitter @handle for the site. None if not configured.
    twitter_site: Option<String>,
    /// Canonical URL (absolute). Always present.
    canonical_url: String,
    /// The site name for `og:site_name`. Always `site.name`.
    site_name: String,
}
```

- [ ] **Step 2: Add `escape_attr` function**

Append to `src/build/seo.rs`:

```rust
/// Escape characters that are unsafe in HTML attribute values.
///
/// Only the four characters that matter for double-quoted attributes
/// are escaped. This is intentionally minimal -- we are generating
/// well-formed `content="..."` attributes, not arbitrary HTML.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}
```

- [ ] **Step 3: Add `make_absolute_url` helper function**

Append to `src/build/seo.rs`:

```rust
/// Resolve a URL to absolute form using the site's base_url.
///
/// - Already absolute (`http://` or `https://`): returned as-is.
/// - Site-relative (starts with `/`): `base_url` is prepended.
/// - Trailing-slash normalization: avoids `https://example.com//path`.
fn make_absolute_url(url: &str, base_url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    let base = base_url.trim_end_matches('/');
    let path = if url.starts_with('/') { url } else { return url.to_string() };
    let mut result = String::with_capacity(base.len() + path.len());
    result.push_str(base);
    result.push_str(path);
    result
}
```

- [ ] **Step 4: Add `resolve_seo` function**

Append to `src/build/seo.rs`:

```rust
/// Merge frontmatter SEO fields with site-level defaults to produce
/// fully resolved values.
///
/// Priority: frontmatter > site.seo > auto-derived defaults.
fn resolve_seo(
    frontmatter_seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> ResolvedSeo {
    // Title: frontmatter > site.seo > site.name
    let title = frontmatter_seo
        .title
        .clone()
        .or_else(|| site_config.seo.title.clone())
        .unwrap_or_else(|| site_config.name.clone());

    // Description: frontmatter > site.seo (None if neither set)
    let description = frontmatter_seo
        .description
        .clone()
        .or_else(|| site_config.seo.description.clone());

    // Image: frontmatter > site.seo, resolved to absolute URL
    let image = frontmatter_seo
        .image
        .clone()
        .or_else(|| site_config.seo.image.clone())
        .map(|img| make_absolute_url(&img, &site_config.base_url));

    // og:type: frontmatter > site.seo > "website"
    let og_type = frontmatter_seo
        .og_type
        .clone()
        .unwrap_or_else(|| site_config.seo.og_type.clone());

    // twitter:card: frontmatter > site.seo > "summary_large_image",
    // but forced to "summary" when no image is available.
    let twitter_card = if image.is_none() {
        "summary".to_string()
    } else {
        frontmatter_seo
            .twitter_card
            .clone()
            .unwrap_or_else(|| site_config.seo.twitter_card.clone())
    };

    // twitter:site from site config only (no per-page override).
    let twitter_site = site_config.seo.twitter_site.clone();

    // Canonical URL: frontmatter override > auto-derived.
    let canonical_url = frontmatter_seo
        .canonical_url
        .clone()
        .unwrap_or_else(|| {
            // Strip index.html for cleaner canonical URLs.
            let clean_path = if current_url.ends_with("/index.html") {
                &current_url[..current_url.len() - "index.html".len()]
            } else {
                current_url
            };
            make_absolute_url(clean_path, &site_config.base_url)
        });

    ResolvedSeo {
        title,
        description,
        image,
        og_type,
        twitter_card,
        twitter_site,
        canonical_url,
        site_name: site_config.name.clone(),
    }
}
```

- [ ] **Step 5: Add `detect_existing_tags` and `has_canonical_link` functions**

Append to `src/build/seo.rs`:

```rust
/// Scan rendered HTML for existing `<meta property="og:*">` and
/// `<meta name="twitter:*">` tags.
///
/// Returns the set of tag names already present (e.g. "og:title",
/// "twitter:card").
fn detect_existing_tags(html: &str) -> HashSet<String> {
    let existing: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));

    // Scan for og:* tags (property attribute).
    let existing_og = existing.clone();
    let og_handler = lol_html::element!("meta[property]", move |el| {
        if let Some(prop) = el.get_attribute("property") {
            if prop.starts_with("og:") {
                existing_og.borrow_mut().insert(prop);
            }
        }
        Ok(())
    });

    // Scan for twitter:* tags (name attribute).
    let existing_tw = existing.clone();
    let tw_handler = lol_html::element!("meta[name]", move |el| {
        if let Some(name) = el.get_attribute("name") {
            if name.starts_with("twitter:") {
                existing_tw.borrow_mut().insert(name);
            }
        }
        Ok(())
    });

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![og_handler, tw_handler],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    existing.borrow().clone()
}

/// Check if the HTML already contains a `<link rel="canonical">` tag.
fn has_canonical_link(html: &str) -> bool {
    let found: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_clone = found.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!(
                "link[rel='canonical']",
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
```

- [ ] **Step 6: Add `generate_meta_html` function**

Append to `src/build/seo.rs`:

```rust
/// Append a `<meta>` tag to `out` if `name` is not in `existing`.
///
/// `attr` is the HTML attribute name: `"property"` for OG tags,
/// `"name"` for Twitter tags.
fn push_meta(
    out: &mut String,
    existing: &HashSet<String>,
    attr: &str,
    name: &str,
    content: &str,
) {
    use std::fmt::Write;
    if !existing.contains(name) {
        let _ = write!(
            out,
            r#"<meta {}="{}" content="{}">"#,
            attr,
            name,
            escape_attr(content),
        );
        out.push('\n');
    }
}

/// Generate meta tag HTML string for all SEO tags that are not
/// already present in the document.
///
/// Returns an empty string if all tags are already present.
fn generate_meta_html(
    seo: &ResolvedSeo,
    existing: &HashSet<String>,
    has_canonical: bool,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    // Open Graph tags.
    push_meta(&mut out, existing, "property", "og:title", &seo.title);
    if let Some(ref desc) = seo.description {
        push_meta(&mut out, existing, "property", "og:description", desc);
    }
    if let Some(ref img) = seo.image {
        push_meta(&mut out, existing, "property", "og:image", img);
    }
    push_meta(&mut out, existing, "property", "og:url", &seo.canonical_url);
    push_meta(&mut out, existing, "property", "og:type", &seo.og_type);
    push_meta(&mut out, existing, "property", "og:site_name", &seo.site_name);

    // Twitter Card tags.
    push_meta(&mut out, existing, "name", "twitter:card", &seo.twitter_card);
    push_meta(&mut out, existing, "name", "twitter:title", &seo.title);
    if let Some(ref desc) = seo.description {
        push_meta(&mut out, existing, "name", "twitter:description", desc);
    }
    if let Some(ref img) = seo.image {
        push_meta(&mut out, existing, "name", "twitter:image", img);
    }
    if let Some(ref handle) = seo.twitter_site {
        push_meta(&mut out, existing, "name", "twitter:site", handle);
    }

    // Canonical URL.
    if !has_canonical {
        let _ = write!(
            out,
            r#"<link rel="canonical" href="{}">"#,
            escape_attr(&seo.canonical_url),
        );
        out.push('\n');
    }

    out
}
```

> **NOTE:** The original version used two closures (`meta_property` and
> `meta_name`) that both mutably borrowed `out` and immutably borrowed
> `existing`. This does not compile in Rust because two closures cannot
> simultaneously hold mutable borrows of the same variable. The fix
> extracts the logic into a standalone `push_meta` function that takes
> `&mut String` explicitly, avoiding the borrow conflict.

- [ ] **Step 7: Add `inject_into_head` function**

Append to `src/build/seo.rs`:

```rust
/// Inject meta tag HTML into the `<head>` element.
///
/// Uses lol_html to append content at the end of `<head>`. Unlike
/// resource hints (which prepend early for performance), SEO tags are
/// appended late because their position does not affect performance
/// and this avoids interfering with preload hint ordering.
fn inject_into_head(html: &str, meta_html: &str) -> Result<String, String> {
    let meta_owned = meta_html.to_string();

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("head", move |el| {
                el.append(
                    &format!("\n{}\n", meta_owned),
                    lol_html::html_content::ContentType::Html,
                );
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| e.to_string())
}
```

- [ ] **Step 8: Add `inject_seo_tags` public function**

Append to `src/build/seo.rs`:

```rust
/// Inject Open Graph and Twitter Card meta tags into HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
///
/// Steps:
/// 1. Resolve SEO values from frontmatter + site config + defaults.
/// 2. Scan HTML for existing OG/Twitter meta tags.
/// 3. Generate meta tag HTML for missing tags only.
/// 4. Inject the tags into `<head>` using lol_html.
///
/// Returns the (possibly rewritten) HTML string.
pub fn inject_seo_tags(
    html: &str,
    frontmatter_seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> String {
    // 1. Resolve SEO values.
    let resolved = resolve_seo(frontmatter_seo, site_config, current_url);

    // 2. Detect existing tags.
    let existing = detect_existing_tags(html);
    let has_canonical = has_canonical_link(html);

    // 3. Generate meta HTML for missing tags.
    let meta_html = generate_meta_html(&resolved, &existing, has_canonical);

    if meta_html.is_empty() {
        return html.to_string();
    }

    // 4. Inject into <head>.
    match inject_into_head(html, &meta_html) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("Failed to inject SEO meta tags: {}", e);
            html.to_string()
        }
    }
}
```

- [ ] **Step 9: Add `resolve_seo_expressions` public function**

Append to `src/build/seo.rs`:

```rust
/// Resolve template expressions in SEO frontmatter fields.
///
/// Uses `minijinja::Environment::render_str` to evaluate expressions
/// like `{{ post.title }}` in the context of the current page.
///
/// If rendering fails for any field, the field is left as `None`
/// (falls back to site defaults) and a warning is logged.
///
/// For fields that contain no template expressions (no `{{` / `{%`),
/// the value is returned as-is without calling render_str (fast path).
pub fn resolve_seo_expressions(
    seo: &SeoMeta,
    env: &minijinja::Environment<'_>,
    ctx: &minijinja::Value,
) -> SeoMeta {
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
                    "Failed to resolve SEO field '{}' expression '{}': {}",
                    field_name,
                    value,
                    err,
                );
                None
            }
        }
    }

    SeoMeta {
        title: resolve_field(&seo.title, env, ctx, "title"),
        description: resolve_field(&seo.description, env, ctx, "description"),
        image: resolve_field(&seo.image, env, ctx, "image"),
        og_type: resolve_field(&seo.og_type, env, ctx, "og_type"),
        twitter_card: resolve_field(&seo.twitter_card, env, ctx, "twitter_card"),
        canonical_url: resolve_field(&seo.canonical_url, env, ctx, "canonical_url"),
    }
}
```

- [ ] **Step 10: Register the module in `build/mod.rs`**

In `src/build/mod.rs`, add `pub mod seo;` to the module declarations (after `pub mod render;`):

```rust
pub mod seo;
```

The complete `src/build/mod.rs` should read:

```rust
//! Build engine: output setup, context assembly, page rendering, fragment
//! extraction, and sitemap generation.

pub mod bundling;
pub mod content_hash;
pub mod context;
pub mod critical_css;
pub mod fragments;
pub mod hints;
pub mod minify;
pub mod output;
pub mod render;
pub mod seo;
pub mod sitemap;

pub use render::build;
```

- [ ] **Step 11: Verify compilation**

Run: `cargo check`

Expected: Clean compilation with no errors. There may be unused warnings for `Rc`, `RefCell` if tests haven't exercised the functions yet -- that is fine.

- [ ] **Step 12: Commit**

```bash
git add src/build/seo.rs src/build/mod.rs
git commit -m "feat(seo): add build::seo module with core logic

Add seo.rs with:
- resolve_seo: merges frontmatter + site config + auto-derived defaults
- detect_existing_tags / has_canonical_link: lol_html scan for duplicates
- generate_meta_html: builds OG/Twitter/canonical tag HTML string
- inject_into_head: lol_html injection into <head>
- inject_seo_tags: public orchestration entry point
- resolve_seo_expressions: minijinja template expression evaluation
- escape_attr / make_absolute_url: utility helpers
- ResolvedSeo: internal resolved values struct"
```

---

## Task 4: Write unit tests for `build::seo`

**Depends on:** Task 3

**Files:**
- Modify: `src/build/seo.rs` (append `#[cfg(test)]` block)

- [ ] **Step 1: Add test module with helper function**

Append to `src/build/seo.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteSeoConfig;

    /// Build a minimal SiteMeta for testing.
    fn test_site(name: &str, base_url: &str) -> SiteMeta {
        SiteMeta {
            name: name.to_string(),
            base_url: base_url.to_string(),
            seo: SiteSeoConfig::default(),
        }
    }

    /// Build a SiteMeta with custom SEO defaults.
    fn test_site_with_seo(
        name: &str,
        base_url: &str,
        seo: SiteSeoConfig,
    ) -> SiteMeta {
        SiteMeta {
            name: name.to_string(),
            base_url: base_url.to_string(),
            seo,
        }
    }
```

- [ ] **Step 2: Add `escape_attr` tests**

Append inside the `mod tests` block:

```rust
    #[test]
    fn test_escape_attr_special_chars() {
        assert_eq!(escape_attr(r#"He said "hello""#), r#"He said &quot;hello&quot;"#);
        assert_eq!(escape_attr("A & B"), "A &amp; B");
        assert_eq!(escape_attr("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_attr("plain text"), "plain text");
    }

    #[test]
    fn test_escape_attr_empty() {
        assert_eq!(escape_attr(""), "");
    }
```

- [ ] **Step 3: Add `make_absolute_url` tests**

Append inside the `mod tests` block:

```rust
    #[test]
    fn test_make_absolute_url_already_absolute() {
        assert_eq!(
            make_absolute_url("https://cdn.example.com/img.jpg", "https://example.com"),
            "https://cdn.example.com/img.jpg",
        );
    }

    #[test]
    fn test_make_absolute_url_relative_path() {
        assert_eq!(
            make_absolute_url("/assets/hero.jpg", "https://example.com"),
            "https://example.com/assets/hero.jpg",
        );
    }

    #[test]
    fn test_make_absolute_url_base_trailing_slash() {
        assert_eq!(
            make_absolute_url("/about.html", "https://example.com/"),
            "https://example.com/about.html",
        );
    }
```

- [ ] **Step 4: Add `resolve_seo` tests**

Append inside the `mod tests` block:

```rust
    #[test]
    fn test_resolve_seo_all_defaults() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let resolved = resolve_seo(&seo, &site, "/about.html");

        assert_eq!(resolved.title, "My Site");
        assert!(resolved.description.is_none());
        assert!(resolved.image.is_none());
        assert_eq!(resolved.og_type, "website");
        assert_eq!(resolved.twitter_card, "summary"); // no image -> summary
        assert!(resolved.twitter_site.is_none());
        assert_eq!(resolved.canonical_url, "https://example.com/about.html");
        assert_eq!(resolved.site_name, "My Site");
    }

    #[test]
    fn test_resolve_seo_frontmatter_overrides() {
        let site = test_site_with_seo("My Site", "https://example.com", SiteSeoConfig {
            title: Some("Site Default Title".into()),
            description: Some("Site default desc".into()),
            ..SiteSeoConfig::default()
        });
        let seo = SeoMeta {
            title: Some("Page Title".into()),
            description: Some("Page description".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.title, "Page Title");
        assert_eq!(resolved.description.as_deref(), Some("Page description"));
    }

    #[test]
    fn test_resolve_seo_site_defaults_fill_gaps() {
        let site = test_site_with_seo("My Site", "https://example.com", SiteSeoConfig {
            description: Some("Site description".into()),
            image: Some("/assets/default-share.jpg".into()),
            ..SiteSeoConfig::default()
        });
        let seo = SeoMeta::default(); // no frontmatter overrides
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.description.as_deref(), Some("Site description"));
        assert_eq!(resolved.image.as_deref(), Some("https://example.com/assets/default-share.jpg"));
    }

    #[test]
    fn test_resolve_seo_canonical_url_auto() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let resolved = resolve_seo(&seo, &site, "/blog/post.html");

        assert_eq!(resolved.canonical_url, "https://example.com/blog/post.html");
    }

    #[test]
    fn test_resolve_seo_canonical_url_override() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            canonical_url: Some("https://original-source.com/article".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/syndicated.html");

        assert_eq!(resolved.canonical_url, "https://original-source.com/article");
    }

    #[test]
    fn test_resolve_seo_canonical_strips_index() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();

        let resolved = resolve_seo(&seo, &site, "/index.html");
        assert_eq!(resolved.canonical_url, "https://example.com/");

        let resolved = resolve_seo(&seo, &site, "/blog/index.html");
        assert_eq!(resolved.canonical_url, "https://example.com/blog/");
    }

    #[test]
    fn test_resolve_seo_image_absolute_url() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            image: Some("https://cdn.example.com/photo.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.image.as_deref(), Some("https://cdn.example.com/photo.jpg"));
    }

    #[test]
    fn test_resolve_seo_image_relative_path() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            image: Some("/assets/hero.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.image.as_deref(), Some("https://example.com/assets/hero.jpg"));
    }

    #[test]
    fn test_resolve_seo_base_url_slash_normalization() {
        let site = test_site("My Site", "https://example.com/");
        let seo = SeoMeta {
            image: Some("/assets/hero.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/about.html");

        // No double slash.
        assert_eq!(resolved.image.as_deref(), Some("https://example.com/assets/hero.jpg"));
        assert_eq!(resolved.canonical_url, "https://example.com/about.html");
    }

    #[test]
    fn test_resolve_seo_twitter_card_no_image() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default(); // no image
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.twitter_card, "summary");
    }

    #[test]
    fn test_resolve_seo_twitter_card_with_image() {
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            image: Some("/assets/hero.jpg".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo(&seo, &site, "/page.html");

        assert_eq!(resolved.twitter_card, "summary_large_image");
    }
```

- [ ] **Step 5: Add `detect_existing_tags` and `has_canonical_link` tests**

Append inside the `mod tests` block:

```rust
    #[test]
    fn test_detect_existing_og_tags() {
        let html = r#"<html><head><meta property="og:title" content="Test"></head><body></body></html>"#;
        let tags = detect_existing_tags(html);
        assert!(tags.contains("og:title"));
        assert!(!tags.contains("og:description"));
    }

    #[test]
    fn test_detect_existing_twitter_tags() {
        let html = r#"<html><head><meta name="twitter:card" content="summary"></head><body></body></html>"#;
        let tags = detect_existing_tags(html);
        assert!(tags.contains("twitter:card"));
        assert!(!tags.contains("twitter:title"));
    }

    #[test]
    fn test_detect_no_existing_tags() {
        let html = r#"<html><head><title>Page</title></head><body></body></html>"#;
        let tags = detect_existing_tags(html);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_has_canonical_link_present() {
        let html = r#"<html><head><link rel="canonical" href="https://example.com/page"></head><body></body></html>"#;
        assert!(has_canonical_link(html));
    }

    #[test]
    fn test_has_canonical_link_absent() {
        let html = r#"<html><head><title>Page</title></head><body></body></html>"#;
        assert!(!has_canonical_link(html));
    }
```

- [ ] **Step 6: Add `generate_meta_html` tests**

Append inside the `mod tests` block:

```rust
    #[test]
    fn test_generate_meta_html_full() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: Some("A description".into()),
            image: Some("https://example.com/img.jpg".into()),
            og_type: "website".into(),
            twitter_card: "summary_large_image".into(),
            twitter_site: Some("@mysite".into()),
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), false);

        assert!(html.contains(r#"<meta property="og:title" content="My Page">"#));
        assert!(html.contains(r#"<meta property="og:description" content="A description">"#));
        assert!(html.contains(r#"<meta property="og:image" content="https://example.com/img.jpg">"#));
        assert!(html.contains(r#"<meta property="og:url" content="https://example.com/page.html">"#));
        assert!(html.contains(r#"<meta property="og:type" content="website">"#));
        assert!(html.contains(r#"<meta property="og:site_name" content="My Site">"#));
        assert!(html.contains(r#"<meta name="twitter:card" content="summary_large_image">"#));
        assert!(html.contains(r#"<meta name="twitter:title" content="My Page">"#));
        assert!(html.contains(r#"<meta name="twitter:description" content="A description">"#));
        assert!(html.contains(r#"<meta name="twitter:image" content="https://example.com/img.jpg">"#));
        assert!(html.contains(r#"<meta name="twitter:site" content="@mysite">"#));
        assert!(html.contains(r#"<link rel="canonical" href="https://example.com/page.html">"#));
    }

    #[test]
    fn test_generate_meta_html_no_description() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: None,
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), false);

        assert!(!html.contains("og:description"));
        assert!(!html.contains("twitter:description"));
    }

    #[test]
    fn test_generate_meta_html_no_image() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: None,
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), false);

        assert!(!html.contains("og:image"));
        assert!(!html.contains("twitter:image"));
    }

    #[test]
    fn test_generate_meta_html_skips_existing() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: Some("Desc".into()),
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let mut existing = HashSet::new();
        existing.insert("og:title".into());
        existing.insert("twitter:card".into());
        let html = generate_meta_html(&seo, &existing, false);

        assert!(!html.contains(r#"<meta property="og:title""#));
        assert!(!html.contains(r#"<meta name="twitter:card""#));
        // Other tags should still be present.
        assert!(html.contains(r#"<meta property="og:description""#));
        assert!(html.contains(r#"<meta name="twitter:title""#));
    }

    #[test]
    fn test_generate_meta_html_skips_canonical() {
        let seo = ResolvedSeo {
            title: "My Page".into(),
            description: None,
            image: None,
            og_type: "website".into(),
            twitter_card: "summary".into(),
            twitter_site: None,
            canonical_url: "https://example.com/page.html".into(),
            site_name: "My Site".into(),
        };
        let html = generate_meta_html(&seo, &HashSet::new(), true);

        assert!(!html.contains("canonical"));
    }
```

- [ ] **Step 7: Add `inject_seo_tags` integration tests**

Append inside the `mod tests` block:

```rust
    #[test]
    fn test_inject_seo_tags_basic() {
        let html = "<html><head><title>Test</title></head><body>Hello</body></html>";
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            title: Some("Page Title".into()),
            description: Some("Page desc".into()),
            ..SeoMeta::default()
        };
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        assert!(result.contains(r#"<meta property="og:title" content="Page Title">"#));
        assert!(result.contains(r#"<meta property="og:description" content="Page desc">"#));
        assert!(result.contains(r#"<meta name="twitter:card" content="summary">"#));
        assert!(result.contains(r#"<link rel="canonical" href="https://example.com/page.html">"#));
        // Original content is preserved.
        assert!(result.contains("<title>Test</title>"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn test_inject_seo_tags_no_head() {
        let html = "<div>No head element</div>";
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        // Should not crash. No <head> means no injection target --
        // lol_html returns the HTML without matching the `head` selector.
        assert!(!result.is_empty());
        assert!(result.contains("No head element"));
        // No meta tags injected.
        assert!(!result.contains("og:title"));
    }

    #[test]
    fn test_inject_seo_tags_all_existing() {
        let html = concat!(
            r#"<html><head>"#,
            r#"<meta property="og:title" content="Existing">"#,
            r#"<meta property="og:url" content="https://example.com/page">"#,
            r#"<meta property="og:type" content="website">"#,
            r#"<meta property="og:site_name" content="Site">"#,
            r#"<meta name="twitter:card" content="summary">"#,
            r#"<meta name="twitter:title" content="Existing">"#,
            r#"<link rel="canonical" href="https://example.com/page">"#,
            r#"</head><body></body></html>"#,
        );
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta::default();
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        // No new tags injected -- the existing ones are preserved, no duplicates.
        // Count og:title occurrences to ensure no duplication.
        assert_eq!(result.matches("og:title").count(), 1);
        assert_eq!(result.matches("twitter:card").count(), 1);
    }

    #[test]
    fn test_inject_seo_tags_partial_existing() {
        let html = concat!(
            r#"<html><head>"#,
            r#"<meta property="og:title" content="Custom Title">"#,
            r#"</head><body></body></html>"#,
        );
        let site = test_site("My Site", "https://example.com");
        let seo = SeoMeta {
            title: Some("Page Title".into()),
            ..SeoMeta::default()
        };
        let result = inject_seo_tags(html, &seo, &site, "/page.html");

        // og:title should NOT be duplicated (existing one kept).
        assert_eq!(result.matches("og:title").count(), 1);
        assert!(result.contains("Custom Title"));
        // But other tags should be injected.
        assert!(result.contains("og:url"));
        assert!(result.contains("og:type"));
        assert!(result.contains("twitter:card"));
        assert!(result.contains("canonical"));
    }
```

- [ ] **Step 8: Add `resolve_seo_expressions` tests**

Append inside the `mod tests` block:

```rust
    #[test]
    fn test_resolve_seo_expressions_basic() {
        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        let ctx = minijinja::Value::from_serialize(
            &serde_json::json!({"post": {"title": "Hello World", "excerpt": "An intro"}}),
        );
        let seo = SeoMeta {
            title: Some("{{ post.title }} | Blog".into()),
            description: Some("{{ post.excerpt }}".into()),
            image: None,
            og_type: Some("article".into()),
            twitter_card: None,
            canonical_url: None,
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        assert_eq!(resolved.title.as_deref(), Some("Hello World | Blog"));
        assert_eq!(resolved.description.as_deref(), Some("An intro"));
        assert_eq!(resolved.og_type.as_deref(), Some("article")); // no expression, pass-through
    }

    #[test]
    fn test_resolve_seo_expressions_no_expressions() {
        let env = minijinja::Environment::new();
        let ctx = minijinja::Value::from(true); // dummy context
        let seo = SeoMeta {
            title: Some("Static Title".into()),
            description: Some("Static desc".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        assert_eq!(resolved.title.as_deref(), Some("Static Title"));
        assert_eq!(resolved.description.as_deref(), Some("Static desc"));
    }

    #[test]
    fn test_resolve_seo_expressions_empty_result() {
        let env = minijinja::Environment::new();
        let ctx = minijinja::Value::from_serialize(
            &serde_json::json!({"post": {"excerpt": ""}}),
        );
        let seo = SeoMeta {
            description: Some("{{ post.excerpt }}".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        // Empty result treated as None.
        assert!(resolved.description.is_none());
    }

    #[test]
    fn test_resolve_seo_expressions_missing_var() {
        let mut env = minijinja::Environment::new();
        // Match production config: strict mode causes error on undefined vars.
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
        let ctx = minijinja::Value::from_serialize(&serde_json::json!({}));
        let seo = SeoMeta {
            title: Some("{{ nonexistent.field }}".into()),
            description: Some("Static fallback".into()),
            ..SeoMeta::default()
        };
        let resolved = resolve_seo_expressions(&seo, &env, &ctx);

        // Missing var in strict mode -> render_str errors -> field becomes None.
        assert!(resolved.title.is_none());
        // Static field unaffected.
        assert_eq!(resolved.description.as_deref(), Some("Static fallback"));
    }
```

- [ ] **Step 9: Close the test module**

Append to close the `mod tests` block:

```rust
}
```

- [ ] **Step 10: Run all seo tests**

Run: `cargo test --lib build::seo -- --nocapture`

Expected: All tests pass. If `resolve_seo_expressions_missing_var` depends on minijinja's strict mode behavior (undefined variables may render as empty string rather than error depending on config), adjust the assertion accordingly -- the test should verify that the field either becomes `None` or falls through gracefully.

- [ ] **Step 11: Commit**

```bash
git add src/build/seo.rs
git commit -m "test(seo): add comprehensive unit tests for build::seo

Cover escape_attr, make_absolute_url, resolve_seo (all fallback
chains, canonical URL stripping, slash normalization, twitter card
no-image downgrade), detect_existing_tags, has_canonical_link,
generate_meta_html (full/partial/skip-existing/skip-canonical),
inject_seo_tags (basic/no-head/all-existing/partial-existing),
and resolve_seo_expressions (basic/no-expressions/empty/missing-var)."
```

---

## Task 5: Wire SEO injection into the build pipeline

**Depends on:** Task 3 (build::seo module exists)

**Files:**
- Modify: `src/build/render.rs`

- [ ] **Step 1: Add import for the seo module**

In `src/build/render.rs`, add `use super::seo;` to the imports (after `use super::hints;`, around line 25):

```rust
use super::seo;
```

- [ ] **Step 2: Add SEO expression resolution and injection to `render_static_page`**

In `render_static_page` in `src/build/render.rs`, make two changes:

**Change A: Resolve SEO expressions after context build.**

After the line `let ctx = context::build_page_context(config, global_data, &page_data, meta, None);` (around line 338), add:

```rust
    // Resolve SEO template expressions (static pages rarely use these,
    // but support them for consistency).
    let resolved_seo = seo::resolve_seo_expressions(
        &page.frontmatter.seo,
        env,
        &ctx,
    );
```

**Change B: Add SEO injection step after hints, before minify.**

After the hints injection block (the `if config.build.hints.enabled { ... }` block that ends around line 413), and before the minify block (`if config.build.minify { ... }`), add:

```rust
    // 4e. SEO meta tag injection (after hints, before minify).
    let full_html = seo::inject_seo_tags(
        &full_html,
        &resolved_seo,
        &config.site,
        &url_path,
    );
```

Also update the subsequent minify comment from `// 4e.` to `// 4f.` to keep numbering consistent:

```rust
    // 4f. Minify HTML (last transformation before writing).
```

- [ ] **Step 3: Add SEO expression resolution and injection to `render_dynamic_page`**

In `render_dynamic_page` in `src/build/render.rs`, make two changes inside the per-item loop:

**Change A: Resolve SEO expressions after context build.**

After the line `let ctx = context::build_page_context(config, global_data, &item_data, meta, Some((item_as, item)));` (around line 594-600), add:

```rust
        // Resolve SEO template expressions for this item.
        let resolved_seo = seo::resolve_seo_expressions(
            &page.frontmatter.seo,
            env,
            &ctx,
        );
```

**Change B: Add SEO injection step after hints, before minify.**

After the hints injection block inside the per-item loop (the `if config.build.hints.enabled { ... }` block that ends around line 680), and before the minify block, add:

```rust
        // SEO meta tag injection (after hints, before minify).
        let full_html = seo::inject_seo_tags(
            &full_html,
            &resolved_seo,
            &config.site,
            &url_path,
        );
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`

Expected: Clean compilation with no errors.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test -- --nocapture`

Expected: All existing tests pass. No regressions. The SEO injection is purely additive -- existing tests that check rendered HTML output will still pass because they either:
- Don't check for the absence of meta tags, or
- Use dev-mode rendering (which skips the SEO step).

- [ ] **Step 6: Commit**

```bash
git add src/build/render.rs
git commit -m "feat(seo): wire SEO injection into build pipeline

Call resolve_seo_expressions after context build and inject_seo_tags
after hints and before minify in both render_static_page and
render_dynamic_page. For dynamic pages, SEO expressions are resolved
per-item so {{ post.title }} works correctly."
```

---

## Task 6: Write feature documentation

**Depends on:** Task 5 (feature is complete)

**Files:**
- Create: `docs/og_twitter_meta.md`

- [ ] **Step 1: Write the documentation file**

Create `docs/og_twitter_meta.md`:

```markdown
# Open Graph / Twitter Card Meta Tags

## Overview

Eigen auto-injects Open Graph (`og:*`) and Twitter Card (`twitter:*`)
meta tags into every page's `<head>` during the build pipeline. This
ensures shared links on social media (Twitter/X, Facebook, LinkedIn,
Slack, Discord, etc.) display rich preview cards with title,
description, and image.

No template changes are required. The feature is always-on.

## How It Works

1. SEO values are resolved from a three-layer cascade:
   - Auto-derived defaults (title = site.name, og:type = "website", canonical URL = base_url + page path)
   - Site-level defaults from `[site.seo]` in `site.toml`
   - Per-page overrides from `[seo]` in template frontmatter

2. The rendered HTML is scanned for existing OG/Twitter meta tags.
3. Only missing tags are generated and injected into `<head>`.

## Configuration

### Site-level defaults (`site.toml`)

```toml
[site]
name = "My Blog"
base_url = "https://blog.example.com"

[site.seo]
title = "My Blog"                          # optional, falls back to site.name
description = "A blog about interesting things"  # optional
image = "/assets/default-share.jpg"        # optional, site-relative or absolute URL
og_type = "website"                        # default: "website"
twitter_site = "@myblog"                   # optional, Twitter @handle
twitter_card = "summary_large_image"       # default: "summary_large_image"
```

All fields are optional. If `[site.seo]` is omitted entirely, sensible
defaults are used.

### Per-page frontmatter

```yaml
---
seo:
  title: About Us
  description: Learn about our team and mission
  image: /assets/about-hero.jpg
  og_type: website
  twitter_card: summary_large_image
  canonical_url: https://example.com/about
---
```

All fields are optional. When absent, site-level defaults are used.

### Dynamic pages with template expressions

For dynamic pages, SEO field values can reference collection item data
using minijinja template expressions:

```yaml
---
collection:
  source: blog_api
  path: /posts
item_as: post
seo:
  title: "{{ post.title }} | My Blog"
  description: "{{ post.excerpt }}"
  image: "{{ post.cover_image }}"
  og_type: article
---
```

Expressions are resolved per-item using the same template context
used for the page template.

## Generated Tags

For a fully configured page, these tags are generated:

```html
<!-- Open Graph -->
<meta property="og:title" content="Page Title">
<meta property="og:description" content="Page description">
<meta property="og:image" content="https://example.com/assets/share.jpg">
<meta property="og:url" content="https://example.com/about.html">
<meta property="og:type" content="website">
<meta property="og:site_name" content="My Site">

<!-- Twitter Card -->
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="Page Title">
<meta name="twitter:description" content="Page description">
<meta name="twitter:image" content="https://example.com/assets/share.jpg">
<meta name="twitter:site" content="@mysite">

<!-- Canonical URL -->
<link rel="canonical" href="https://example.com/about.html">
```

Tags are omitted when their value is not available:
- No description at any level: `og:description` and `twitter:description` omitted
- No image at any level: `og:image`, `twitter:image` omitted, `twitter:card` forced to `"summary"`
- No `twitter_site` configured: `twitter:site` omitted

## Duplicate Detection

If a template already contains OG/Twitter meta tags or a canonical
link, the build pipeline detects them and does not inject duplicates.
This allows template authors to manually control specific tags when
needed.

## URL Resolution

- Image paths starting with `/` are resolved to absolute URLs using `site.base_url`
- Already-absolute URLs (`http://` or `https://`) are used as-is
- Canonical URLs are auto-generated from `site.base_url + page_path`
- `/index.html` paths are normalized to `/` in canonical URLs

## Pipeline Position

SEO injection runs after preload/prefetch hints and before HTML
minification. It does not run during dev server rendering.

## Module

`src/build/seo.rs` -- single file containing all resolution, detection,
generation, and injection logic.
```

- [ ] **Step 2: Commit**

```bash
git add docs/og_twitter_meta.md
git commit -m "docs(seo): add Open Graph / Twitter Card feature documentation

Document configuration (site.toml and frontmatter), generated tags,
dynamic page template expressions, duplicate detection, URL resolution,
and pipeline position."
```

---

## Summary

| Task | Description | Files | Depends on |
|------|-------------|-------|------------|
| 1 | Add `SiteSeoConfig` to configuration | `src/config/mod.rs` + 6 test helper files (context, sitemap, discovery, environment, functions, filters) | -- |
| 2 | Add `SeoMeta` to frontmatter | `src/frontmatter/mod.rs` | -- |
| 3 | Create `build::seo` module (core logic) | `src/build/seo.rs`, `src/build/mod.rs` | 1, 2 |
| 4 | Write unit tests for `build::seo` | `src/build/seo.rs` | 3 |
| 5 | Wire SEO injection into build pipeline | `src/build/render.rs` | 3 |
| 6 | Write feature documentation | `docs/og_twitter_meta.md` | 5 |

**Total tasks: 6**
**Total steps: 45**
**Parallelizable: Tasks 1 and 2 are independent and can run in parallel.**
