# robots.txt Generation -- Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-generate a valid `robots.txt` file at `dist/robots.txt` during build. Add `[robots]` config in `site.toml` specifying allow/disallow rules per user-agent. Auto-reference the generated `sitemap.xml` URL. Support additional sitemap URLs via `extra_sitemaps`.

**Architecture:** A new `src/build/robots.rs` module generates plain-text `robots.txt` from config. The `[robots]` table in `site.toml` is `Option<RobotsConfig>` -- when absent, no file is generated; when present (even empty), defaults apply. Generation runs in the build pipeline after sitemap generation and before feed generation.

**Tech Stack:** Rust, eyre (error handling), serde (config deserialization). No new dependencies.

---

## Task 1: Add `RobotsConfig` and `RobotsRule` to `src/config/mod.rs`

### 1.1 Add config structs and defaults

- [ ] Add the following after the `FeedConfig` default functions (after `default_slug_field()` at line 462, before `SiteSeoConfig` at line 464):

```rust
/// Configuration for robots.txt generation.
///
/// Located under `[robots]` in site.toml. When present (even as an
/// empty table), a robots.txt file is generated during build.
#[derive(Debug, Clone, Deserialize)]
pub struct RobotsConfig {
    /// Whether to include a `Sitemap:` directive for the generated sitemap.xml.
    #[serde(default = "default_true")]
    pub sitemap: bool,

    /// Additional absolute sitemap URLs to include as `Sitemap:` directives.
    #[serde(default)]
    pub extra_sitemaps: Vec<String>,

    /// Rule groups. Defaults to a single rule allowing all crawlers.
    #[serde(default = "default_robots_rules")]
    pub rules: Vec<RobotsRule>,
}

/// A single user-agent rule group in robots.txt.
#[derive(Debug, Clone, Deserialize)]
pub struct RobotsRule {
    /// The user-agent string, e.g. `"*"` or `"Googlebot"`.
    pub user_agent: String,

    /// Paths to allow.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Paths to disallow.
    #[serde(default)]
    pub disallow: Vec<String>,
}

fn default_robots_rules() -> Vec<RobotsRule> {
    vec![RobotsRule {
        user_agent: "*".to_string(),
        allow: vec!["/".to_string()],
        disallow: Vec::new(),
    }]
}
```

### 1.2 Add `robots` field to `SiteConfig`

- [ ] In `SiteConfig` struct (around line 9-26), add after `feed`:

```rust
    /// robots.txt generation configuration.
    /// When present, a robots.txt file is generated during build.
    /// When absent (`None`), no robots.txt is generated.
    #[serde(default)]
    pub robots: Option<RobotsConfig>,
```

### 1.3 Add robots.txt validation

- [ ] Add a new validation function after `validate_feed_configs` (around line 675):

```rust
/// Validate robots.txt configuration.
fn validate_robots_config(config: &SiteConfig) -> Result<()> {
    let robots = match &config.robots {
        Some(r) => r,
        None => return Ok(()),
    };

    for (i, rule) in robots.rules.iter().enumerate() {
        if rule.user_agent.is_empty() {
            bail!(
                "robots.rules[{}] has an empty `user_agent`. \
                 Each rule must specify a user-agent string.",
                i,
            );
        }

        if rule.allow.is_empty() && rule.disallow.is_empty() {
            tracing::warn!(
                "robots.rules[{}] (user-agent '{}') has no allow or \
                 disallow directives. This rule has no effect.",
                i,
                rule.user_agent,
            );
        }
    }

    for (i, url) in robots.extra_sitemaps.iter().enumerate() {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            bail!(
                "robots.extra_sitemaps[{}] = '{}' is not an absolute URL. \
                 Sitemap URLs must start with http:// or https://.",
                i,
                url,
            );
        }
    }

    Ok(())
}
```

- [ ] Call `validate_robots_config(config)?;` at the end of `validate_config` (line 620, after `validate_feed_configs(config)?;` and before `Ok(())`).

### 1.4 Update test helpers

- [ ] Update ALL `test_config()` functions that manually construct `SiteConfig` to include `robots: None`. Files and exact locations:
  - `src/build/sitemap.rs` line 79-93: add `robots: None,` after `feed: HashMap::new(),`
  - `src/build/context.rs` line 90-103: add `robots: None,` after `feed: HashMap::new(),`
  - `src/discovery/mod.rs` line 219-231: add `robots: None,` after `feed: HashMap::new(),`
  - `src/template/functions.rs` line 117-135 (`test_config()`): add `robots: None,` after `feed: HashMap::new(),`
  - `src/template/functions.rs` line 138-154 (`test_config_no_fragments()`): add `robots: None,` after `feed: HashMap::new(),`
  - `src/template/environment.rs` line 175-191: add `robots: None,` after `feed: HashMap::new(),`

### 1.5 Add config tests

- [ ] Add tests in `src/config/mod.rs` `tests` module:

```rust
// --- Robots config tests ---

#[test]
fn test_robots_config_absent() {
    let toml_str = r#"
[site]
name = "No Robots"
base_url = "https://example.com"
"#;
    let config = parse_toml(toml_str).unwrap();
    assert!(config.robots.is_none());
}

#[test]
fn test_robots_config_empty_table() {
    let toml_str = r#"
[site]
name = "Robots Default"
base_url = "https://example.com"

[robots]
"#;
    let config = parse_toml(toml_str).unwrap();
    assert!(config.robots.is_some());
    let robots = config.robots.unwrap();
    assert!(robots.sitemap);
    assert!(robots.extra_sitemaps.is_empty());
    assert_eq!(robots.rules.len(), 1);
    assert_eq!(robots.rules[0].user_agent, "*");
    assert_eq!(robots.rules[0].allow, vec!["/"]);
    assert!(robots.rules[0].disallow.is_empty());
}

#[test]
fn test_robots_config_full() {
    let toml_str = r#"
[site]
name = "Robots Full"
base_url = "https://example.com"

[robots]
sitemap = true
extra_sitemaps = ["https://example.com/news-sitemap.xml"]

[[robots.rules]]
user_agent = "*"
allow = ["/"]
disallow = ["/admin/", "/private/"]

[[robots.rules]]
user_agent = "BadBot"
disallow = ["/"]
"#;
    let config = parse_toml(toml_str).unwrap();
    let robots = config.robots.unwrap();
    assert!(robots.sitemap);
    assert_eq!(robots.extra_sitemaps, vec!["https://example.com/news-sitemap.xml"]);
    assert_eq!(robots.rules.len(), 2);
    assert_eq!(robots.rules[0].user_agent, "*");
    assert_eq!(robots.rules[0].allow, vec!["/"]);
    assert_eq!(robots.rules[0].disallow, vec!["/admin/", "/private/"]);
    assert_eq!(robots.rules[1].user_agent, "BadBot");
    assert!(robots.rules[1].allow.is_empty());
    assert_eq!(robots.rules[1].disallow, vec!["/"]);
}

#[test]
fn test_robots_config_no_sitemap() {
    let toml_str = r#"
[site]
name = "Robots No Sitemap"
base_url = "https://example.com"

[robots]
sitemap = false
"#;
    let config = parse_toml(toml_str).unwrap();
    let robots = config.robots.unwrap();
    assert!(!robots.sitemap);
}

#[test]
fn test_robots_config_multiple_rules() {
    let toml_str = r#"
[site]
name = "Robots Multi"
base_url = "https://example.com"

[[robots.rules]]
user_agent = "Googlebot"
allow = ["/"]

[[robots.rules]]
user_agent = "Bingbot"
allow = ["/public/"]
disallow = ["/private/"]
"#;
    let config = parse_toml(toml_str).unwrap();
    let robots = config.robots.unwrap();
    assert_eq!(robots.rules.len(), 2);
}
```

### 1.6 Add validation tests

- [ ] Add validation tests:

```rust
#[test]
fn test_robots_validation_empty_user_agent() {
    let toml_str = r#"
[site]
name = "Bad Robots"
base_url = "https://example.com"

[[robots.rules]]
user_agent = ""
allow = ["/"]
"#;
    let config = parse_toml(toml_str).unwrap();
    let result = validate_robots_config(&config);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("user_agent"));
}

#[test]
fn test_robots_validation_bad_extra_sitemap() {
    let toml_str = r#"
[site]
name = "Bad Sitemap URL"
base_url = "https://example.com"

[robots]
extra_sitemaps = ["/sitemap-news.xml"]
"#;
    let config = parse_toml(toml_str).unwrap();
    let result = validate_robots_config(&config);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("absolute URL"));
}
```

---

## Task 2: Create `src/build/robots.rs`

### 2.1 Module structure

- [ ] Create `src/build/robots.rs` with the following structure:

```rust
//! robots.txt generation.
//!
//! When `[robots]` is configured in site.toml, generates a `robots.txt`
//! file in `dist/` with user-agent rules and sitemap references.

use eyre::{Result, WrapErr};
use std::path::Path;

use crate::config::{RobotsConfig, RobotsRule, SiteConfig};
```

### 2.2 Implement `generate_robots_txt` (public entry point)

- [ ] Implement:

```rust
/// Generate `robots.txt` and write it to `dist/robots.txt`.
///
/// Only called when `config.robots` is `Some`. The caller checks this.
pub fn generate_robots_txt(
    dist_dir: &Path,
    config: &SiteConfig,
) -> Result<()> {
    let robots = match &config.robots {
        Some(r) => r,
        None => return Ok(()),
    };

    let base_url = config.site.base_url.trim_end_matches('/');
    let content = build_robots_content(robots, base_url);

    let robots_path = dist_dir.join("robots.txt");
    std::fs::write(&robots_path, &content)
        .wrap_err_with(|| format!("Failed to write {}", robots_path.display()))?;

    Ok(())
}
```

### 2.3 Implement `build_robots_content`

- [ ] Implement the content builder (separated from I/O for testability):

```rust
/// Build the full robots.txt content string.
fn build_robots_content(robots: &RobotsConfig, base_url: &str) -> String {
    let mut content = String::with_capacity(256);

    // Write rule groups.
    for (i, rule) in robots.rules.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        format_rule(&mut content, rule);
    }

    // Write Sitemap directives.
    let has_sitemaps = robots.sitemap || !robots.extra_sitemaps.is_empty();
    if has_sitemaps && !robots.rules.is_empty() {
        content.push('\n');
    }

    if robots.sitemap {
        content.push_str("Sitemap: ");
        content.push_str(base_url);
        content.push_str("/sitemap.xml\n");
    }

    for url in &robots.extra_sitemaps {
        content.push_str("Sitemap: ");
        content.push_str(url);
        content.push('\n');
    }

    content
}
```

### 2.4 Implement `format_rule`

- [ ] Implement rule formatting:

```rust
/// Append a single rule group to the output string.
fn format_rule(out: &mut String, rule: &RobotsRule) {
    out.push_str("User-agent: ");
    out.push_str(&rule.user_agent);
    out.push('\n');

    for path in &rule.allow {
        out.push_str("Allow: ");
        out.push_str(path);
        out.push('\n');
    }

    for path in &rule.disallow {
        out.push_str("Disallow: ");
        out.push_str(path);
        out.push('\n');
    }
}
```

### 2.5 Add unit tests

- [ ] Add comprehensive tests in `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        BuildConfig, RobotsConfig, RobotsRule, SiteConfig, SiteMeta,
        SiteSchemaConfig, SiteSeoConfig,
    };
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn test_config_with_robots(robots: RobotsConfig) -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://example.com".into(),
                seo: SiteSeoConfig::default(),
                schema: SiteSchemaConfig::default(),
            },
            build: BuildConfig::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
            feed: HashMap::new(),
            robots: Some(robots),
        }
    }

    fn default_robots() -> RobotsConfig {
        RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: Vec::new(),
            }],
        }
    }

    // --- build_robots_content ---

    #[test]
    fn test_build_content_default() {
        let robots = default_robots();
        let content = build_robots_content(&robots, "https://example.com");
        assert_eq!(
            content,
            "User-agent: *\nAllow: /\n\nSitemap: https://example.com/sitemap.xml\n"
        );
    }

    #[test]
    fn test_build_content_custom_rules() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![
                RobotsRule {
                    user_agent: "*".into(),
                    allow: vec!["/".into()],
                    disallow: vec!["/admin/".into(), "/private/".into()],
                },
                RobotsRule {
                    user_agent: "Googlebot".into(),
                    allow: vec!["/".into()],
                    disallow: Vec::new(),
                },
            ],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(content.contains("User-agent: *\nAllow: /\nDisallow: /admin/\nDisallow: /private/\n"));
        assert!(content.contains("User-agent: Googlebot\nAllow: /\n"));
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml\n"));
    }

    #[test]
    fn test_build_content_no_sitemap() {
        let robots = RobotsConfig {
            sitemap: false,
            extra_sitemaps: Vec::new(),
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: Vec::new(),
            }],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(!content.contains("Sitemap:"));
    }

    #[test]
    fn test_build_content_extra_sitemaps() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: vec!["https://example.com/news-sitemap.xml".into()],
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: Vec::new(),
            }],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml\n"));
        assert!(content.contains("Sitemap: https://example.com/news-sitemap.xml\n"));
    }

    #[test]
    fn test_build_content_trailing_slash_base_url() {
        let robots = default_robots();
        let content = build_robots_content(&robots, "https://example.com");
        // base_url is already trimmed by the caller, but verify no double slash.
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml"));
        assert!(!content.contains("https://example.com//sitemap.xml"));
    }

    #[test]
    fn test_build_content_disallow_only() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: Vec::new(),
                disallow: vec!["/secret/".into()],
            }],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(content.contains("User-agent: *\nDisallow: /secret/\n"));
        assert!(!content.contains("Allow:"));
    }

    #[test]
    fn test_build_content_empty_rules() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: Vec::new(),
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(!content.contains("User-agent:"));
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml\n"));
    }

    // --- format_rule ---

    #[test]
    fn test_format_rule_basic() {
        let rule = RobotsRule {
            user_agent: "*".into(),
            allow: vec!["/".into()],
            disallow: Vec::new(),
        };
        let mut out = String::new();
        format_rule(&mut out, &rule);
        assert_eq!(out, "User-agent: *\nAllow: /\n");
    }

    #[test]
    fn test_format_rule_multiple_paths() {
        let rule = RobotsRule {
            user_agent: "Googlebot".into(),
            allow: vec!["/public/".into(), "/blog/".into()],
            disallow: vec!["/admin/".into()],
        };
        let mut out = String::new();
        format_rule(&mut out, &rule);
        assert_eq!(
            out,
            "User-agent: Googlebot\nAllow: /public/\nAllow: /blog/\nDisallow: /admin/\n"
        );
    }

    // --- generate_robots_txt (integration with filesystem) ---

    #[test]
    fn test_generate_robots_txt_default() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let config = test_config_with_robots(default_robots());
        generate_robots_txt(&dist, &config).unwrap();

        let content = fs::read_to_string(dist.join("robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Allow: /"));
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml"));
    }

    #[test]
    fn test_generate_robots_txt_none() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let config = SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://example.com".into(),
                seo: SiteSeoConfig::default(),
                schema: SiteSchemaConfig::default(),
            },
            build: BuildConfig::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
            feed: HashMap::new(),
            robots: None,
        };
        generate_robots_txt(&dist, &config).unwrap();

        // No file should be written.
        assert!(!dist.join("robots.txt").exists());
    }

    #[test]
    fn test_generate_robots_txt_custom_rules() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![
                RobotsRule {
                    user_agent: "*".into(),
                    allow: vec!["/".into()],
                    disallow: vec!["/admin/".into()],
                },
                RobotsRule {
                    user_agent: "BadBot".into(),
                    allow: Vec::new(),
                    disallow: vec!["/".into()],
                },
            ],
        };
        let config = test_config_with_robots(robots);
        generate_robots_txt(&dist, &config).unwrap();

        let content = fs::read_to_string(dist.join("robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Disallow: /admin/"));
        assert!(content.contains("User-agent: BadBot"));
        assert!(content.contains("Sitemap:"));
    }
}
```

---

## Task 3: Register module in `src/build/mod.rs`

- [ ] Add `pub mod robots;` to `src/build/mod.rs` between `pub mod render;` (line 14) and `pub mod seo;` (line 15), maintaining alphabetical order:

```rust
pub mod render;
pub mod robots;
pub mod seo;
```

---

## Task 4: Integrate into build pipeline in `src/build/render.rs`

- [ ] Add `use super::robots;` to the imports between `use super::output;` and `use super::seo;`, maintaining alphabetical order. Current imports (lines 20-31):

```rust
use super::bundling;
use super::content_hash;
use super::context::{self, PageMeta};
use super::critical_css;
use super::feed;
use super::fragments;
use super::hints;
use super::json_ld;
use super::minify;
use super::output;
use super::robots;    // <-- NEW
use super::seo;
use super::sitemap;
```

- [ ] After the sitemap generation block (after line 200 `tracing::info!("Generating sitemap... done");`), before the feed generation block (line 203 `if !config.feed.is_empty()`), insert:

```rust
    // Generate robots.txt.
    if config.robots.is_some() {
        robots::generate_robots_txt(&dist_dir, &config)?;
        tracing::info!("Generating robots.txt... done");
    }
```

---

## Task 5: Run tests and verify

- [ ] Run `cargo test` to confirm all existing tests pass with the new `robots` field on `SiteConfig`.
- [ ] Run `cargo test robots` to confirm all new robots-specific tests pass.
- [ ] Run `cargo clippy` to confirm no warnings.

---

## Summary

**Total tasks: 5** (with 14 sub-steps)

| Task | Files | Description |
|---|---|---|
| 1 | `src/config/mod.rs` | Add `RobotsConfig`, `RobotsRule`, `SiteConfig.robots` field, validation, defaults, tests |
| 2 | `src/build/robots.rs` | NEW: robots.txt generation module with tests |
| 3 | `src/build/mod.rs` | Register `robots` module |
| 4 | `src/build/render.rs` | Call `generate_robots_txt` in build pipeline |
| 5 | (all) | Run tests, clippy, verify |

**New dependencies:** None. Uses existing `eyre`, `serde`.

**Estimated test count:** 12 unit tests in `robots.rs` + 7 config tests in `config/mod.rs` = ~19 new tests.

**Affected files summary:**

| File | Action |
|---|---|
| `src/config/mod.rs` | MODIFY: add ~100 lines (structs, defaults, validation, tests) |
| `src/build/mod.rs` | MODIFY: add 1 line |
| `src/build/robots.rs` | CREATE: ~250 lines (module + tests) |
| `src/build/render.rs` | MODIFY: add ~6 lines (import + call) |
| `src/build/sitemap.rs` | MODIFY: add `robots: None` to test helper (line 92) |
| `src/build/context.rs` | MODIFY: add `robots: None` to test helper (line 102) |
| `src/discovery/mod.rs` | MODIFY: add `robots: None` to test helper (line 230) |
| `src/template/functions.rs` | MODIFY: add `robots: None` to 2 test helpers (lines 134, 153) |
| `src/template/environment.rs` | MODIFY: add `robots: None` to test helper (line 190) |
