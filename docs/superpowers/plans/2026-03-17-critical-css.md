# Critical CSS Inlining Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract per-page "used CSS" from rendered HTML, inline it as a `<style>` block in `<head>`, and defer the full stylesheet load to eliminate render-blocking CSS.

**Architecture:** A new `build::critical_css` module with three files (`mod.rs`, `extract.rs`, `rewrite.rs`) slots into the existing build pipeline between plugin `post_render_html` hooks and HTML minification. It uses `scraper` for DOM querying (selector matching against rendered HTML) and `lightningcss` for CSS parsing/serialization. `lol_html` (already a dependency) handles the HTML rewriting (injecting `<style>`, rewriting `<link>` tags). A `StylesheetCache` avoids redundant filesystem reads across pages. The feature is opt-in via `[build.critical_css]` in `site.toml` and defaults to disabled.

**Tech Stack:** Rust, lightningcss (CSS parsing), scraper (HTML DOM + selector matching), lol_html (HTML streaming rewrite), regex (pseudo-selector stripping), glob (exclude pattern matching)

**Design spec:** `docs/superpowers/specs/2026-03-17-critical-css-design.md`

---

## File Structure

### New files to create

| File | Responsibility |
|------|---------------|
| `src/build/critical_css/mod.rs` | Public API (`inline_critical_css`, `StylesheetCache`), orchestration, caching, link extraction |
| `src/build/critical_css/extract.rs` | CSS parsing with lightningcss, recursive rule walking, selector matching via scraper, pseudo-selector stripping, global dependency collection (@font-face, @keyframes, custom properties) |
| `src/build/critical_css/rewrite.rs` | lol_html-based HTML mutation: inject `<style>` block, rewrite/remove `<link>` tags, add `<noscript>` fallbacks |
| `docs/critical_css.md` | Feature documentation for future reference |

### Existing files to modify

| File | Change |
|------|--------|
| `Cargo.toml` | Add `lightningcss = "1"` and `scraper = "0.25"` dependencies |
| `src/config/mod.rs` | Add `CriticalCssConfig` struct, add `critical_css` field to `BuildConfig`, update `Default` impl |
| `src/build/mod.rs` | Add `pub mod critical_css;` declaration |
| `src/build/render.rs` | Create `StylesheetCache` in `build()`, pass it through render functions, insert critical CSS step between plugin hooks and minification in both `render_static_page` and `render_dynamic_page` |

---

## Task 1: Add dependencies and configuration types

**Depends on:** Nothing (starting point)

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/config/mod.rs`

- [ ] **Step 1: Add crate dependencies to Cargo.toml**

Add these two lines to the `[dependencies]` section of `Cargo.toml` (alphabetical order, after `lol_html`):

```toml
lightningcss = "1"
```

Note: `lightningcss` is already an indirect dependency via `minify-html` (version `1.0.0-alpha.71` in the lockfile). Adding it as a direct dependency will reuse the same version.

And after `reqwest`:

```toml
scraper = "0.25"
```

- [ ] **Step 2: Add `CriticalCssConfig` struct to `src/config/mod.rs`**

Add the following after the `ImageOptimConfig` struct and its `Default` impl (around line 169, after the `default_image_exclude` function). Note: `default_true` already exists at line 65 and can be reused for `preload_full`.

```rust
/// Configuration for critical CSS inlining.
///
/// Located under `[build.critical_css]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct CriticalCssConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Maximum size in bytes for the inlined `<style>` block.
    /// If the critical CSS exceeds this, fall back to the original
    /// `<link>` tag (no inlining for that page).
    /// Default: 50_000 (50 KB).
    #[serde(default = "default_max_inline_size")]
    pub max_inline_size: usize,

    /// Whether to keep the original `<link>` tag for async loading of
    /// the full stylesheet. Default: true.
    /// When false, the `<link>` is removed entirely (pure tree-shaking mode).
    #[serde(default = "default_true")]
    pub preload_full: bool,

    /// Glob patterns for stylesheet paths to exclude from critical CSS
    /// processing. Matched against the href value.
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_max_inline_size() -> usize {
    50_000
}

impl Default for CriticalCssConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_inline_size: default_max_inline_size(),
            preload_full: true,
            exclude: Vec::new(),
        }
    }
}
```

- [ ] **Step 3: Add `critical_css` field to `BuildConfig`**

In `src/config/mod.rs`, add a new field to the `BuildConfig` struct (after the `minify` field, around line 50):

```rust
    /// Critical CSS inlining configuration.
    #[serde(default)]
    pub critical_css: CriticalCssConfig,
```

Update the `Default` impl for `BuildConfig` (around line 53) to include:

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
        }
    }
}
```

- [ ] **Step 4: Write tests for the new config types**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/config/mod.rs`:

```rust
    #[test]
    fn test_critical_css_config_defaults() {
        let toml_str = r#"
[site]
name = "CSS Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.critical_css.enabled);
        assert_eq!(config.build.critical_css.max_inline_size, 50_000);
        assert!(config.build.critical_css.preload_full);
        assert!(config.build.critical_css.exclude.is_empty());
    }

    #[test]
    fn test_critical_css_config_custom() {
        let toml_str = r#"
[site]
name = "CSS Custom"
base_url = "https://example.com"

[build.critical_css]
enabled = true
max_inline_size = 30000
preload_full = false
exclude = ["**/vendor/**", "**/print.css"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.critical_css.enabled);
        assert_eq!(config.build.critical_css.max_inline_size, 30_000);
        assert!(!config.build.critical_css.preload_full);
        assert_eq!(config.build.critical_css.exclude.len(), 2);
    }

    #[test]
    fn test_critical_css_enabled_only() {
        let toml_str = r#"
[site]
name = "CSS Enabled"
base_url = "https://example.com"

[build.critical_css]
enabled = true
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.critical_css.enabled);
        // Other fields should have defaults.
        assert_eq!(config.build.critical_css.max_inline_size, 50_000);
        assert!(config.build.critical_css.preload_full);
    }
```

- [ ] **Step 5: Run tests to verify config changes compile and pass**

Run: `cargo test --lib config -- --nocapture`

Expected: All existing config tests pass, plus the 3 new tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/config/mod.rs
git commit -m "feat(critical-css): add CriticalCssConfig and dependencies

Add lightningcss and scraper crate dependencies.
Add CriticalCssConfig struct with enabled, max_inline_size, preload_full,
and exclude fields. Wire into BuildConfig with serde defaults."
```

---

## Task 2: Create `critical_css/extract.rs` -- pseudo-selector stripping

**Depends on:** Task 1 (dependencies must be in Cargo.toml)

**Files:**
- Create: `src/build/critical_css/extract.rs`

This task implements the `strip_pseudo_for_matching` and `selector_matches` functions. These are the foundational building blocks for CSS rule matching that later steps build on.

- [ ] **Step 1: Create the extract.rs file with pseudo-selector stripping**

Create the file `src/build/critical_css/extract.rs`:

```rust
//! CSS extraction: parse stylesheets, match selectors against HTML DOM,
//! collect critical rules with transitive dependencies.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

// Pre-compiled regex patterns (compiled once, reused across all calls).
// These are called per-selector and per-rule, so avoiding recompilation
// is critical for performance on large stylesheets.

/// Matches CSS pseudo-elements like `::before`, `::after`, `::placeholder`.
static PSEUDO_ELEMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"::[a-zA-Z-]+(?:\([^)]*\))?").expect("pseudo-element regex is valid")
});

/// Matches dynamic pseudo-classes like `:hover`, `:focus`, `:visited`,
/// but preserves structural ones (`:root`, `:first-child`, `:nth-child`,
/// `:not`, `:is`, `:where`, `:has`, `:empty`, etc.) that scraper supports.
static PSEUDO_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r":(?!root\b|first-child\b|last-child\b|nth-child\b|nth-last-child\b|nth-of-type\b|nth-last-of-type\b|first-of-type\b|last-of-type\b|only-child\b|only-of-type\b|empty\b|not\b|is\b|where\b|has\b|:)[a-zA-Z-]+(?:\([^)]*\))?"
    ).expect("pseudo-class regex is valid")
});

/// Matches `font-family` declarations.
static FONT_FAMILY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)font-family\s*:\s*([^;]+)"#).expect("font-family regex is valid")
});

/// Matches `animation-name` declarations.
static ANIM_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)animation-name\s*:\s*([^;]+)"#).expect("animation-name regex is valid")
});

/// Matches `animation` shorthand declarations.
static ANIM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)animation\s*:\s*([^;]+)").expect("animation regex is valid")
});

/// Matches `var(--name)` references.
static VAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"var\(\s*(--[a-zA-Z0-9_-]+)").expect("var() regex is valid")
});

/// Matches CSS custom property definitions (`--name:`).
static CUSTOM_PROP_DEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(--[a-zA-Z0-9_-]+)\s*:").expect("custom prop regex is valid")
});

/// Strip dynamic pseudo-classes and pseudo-elements from a selector string
/// to produce a version matchable against a static DOM.
///
/// Returns `Some(stripped)` if the result is non-empty, or `None` if stripping
/// produces an empty selector (e.g. `::selection`) -- meaning the rule should
/// be included unconditionally.
///
/// Structural pseudo-classes (`:root`, `:first-child`, `:nth-child`, etc.) are
/// preserved because `scraper` supports them natively.
pub fn strip_pseudo_for_matching(selector: &str) -> Option<String> {
    let result = PSEUDO_ELEMENT_RE.replace_all(selector, "");
    let result = PSEUDO_CLASS_RE.replace_all(&result, "");

    // Collapse any leftover whitespace.
    let result = result.split_whitespace().collect::<Vec<_>>().join(" ");
    let result = result.trim().to_string();

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Test whether a CSS selector matches any element in the HTML document.
///
/// If the selector cannot be parsed by scraper, returns `true` (conservative:
/// include the rule rather than risk dropping it).
pub fn selector_matches(selector: &str, document: &scraper::Html) -> bool {
    match scraper::Selector::parse(selector) {
        Ok(sel) => document.select(&sel).next().is_some(),
        Err(_) => {
            tracing::debug!("Could not parse selector for matching: {}", selector);
            true // conservative: include the rule
        }
    }
}

/// Tracks which global rules (font-face, keyframes, custom properties)
/// are transitively referenced by matched style rules.
#[derive(Debug, Default)]
pub struct GlobalDependencies {
    /// Font family names referenced by matched rules.
    pub font_families: HashSet<String>,
    /// Animation names referenced by matched rules.
    pub animation_names: HashSet<String>,
    /// CSS custom property names (e.g. "--color-primary") referenced
    /// via var() in matched rules.
    pub custom_properties: HashSet<String>,
}

/// Scan a CSS declaration block (serialized text) for references to font
/// families, animation names, and CSS custom properties.
pub fn collect_global_deps(declarations: &str) -> GlobalDependencies {
    let mut deps = GlobalDependencies::default();

    // Font family: match `font-family:` property values.
    // Extract quoted or unquoted family names.
    if let Some(cap) = FONT_FAMILY_RE.captures(declarations) {
        let families = &cap[1];
        for family in families.split(',') {
            let name = family.trim().trim_matches('"').trim_matches('\'').trim();
            if !name.is_empty() && !is_generic_font_family(name) {
                deps.font_families.insert(name.to_string());
            }
        }
    }

    // Animation names: match `animation-name:` declaration.
    if let Some(cap) = ANIM_NAME_RE.captures(declarations) {
        let names = &cap[1];
        for name in names.split(',') {
            let name = name.trim();
            if !name.is_empty() && name != "none" && name != "initial" && name != "inherit" {
                deps.animation_names.insert(name.to_string());
            }
        }
    }

    // Also check `animation:` shorthand -- animation name is typically the
    // first non-numeric, non-timing-function, non-keyword token.
    if let Some(cap) = ANIM_RE.captures(declarations) {
        // Only process if animation-name wasn't already found.
        if deps.animation_names.is_empty() {
            extract_animation_names_from_shorthand(&cap[1], &mut deps.animation_names);
        }
    }

    // CSS custom properties: match `var(--name)` references.
    for cap in VAR_RE.captures_iter(declarations) {
        deps.custom_properties.insert(cap[1].to_string());
    }

    deps
}

/// Check if a font family name is a generic CSS family keyword.
fn is_generic_font_family(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy"
        | "system-ui" | "ui-serif" | "ui-sans-serif" | "ui-monospace"
        | "ui-rounded" | "emoji" | "math" | "fangsong"
        | "inherit" | "initial" | "unset" | "revert"
    )
}

/// Extract animation names from the `animation` shorthand value.
///
/// The shorthand format is: `name duration timing-function delay ...`
/// We look for identifiers that are not timing keywords or durations.
fn extract_animation_names_from_shorthand(value: &str, names: &mut HashSet<String>) {
    let timing_keywords: HashSet<&str> = [
        "ease", "ease-in", "ease-out", "ease-in-out", "linear",
        "step-start", "step-end", "infinite", "none", "normal",
        "reverse", "alternate", "alternate-reverse", "forwards",
        "backwards", "both", "running", "paused", "initial",
        "inherit", "unset",
    ].into_iter().collect();

    // Each comma-separated value is one animation.
    for animation in value.split(',') {
        for token in animation.split_whitespace() {
            let token = token.trim();
            // Skip durations (e.g. "1s", "200ms", "0.5s").
            if token.ends_with('s') || token.ends_with("ms") {
                continue;
            }
            // Skip numeric values.
            if token.parse::<f64>().is_ok() {
                continue;
            }
            // Skip timing keywords.
            if timing_keywords.contains(token.to_lowercase().as_str()) {
                continue;
            }
            // Skip cubic-bezier/steps functions.
            if token.starts_with("cubic-bezier") || token.starts_with("steps(") {
                continue;
            }
            // This is likely the animation name.
            if !token.is_empty() {
                names.insert(token.to_string());
                break; // Only one name per animation value.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_pseudo_for_matching tests ---

    #[test]
    fn test_strip_pseudo_hover() {
        assert_eq!(
            strip_pseudo_for_matching(".btn:hover"),
            Some(".btn".to_string())
        );
    }

    #[test]
    fn test_strip_pseudo_before() {
        assert_eq!(
            strip_pseudo_for_matching(".icon::before"),
            Some(".icon".to_string())
        );
    }

    #[test]
    fn test_strip_pseudo_preserves_structural() {
        // :root should NOT be stripped.
        assert_eq!(
            strip_pseudo_for_matching(":root"),
            Some(":root".to_string())
        );
        assert_eq!(
            strip_pseudo_for_matching(":first-child"),
            Some(":first-child".to_string())
        );
        assert_eq!(
            strip_pseudo_for_matching(":nth-child(2n)"),
            Some(":nth-child(2n)".to_string())
        );
        assert_eq!(
            strip_pseudo_for_matching(":not(.hidden)"),
            Some(":not(.hidden)".to_string())
        );
    }

    #[test]
    fn test_strip_pseudo_selection() {
        // ::selection with no base selector -> None (include unconditionally).
        assert_eq!(strip_pseudo_for_matching("::selection"), None);
    }

    #[test]
    fn test_strip_pseudo_compound() {
        assert_eq!(
            strip_pseudo_for_matching("a:hover > .icon::after"),
            Some("a > .icon".to_string())
        );
    }

    #[test]
    fn test_strip_pseudo_focus_visible() {
        assert_eq!(
            strip_pseudo_for_matching(".input:focus-visible"),
            Some(".input".to_string())
        );
    }

    #[test]
    fn test_strip_pseudo_visited() {
        assert_eq!(
            strip_pseudo_for_matching("a:visited"),
            Some("a".to_string())
        );
    }

    #[test]
    fn test_strip_pseudo_placeholder() {
        assert_eq!(
            strip_pseudo_for_matching("input::placeholder"),
            Some("input".to_string())
        );
    }

    // --- selector_matches tests ---

    #[test]
    fn test_selector_matches_basic() {
        let html = scraper::Html::parse_document(r#"<div class="exists">Hello</div>"#);
        assert!(selector_matches(".exists", &html));
    }

    #[test]
    fn test_selector_matches_absent() {
        let html = scraper::Html::parse_document(r#"<div class="exists">Hello</div>"#);
        assert!(!selector_matches(".missing", &html));
    }

    #[test]
    fn test_selector_matches_tag() {
        let html = scraper::Html::parse_document("<p>Hello</p>");
        assert!(selector_matches("p", &html));
        assert!(!selector_matches("span", &html));
    }

    #[test]
    fn test_selector_matches_id() {
        let html = scraper::Html::parse_document(r#"<div id="main">Hello</div>"#);
        assert!(selector_matches("#main", &html));
        assert!(!selector_matches("#other", &html));
    }

    #[test]
    fn test_selector_matches_combinator() {
        let html = scraper::Html::parse_document(r#"<div class="parent"><span class="child">Hi</span></div>"#);
        assert!(selector_matches(".parent > .child", &html));
        assert!(!selector_matches(".parent > .other", &html));
    }

    #[test]
    fn test_selector_matches_unparseable() {
        // An unparseable selector should return true (conservative: include the rule).
        let html = scraper::Html::parse_document("<div>Hello</div>");
        assert!(selector_matches("!!invalid%%selector", &html));
    }

    // --- collect_global_deps tests ---

    #[test]
    fn test_collect_font_family() {
        let decls = r#"font-family: "Inter", sans-serif;"#;
        let deps = collect_global_deps(decls);
        assert!(deps.font_families.contains("Inter"));
        assert!(!deps.font_families.contains("sans-serif"));
    }

    #[test]
    fn test_collect_animation_name() {
        let decls = "animation-name: spin;";
        let deps = collect_global_deps(decls);
        assert!(deps.animation_names.contains("spin"));
    }

    #[test]
    fn test_collect_animation_shorthand() {
        let decls = "animation: fadeIn 0.3s ease-in-out;";
        let deps = collect_global_deps(decls);
        assert!(deps.animation_names.contains("fadeIn"));
    }

    #[test]
    fn test_collect_var_references() {
        let decls = "color: var(--color-primary); background: var(--bg-main);";
        let deps = collect_global_deps(decls);
        assert!(deps.custom_properties.contains("--color-primary"));
        assert!(deps.custom_properties.contains("--bg-main"));
    }

    #[test]
    fn test_collect_no_deps() {
        let decls = "color: red; margin: 0;";
        let deps = collect_global_deps(decls);
        assert!(deps.font_families.is_empty());
        assert!(deps.animation_names.is_empty());
        assert!(deps.custom_properties.is_empty());
    }
}
```

- [ ] **Step 2: Create a minimal `mod.rs` to make the module compilable**

Create `src/build/critical_css/mod.rs`:

```rust
//! Critical CSS inlining: extract per-page used CSS and inline it to
//! eliminate render-blocking stylesheets.

pub mod extract;
pub mod rewrite;
```

Create a placeholder `src/build/critical_css/rewrite.rs`:

```rust
//! HTML rewriting for critical CSS: inject `<style>` blocks and
//! rewrite `<link>` tags for deferred loading.
```

- [ ] **Step 3: Register the module in `src/build/mod.rs`**

Add `pub mod critical_css;` to `src/build/mod.rs` after `pub mod context;`:

```rust
pub mod context;
pub mod critical_css;
pub mod fragments;
```

- [ ] **Step 4: Run the extract.rs tests**

Run: `cargo test --lib build::critical_css::extract -- --nocapture`

Expected: All 15 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/build/critical_css/ src/build/mod.rs
git commit -m "feat(critical-css): add pseudo-selector stripping and selector matching

Implement strip_pseudo_for_matching (strips dynamic pseudo-classes and
pseudo-elements while preserving structural ones), selector_matches
(DOM matching via scraper), and collect_global_deps (font-family,
animation-name, var() reference scanning)."
```

---

## Task 3: Implement CSS rule extraction (`extract_critical_css`)

**Depends on:** Task 2 (needs `strip_pseudo_for_matching`, `selector_matches`, `collect_global_deps`)

**Files:**
- Modify: `src/build/critical_css/extract.rs`

This task adds the main `extract_critical_css` function that parses a CSS string with lightningcss, walks all rules recursively, matches selectors against the HTML DOM, collects transitive global dependencies (@font-face, @keyframes, custom properties), and serializes the matching rules back to a CSS string.

- [ ] **Step 1: Add the `extract_critical_css` public function**

Add the following to `src/build/critical_css/extract.rs`, before the `#[cfg(test)]` block. Add `use lightningcss` imports at the top of the file:

```rust
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::rules::CssRule;
use lightningcss::traits::ToCss;
```

Then add the function:

```rust
/// Match CSS rules against an HTML document and return only the rules
/// whose selectors match at least one element, plus transitively
/// referenced global rules (@font-face, @keyframes, custom properties).
///
/// Returns the critical CSS as a serialized string. Returns an empty
/// string if no selectors match.
pub fn extract_critical_css(css: &str, document: &scraper::Html) -> Result<String, String> {
    let options = ParserOptions {
        error_recovery: true,
        ..ParserOptions::default()
    };
    let stylesheet = StyleSheet::parse(css, options)
        .map_err(|e| format!("CSS parse error: {}", e))?;

    let mut matched_rules: Vec<String> = Vec::new();
    let mut global_deps = GlobalDependencies::default();

    // First pass: collect @layer statement rules (always included) and
    // walk style rules for selector matching.
    let mut font_face_rules: Vec<String> = Vec::new();
    let mut keyframe_rules: Vec<String> = Vec::new();
    let mut custom_prop_rules: Vec<(String, String)> = Vec::new(); // (property_name, serialized_rule)

    walk_rules(
        &stylesheet.rules.0,
        document,
        &mut matched_rules,
        &mut global_deps,
        &mut font_face_rules,
        &mut keyframe_rules,
        &mut custom_prop_rules,
    );

    if matched_rules.is_empty() {
        return Ok(String::new());
    }

    // Second pass: include transitively referenced global rules.
    let mut critical_parts: Vec<String> = Vec::new();

    // Include referenced @font-face rules.
    for rule_css in &font_face_rules {
        // Check if any referenced font family appears in this @font-face.
        // We check the serialized rule text for the font family name.
        for family in &global_deps.font_families {
            if rule_css.contains(family) {
                critical_parts.push(rule_css.clone());
                break;
            }
        }
    }

    // Include referenced @keyframes rules.
    for rule_css in &keyframe_rules {
        for anim_name in &global_deps.animation_names {
            if rule_css.contains(anim_name) {
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

/// Recursively walk CSS rules, matching style rules against the DOM and
/// collecting global rules for later dependency resolution.
fn walk_rules(
    rules: &[CssRule],
    document: &scraper::Html,
    matched_rules: &mut Vec<String>,
    global_deps: &mut GlobalDependencies,
    font_face_rules: &mut Vec<String>,
    keyframe_rules: &mut Vec<String>,
    custom_prop_rules: &mut Vec<(String, String)>,
) {
    for rule in rules {
        match rule {
            CssRule::Style(style_rule) => {
                handle_style_rule(style_rule, document, matched_rules, global_deps, custom_prop_rules);
            }
            CssRule::Media(media_rule) => {
                // Recursively process child rules inside @media.
                let mut media_matched: Vec<String> = Vec::new();
                walk_rules(
                    &media_rule.rules.0,
                    document,
                    &mut media_matched,
                    global_deps,
                    font_face_rules,
                    keyframe_rules,
                    custom_prop_rules,
                );
                if !media_matched.is_empty() {
                    // Serialize the @media prelude.
                    let query = media_rule
                        .query
                        .to_css_string(PrinterOptions::default())
                        .unwrap_or_default();
                    let inner = media_matched.join("\n");
                    matched_rules.push(format!("@media {} {{\n{}\n}}", query, inner));
                }
            }
            CssRule::Supports(supports_rule) => {
                let mut supports_matched: Vec<String> = Vec::new();
                walk_rules(
                    &supports_rule.rules.0,
                    document,
                    &mut supports_matched,
                    global_deps,
                    font_face_rules,
                    keyframe_rules,
                    custom_prop_rules,
                );
                if !supports_matched.is_empty() {
                    let condition = supports_rule
                        .condition
                        .to_css_string(PrinterOptions::default())
                        .unwrap_or_default();
                    let inner = supports_matched.join("\n");
                    matched_rules.push(format!("@supports {} {{\n{}\n}}", condition, inner));
                }
            }
            CssRule::LayerBlock(layer_rule) => {
                let mut layer_matched: Vec<String> = Vec::new();
                walk_rules(
                    &layer_rule.rules.0,
                    document,
                    &mut layer_matched,
                    global_deps,
                    font_face_rules,
                    keyframe_rules,
                    custom_prop_rules,
                );
                if !layer_matched.is_empty() {
                    let name = layer_rule
                        .name
                        .as_ref()
                        .map(|n| {
                            n.to_css_string(PrinterOptions::default())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default();
                    let inner = layer_matched.join("\n");
                    if name.is_empty() {
                        matched_rules.push(format!("@layer {{\n{}\n}}", inner));
                    } else {
                        matched_rules.push(format!("@layer {} {{\n{}\n}}", name, inner));
                    }
                }
            }
            CssRule::LayerStatement(_) => {
                // Layer ordering declarations are always included.
                if let Ok(css) = rule.to_css_string(PrinterOptions::default()) {
                    matched_rules.push(css);
                }
            }
            CssRule::FontFace(_) => {
                // Collect for later dependency resolution.
                if let Ok(css) = rule.to_css_string(PrinterOptions::default()) {
                    font_face_rules.push(css);
                }
            }
            CssRule::Keyframes(_) => {
                if let Ok(css) = rule.to_css_string(PrinterOptions::default()) {
                    keyframe_rules.push(css);
                }
            }
            // Other rule types (import, charset, namespace, etc.) are
            // passed through unconditionally since they may be needed.
            CssRule::Import(_) | CssRule::Namespace(_) => {
                if let Ok(css) = rule.to_css_string(PrinterOptions::default()) {
                    matched_rules.push(css);
                }
            }
            _ => {
                // Unknown/other rules: include them to be safe.
                if let Ok(css) = rule.to_css_string(PrinterOptions::default()) {
                    matched_rules.push(css);
                }
            }
        }
    }
}

/// Handle a single style rule: check if its selectors match the DOM,
/// and if so, serialize it and collect its global dependencies.
/// Also collects custom property definitions from ALL style rules
/// (matched or not) so they can be included based on var() references.
fn handle_style_rule(
    style_rule: &lightningcss::rules::style::StyleRule,
    document: &scraper::Html,
    matched_rules: &mut Vec<String>,
    global_deps: &mut GlobalDependencies,
    custom_prop_rules: &mut Vec<(String, String)>,
) {
    let selector_list = style_rule
        .selectors
        .to_css_string(PrinterOptions::default())
        .unwrap_or_default();

    // Serialize the rule text for analysis (needed for both custom prop
    // collection and for including matched rules).
    let css_text = match style_rule.to_css_string(PrinterOptions::default()) {
        Ok(css) => css,
        Err(_) => return,
    };

    // Collect custom property definitions from this rule regardless of
    // whether it matches. These are needed for var() dependency resolution.
    let prop_defs = extract_custom_property_definitions(&css_text);
    custom_prop_rules.extend(prop_defs);

    // Check each selector in the comma-separated list.
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
                if selector_matches(&stripped, document) {
                    any_match = true;
                    break;
                }
            }
        }
    }

    if any_match {
        // Collect global dependencies from the declaration block.
        let deps = collect_global_deps(&css_text);
        global_deps.font_families.extend(deps.font_families);
        global_deps.animation_names.extend(deps.animation_names);
        global_deps.custom_properties.extend(deps.custom_properties);

        matched_rules.push(css_text);
    }
}

/// Extract custom property definitions from a serialized style rule.
/// Returns a list of (property_name, serialized_rule) pairs.
fn extract_custom_property_definitions(css: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for cap in CUSTOM_PROP_DEF_RE.captures_iter(css) {
        result.push((cap[1].to_string(), css.to_string()));
    }
    result
}
```

- [ ] **Step 2: Add tests for `extract_critical_css`**

Add to the test module in `extract.rs`:

```rust
    // --- extract_critical_css tests ---

    #[test]
    fn test_extract_simple() {
        let css = r#"
            .exists { color: red; }
            .missing { color: blue; }
        "#;
        let html = scraper::Html::parse_document(r#"<div class="exists">Hello</div>"#);
        let result = extract_critical_css(css, &html).unwrap();
        assert!(result.contains(".exists"));
        assert!(result.contains("color"));
        assert!(!result.contains(".missing"));
    }

    #[test]
    fn test_extract_media_query() {
        let css = r#"
            @media (max-width: 768px) {
                .exists { color: red; }
                .missing { color: blue; }
            }
        "#;
        let html = scraper::Html::parse_document(r#"<div class="exists">Hello</div>"#);
        let result = extract_critical_css(css, &html).unwrap();
        assert!(result.contains("@media"));
        assert!(result.contains(".exists"));
        assert!(!result.contains(".missing"));
    }

    #[test]
    fn test_extract_media_query_no_match() {
        let css = r#"
            @media (max-width: 768px) {
                .missing { color: blue; }
            }
        "#;
        let html = scraper::Html::parse_document(r#"<div class="exists">Hello</div>"#);
        let result = extract_critical_css(css, &html).unwrap();
        assert!(!result.contains("@media"));
        assert!(!result.contains(".missing"));
    }

    #[test]
    fn test_extract_font_face_transitive() {
        let css = r#"
            @font-face {
                font-family: "Inter";
                src: url("/fonts/inter.woff2") format("woff2");
            }
            .heading { font-family: "Inter", sans-serif; }
            .unused { color: green; }
        "#;
        let html = scraper::Html::parse_document(r#"<h1 class="heading">Title</h1>"#);
        let result = extract_critical_css(css, &html).unwrap();
        assert!(result.contains("@font-face"));
        assert!(result.contains("Inter"));
        assert!(result.contains(".heading"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_extract_keyframes_transitive() {
        let css = r#"
            @keyframes spin {
                from { transform: rotate(0deg); }
                to { transform: rotate(360deg); }
            }
            .spinner { animation-name: spin; }
            .unused { color: green; }
        "#;
        let html = scraper::Html::parse_document(r#"<div class="spinner">Loading</div>"#);
        let result = extract_critical_css(css, &html).unwrap();
        assert!(result.contains("@keyframes"));
        assert!(result.contains("spin"));
        assert!(result.contains(".spinner"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_extract_custom_property() {
        let css = r#"
            :root { --color-primary: blue; --unused-var: green; }
            .btn { color: var(--color-primary); }
        "#;
        let html = scraper::Html::parse_document(
            r#"<html><body><button class="btn">Click</button></body></html>"#
        );
        let result = extract_critical_css(css, &html).unwrap();
        // :root matches <html>, so the :root rule should be included via
        // normal selector matching regardless of var() tracking.
        assert!(result.contains(":root"));
        assert!(result.contains(".btn"));
    }

    #[test]
    fn test_extract_layer_statement() {
        let css = r#"
            @layer reset, base, components;
            .exists { color: red; }
        "#;
        let html = scraper::Html::parse_document(r#"<div class="exists">Hello</div>"#);
        let result = extract_critical_css(css, &html).unwrap();
        assert!(result.contains("@layer"));
        assert!(result.contains(".exists"));
    }

    #[test]
    fn test_extract_empty_result() {
        let css = r#"
            .missing { color: red; }
            .also-missing { color: blue; }
        "#;
        let html = scraper::Html::parse_document("<div>No matching classes</div>");
        let result = extract_critical_css(css, &html).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_pseudo_selector_included() {
        let css = r#"
            .btn { color: blue; }
            .btn:hover { color: red; }
        "#;
        let html = scraper::Html::parse_document(r#"<button class="btn">Click</button>"#);
        let result = extract_critical_css(css, &html).unwrap();
        // Both the base rule and the :hover rule should be included.
        assert!(result.contains(".btn:hover") || result.contains(".btn:hover"));
        assert!(result.contains("color"));
    }
```

- [ ] **Step 3: Run all extract tests**

Run: `cargo test --lib build::critical_css::extract -- --nocapture`

Expected: All tests pass (previous 15 + new 9 = 24 tests).

- [ ] **Step 4: Commit**

```bash
git add src/build/critical_css/extract.rs
git commit -m "feat(critical-css): implement CSS rule extraction with DOM matching

Add extract_critical_css which parses CSS with lightningcss, walks
rules recursively (handling @media, @supports, @layer blocks),
matches selectors against the HTML DOM via scraper, and collects
transitive @font-face/@keyframes/var() dependencies."
```

---

## Task 4: Implement HTML rewriting (`rewrite.rs`)

**Depends on:** Task 1 (needs lol_html dependency which is already present)

**Files:**
- Modify: `src/build/critical_css/rewrite.rs`

This task implements the `lol_html`-based HTML rewriting that injects a `<style>` block containing the critical CSS and rewrites `<link rel="stylesheet">` tags to load asynchronously (or removes them).

- [ ] **Step 1: Implement `rewrite_html` function**

Replace the placeholder content in `src/build/critical_css/rewrite.rs`:

```rust
//! HTML rewriting for critical CSS: inject `<style>` blocks and
//! rewrite `<link>` tags for deferred loading.
//!
//! Uses `lol_html` for streaming HTML rewriting. This is the same library
//! used by the asset localization module.

use std::cell::RefCell;
use std::rc::Rc;

/// Rewrite HTML to inline critical CSS and defer full stylesheets.
///
/// 1. Injects a `<style>` block containing `critical_css` before the first
///    processed `<link>` tag.
/// 2. Rewrites each `<link>` in `processed_hrefs` to load asynchronously
///    (if `preload_full` is true) or removes it entirely (if false).
/// 3. Leaves `<link>` tags not in `processed_hrefs` unchanged.
pub fn rewrite_html(
    html: &str,
    critical_css: &str,
    processed_hrefs: &[String],
    preload_full: bool,
) -> Result<String, String> {
    let style_injected = Rc::new(RefCell::new(false));
    let style_injected_clone = style_injected.clone();

    let critical_css_owned = critical_css.to_string();
    let processed_set: std::collections::HashSet<String> =
        processed_hrefs.iter().cloned().collect();
    let processed_set = Rc::new(processed_set);
    let processed_set_clone = processed_set.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("link[rel='stylesheet']", move |el| {
                    let href = match el.get_attribute("href") {
                        Some(h) => h,
                        None => return Ok(()),
                    };

                    // Only process links that we have critical CSS for.
                    if !processed_set_clone.contains(&href) {
                        return Ok(());
                    }

                    // Inject <style> block before the first processed link.
                    if !*style_injected_clone.borrow() {
                        let style_tag = format!("<style>{}</style>", critical_css_owned);
                        el.before(&style_tag, lol_html::html_content::ContentType::Html);
                        *style_injected_clone.borrow_mut() = true;
                    }

                    if preload_full {
                        // Rewrite to preload pattern:
                        // <link rel="preload" href="..." as="style"
                        //       onload="this.onload=null;this.rel='stylesheet'">
                        // <noscript><link rel="stylesheet" href="..."></noscript>
                        let preload_html = format!(
                            r#"<link rel="preload" href="{}" as="style" onload="this.onload=null;this.rel='stylesheet'"><noscript><link rel="stylesheet" href="{}"></noscript>"#,
                            href, href
                        );
                        el.replace(&preload_html, lol_html::html_content::ContentType::Html);
                    } else {
                        // Remove the link entirely (tree-shaking mode).
                        el.remove();
                    }

                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| format!("lol_html rewrite error: {}", e))?;

    Ok(output)
}

/// Extract local stylesheet hrefs from HTML.
///
/// Returns hrefs of `<link rel="stylesheet">` tags that:
/// - Have a non-empty `href` attribute
/// - Are not external (http/https)
/// - Do not have a `media` attribute (those are already non-blocking)
/// - Do not match any exclude patterns
pub fn extract_stylesheet_hrefs(html: &str, exclude: &[String]) -> Vec<String> {
    let hrefs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let hrefs_clone = hrefs.clone();
    let exclude_owned: Vec<String> = exclude.to_vec();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("link[rel='stylesheet']", move |el| {
                    // Skip if has media attribute (already non-blocking).
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
                    for pattern in &exclude_owned {
                        if let Ok(pat) = glob::Pattern::new(pattern) {
                            if pat.matches(&href) {
                                return Ok(());
                            }
                        }
                    }

                    hrefs_clone.borrow_mut().push(href);
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = hrefs.borrow().clone();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_stylesheet_hrefs tests ---

    #[test]
    fn test_extract_single_link() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert_eq!(hrefs, vec!["/css/style.css"]);
    }

    #[test]
    fn test_extract_multiple_links() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/main.css">
        </head></html>"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert_eq!(hrefs, vec!["/css/reset.css", "/css/main.css"]);
    }

    #[test]
    fn test_extract_skips_external() {
        let html = r#"<link rel="stylesheet" href="https://cdn.example.com/style.css">"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert!(hrefs.is_empty());
    }

    #[test]
    fn test_extract_skips_media_attr() {
        let html = r#"<link rel="stylesheet" href="/css/print.css" media="print">"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert!(hrefs.is_empty());
    }

    #[test]
    fn test_extract_skips_excluded_pattern() {
        let html = r#"<link rel="stylesheet" href="/css/vendor/bootstrap.css">"#;
        let hrefs = extract_stylesheet_hrefs(html, &["**/vendor/**".to_string()]);
        assert!(hrefs.is_empty());
    }

    #[test]
    fn test_extract_preserves_non_stylesheet_links() {
        let html = r#"<link rel="icon" href="/favicon.ico"><link rel="stylesheet" href="/css/style.css">"#;
        let hrefs = extract_stylesheet_hrefs(html, &[]);
        assert_eq!(hrefs, vec!["/css/style.css"]);
    }

    // --- rewrite_html tests ---

    #[test]
    fn test_rewrite_single_link() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><p>Hi</p></body></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            true,
        ).unwrap();

        assert!(result.contains("<style>body { color: red; }</style>"));
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains(r#"as="style""#));
        assert!(result.contains("onload="));
        assert!(result.contains("<noscript>"));
        // Original link should be gone.
        assert!(!result.contains(r#"<link rel="stylesheet" href="/css/style.css">"#));
    }

    #[test]
    fn test_rewrite_no_preload() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            false,
        ).unwrap();

        assert!(result.contains("<style>body { color: red; }</style>"));
        // No preload link, no noscript fallback.
        assert!(!result.contains("preload"));
        assert!(!result.contains("<noscript>"));
        // Original link should be removed.
        assert!(!result.contains(r#"href="/css/style.css""#));
    }

    #[test]
    fn test_rewrite_multiple_links() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/main.css">
        </head></html>"#;
        let result = rewrite_html(
            html,
            ".a { color: red; } .b { color: blue; }",
            &["/css/reset.css".to_string(), "/css/main.css".to_string()],
            true,
        ).unwrap();

        // Should have exactly one <style> block.
        assert_eq!(result.matches("<style>").count(), 1);
        // Both links should be rewritten to preload.
        assert_eq!(result.matches(r#"rel="preload""#).count(), 2);
    }

    #[test]
    fn test_rewrite_preserves_other_links() {
        let html = r#"<html><head>
            <link rel="icon" href="/favicon.ico">
            <link rel="stylesheet" href="/css/style.css">
        </head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            true,
        ).unwrap();

        // Favicon link should be untouched.
        assert!(result.contains(r#"<link rel="icon" href="/favicon.ico">"#));
    }

    #[test]
    fn test_rewrite_noscript_fallback() {
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/style.css".to_string()],
            true,
        ).unwrap();

        assert!(result.contains(r#"<noscript><link rel="stylesheet" href="/css/style.css"></noscript>"#));
    }

    #[test]
    fn test_rewrite_external_link_untouched() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="https://cdn.example.com/lib.css">
            <link rel="stylesheet" href="/css/local.css">
        </head></html>"#;
        let result = rewrite_html(
            html,
            "body { color: red; }",
            &["/css/local.css".to_string()],
            true,
        ).unwrap();

        // External link should remain unchanged.
        assert!(result.contains(r#"href="https://cdn.example.com/lib.css""#));
        // Local link should be rewritten.
        assert!(result.contains(r#"rel="preload""#));
    }
}
```

- [ ] **Step 2: Run the rewrite tests**

Run: `cargo test --lib build::critical_css::rewrite -- --nocapture`

Expected: All 11 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/build/critical_css/rewrite.rs
git commit -m "feat(critical-css): implement HTML rewriting for stylesheet deferral

Add extract_stylesheet_hrefs (lol_html-based link extraction) and
rewrite_html (inject <style>, rewrite <link> to preload or remove,
add <noscript> fallback). Handles multiple stylesheets, external
URLs, media attributes, and exclude patterns."
```

---

## Task 5: Implement the orchestrator (`mod.rs`) with `StylesheetCache`

**Depends on:** Tasks 3 and 4 (needs both extract and rewrite modules)

**Files:**
- Modify: `src/build/critical_css/mod.rs`

This task wires everything together: the `inline_critical_css` public function that orchestrates stylesheet loading, critical CSS extraction, size checking, and HTML rewriting. It also implements the `StylesheetCache`.

- [ ] **Step 1: Implement `mod.rs` with full orchestration logic**

Replace the content of `src/build/critical_css/mod.rs`:

```rust
//! Critical CSS inlining: extract per-page used CSS and inline it to
//! eliminate render-blocking stylesheets.
//!
//! This module provides `inline_critical_css`, the main entry point called
//! from the build pipeline. It:
//! 1. Parses HTML to find `<link rel="stylesheet">` tags.
//! 2. Reads each referenced local stylesheet from dist_dir.
//! 3. Parses the CSS and matches selectors against the HTML DOM.
//! 4. Inlines matched CSS as a `<style>` block in `<head>`.
//! 5. Rewrites `<link>` tags to load asynchronously or removes them.

pub mod extract;
pub mod rewrite;

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::config::CriticalCssConfig;

/// Matches `@import` directives in CSS (both `url()` and string forms).
static IMPORT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"@import\s+(?:url\(\s*['"]?([^'")]+)['"]?\s*\)|['"]([^'"]+)['"]);?"#
    ).expect("import regex is valid")
});

/// Cache for parsed stylesheets, keyed by href.
///
/// Avoids re-reading and re-resolving the same CSS file for every page.
/// Created once per build and passed through the pipeline.
pub struct StylesheetCache {
    cache: HashMap<String, String>,
}

impl StylesheetCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Get or load a stylesheet's CSS content.
    ///
    /// Returns `Some(&str)` if the stylesheet is cached or can be loaded,
    /// `None` if loading fails.
    fn get_or_load(&mut self, href: &str, dist_dir: &Path) -> Option<&str> {
        if !self.cache.contains_key(href) {
            match load_stylesheet(href, dist_dir) {
                Ok(css) => {
                    self.cache.insert(href.to_string(), css);
                }
                Err(e) => {
                    tracing::warn!("Failed to load stylesheet '{}': {}", href, e);
                    return None;
                }
            }
        }
        self.cache.get(href).map(|s| s.as_str())
    }
}

/// Extract critical CSS and inline it into the HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
    css_cache: &mut StylesheetCache,
) -> String {
    if !config.enabled {
        return html.to_string();
    }

    // Step 1: Find local stylesheet links.
    let hrefs = rewrite::extract_stylesheet_hrefs(html, &config.exclude);
    if hrefs.is_empty() {
        return html.to_string();
    }

    // Step 2: Parse the HTML into a DOM for selector matching.
    let document = scraper::Html::parse_document(html);

    // Step 3: Load and extract critical CSS from each stylesheet.
    let mut combined_critical_css = String::new();
    let mut processed_hrefs: Vec<String> = Vec::new();

    for href in &hrefs {
        let css_content = match css_cache.get_or_load(href, dist_dir) {
            Some(css) => css.to_string(),
            None => continue, // Warning already logged by get_or_load.
        };

        match extract::extract_critical_css(&css_content, &document) {
            Ok(critical) => {
                if !critical.is_empty() {
                    if !combined_critical_css.is_empty() {
                        combined_critical_css.push('\n');
                    }
                    combined_critical_css.push_str(&critical);
                    processed_hrefs.push(href.clone());
                }
            }
            Err(e) => {
                tracing::warn!("Failed to extract critical CSS from '{}': {}", href, e);
            }
        }
    }

    if combined_critical_css.is_empty() {
        return html.to_string();
    }

    // Step 4: Check size limit.
    if combined_critical_css.len() > config.max_inline_size {
        tracing::info!(
            "Critical CSS ({} bytes) exceeds max_inline_size ({} bytes), skipping inlining",
            combined_critical_css.len(),
            config.max_inline_size,
        );
        return html.to_string();
    }

    // Step 5: Rewrite HTML.
    match rewrite::rewrite_html(
        html,
        &combined_critical_css,
        &processed_hrefs,
        config.preload_full,
    ) {
        Ok(rewritten) => rewritten,
        Err(e) => {
            tracing::warn!("Failed to rewrite HTML for critical CSS: {}", e);
            html.to_string()
        }
    }
}

/// Read a CSS file from dist_dir, resolving the href to a filesystem path.
///
/// Resolves `@import` directives by inlining imported files. Uses a simple
/// recursive approach: reads the main file, finds `@import` statements via
/// regex, reads each imported file, and replaces the `@import` with the
/// imported content. This handles the common case of local `@import`s
/// without requiring lightningcss's full bundler API (which needs a
/// `SourceProvider` trait implementation).
///
/// External `@import` URLs (http/https) are left as-is.
fn load_stylesheet(href: &str, dist_dir: &Path) -> Result<String, String> {
    let relative = href.trim_start_matches('/');
    let css_path = dist_dir.join(relative);

    if !css_path.exists() {
        return Err(format!("File not found: {}", css_path.display()));
    }

    let css = std::fs::read_to_string(&css_path)
        .map_err(|e| format!("Failed to read {}: {}", css_path.display(), e))?;

    // Resolve @import directives.
    resolve_imports(&css, &css_path, dist_dir, 0)
}

/// Recursively resolve `@import` directives in CSS content.
/// `depth` prevents infinite recursion from circular imports.
fn resolve_imports(
    css: &str,
    css_path: &Path,
    dist_dir: &Path,
    depth: usize,
) -> Result<String, String> {
    const MAX_IMPORT_DEPTH: usize = 10;

    if depth > MAX_IMPORT_DEPTH {
        tracing::warn!(
            "Circular or deeply nested @import detected in {}",
            css_path.display()
        );
        return Ok(css.to_string());
    }

    let css_dir = css_path.parent().unwrap_or(dist_dir);
    let mut result = css.to_string();

    for cap in IMPORT_RE.captures_iter(css) {
        let import_path_str = cap.get(1).or(cap.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");

        // Skip external imports.
        if import_path_str.starts_with("http://") || import_path_str.starts_with("https://") {
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
                    // Recursively resolve imports in the imported file.
                    let resolved = resolve_imports(
                        &imported_css, &import_path, dist_dir, depth + 1
                    )?;
                    result = result.replace(cap.get(0).unwrap().as_str(), &resolved);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to read @import '{}': {}",
                        import_path.display(), e
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

    Ok(result)
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
    fn test_inline_critical_css_disabled() {
        let config = CriticalCssConfig::default(); // enabled: false
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head></html>"#;
        let result = inline_critical_css(html, &config, Path::new("/tmp"), &mut cache);
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_no_links() {
        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = "<html><head><title>No CSS</title></head><body><p>Hi</p></body></html>";
        let result = inline_critical_css(html, &config, Path::new("/tmp"), &mut cache);
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_file_not_found() {
        let tmp = TempDir::new().unwrap();
        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/missing.css"></head></html>"#;
        let result = inline_critical_css(html, &config, tmp.path(), &mut cache);
        // Should return original HTML unchanged.
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_basic() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", r#"
            .hero { color: red; }
            .unused { color: green; }
        "#);

        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache);

        assert!(result.contains("<style>"));
        assert!(result.contains(".hero"));
        assert!(!result.contains(".unused"));
        assert!(result.contains(r#"rel="preload""#));
        assert!(result.contains("<noscript>"));
    }

    #[test]
    fn test_inline_critical_css_exceeds_max_size() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        // Create CSS that will exceed a very small limit.
        let large_css = ".exists { color: red; padding: 0; margin: 0; border: none; }";
        write_file(dist, "css/style.css", large_css);

        let config = CriticalCssConfig {
            enabled: true,
            max_inline_size: 10, // Very small limit.
            ..Default::default()
        };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="exists">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache);

        // Should return original HTML unchanged (no inlining).
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_excluded_pattern() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/vendor/bootstrap.css", ".btn { color: red; }");

        let config = CriticalCssConfig {
            enabled: true,
            exclude: vec!["**/vendor/**".to_string()],
            ..Default::default()
        };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/vendor/bootstrap.css"></head><body><div class="btn">Click</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache);

        // Excluded stylesheet should not be processed.
        assert_eq!(result, html);
    }

    #[test]
    fn test_inline_critical_css_preload_false() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", ".hero { color: red; }");

        let config = CriticalCssConfig {
            enabled: true,
            preload_full: false,
            ..Default::default()
        };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache);

        assert!(result.contains("<style>"));
        assert!(result.contains(".hero"));
        // No preload or noscript.
        assert!(!result.contains("preload"));
        assert!(!result.contains("<noscript>"));
        // Link should be gone entirely.
        assert!(!result.contains(r#"href="/css/style.css""#));
    }

    #[test]
    fn test_inline_critical_css_with_import() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/base.css", ".base { margin: 0; }");
        write_file(dist, "css/style.css", r#"
            @import "base.css";
            .hero { color: red; }
            .unused { color: green; }
        "#);

        let config = CriticalCssConfig { enabled: true, ..Default::default() };
        let mut cache = StylesheetCache::new();
        let html = r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero base">Hello</div></body></html>"#;

        let result = inline_critical_css(html, &config, dist, &mut cache);

        assert!(result.contains("<style>"));
        assert!(result.contains(".hero"));
        assert!(result.contains(".base"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_stylesheet_cache_reuse() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", ".hero { color: red; }");

        let mut cache = StylesheetCache::new();

        // First load.
        let css1 = cache.get_or_load("/css/style.css", dist);
        assert!(css1.is_some());

        // Second load should hit cache (even if file is deleted).
        fs::remove_file(dist.join("css/style.css")).unwrap();
        let css2 = cache.get_or_load("/css/style.css", dist);
        assert!(css2.is_some());
        assert_eq!(css1, css2);
    }
}
```

- [ ] **Step 2: Run all critical_css module tests**

Run: `cargo test --lib build::critical_css -- --nocapture`

Expected: All tests pass across all three files (extract, rewrite, mod).

- [ ] **Step 3: Commit**

```bash
git add src/build/critical_css/mod.rs
git commit -m "feat(critical-css): implement orchestrator with StylesheetCache

Add inline_critical_css public entry point that coordinates stylesheet
loading (with caching), critical CSS extraction, size limit checking,
and HTML rewriting. Infallible by design -- all errors fall back to
returning the original HTML."
```

---

## Task 6: Integrate into the build pipeline

**Depends on:** Task 5 (needs the full critical_css module)

**Files:**
- Modify: `src/build/render.rs`

This task wires `inline_critical_css` into the production build pipeline, placing it after plugin `post_render_html` hooks and before HTML minification.

- [ ] **Step 1: Add critical_css import and create StylesheetCache in `build()`**

In `src/build/render.rs`, add the import at the top (after the existing `use super::` imports):

```rust
use super::critical_css;
```

In the `build()` function, after the minification log (around line 102, after the `if config.build.minify` block), add:

```rust
    // Critical CSS cache.
    let mut css_cache = critical_css::StylesheetCache::new();
    if config.build.critical_css.enabled {
        tracing::info!("Critical CSS inlining enabled.");
    }
```

- [ ] **Step 2: Pass `css_cache` to `render_static_page` and `render_dynamic_page`**

Update the `render_static_page` call in `build()` (around line 115) to add `&mut css_cache` as a parameter:

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
                )?;
```

Update the `render_dynamic_page` call similarly (around line 133):

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
                )?;
```

- [ ] **Step 3: Update `render_static_page` signature and add critical CSS step**

Add `css_cache: &mut critical_css::StylesheetCache` parameter to the function signature.

Insert the critical CSS step between the plugin `post_render_html` call and the minification step. In `render_static_page`, after the `plugin_registry.post_render_html(...)` call (around line 303) and before the `if config.build.minify` block (around line 306):

```rust
    // Critical CSS inlining (after plugins, before minify).
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

- [ ] **Step 4: Update `render_dynamic_page` signature and add critical CSS step**

Add `css_cache: &mut critical_css::StylesheetCache` parameter to the function signature.

Insert the same critical CSS step in `render_dynamic_page`, after the `plugin_registry.post_render_html(...)` call (around line 539) and before the minification block (around line 542):

```rust
        // Critical CSS inlining (after plugins, before minify).
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

- [ ] **Step 5: Run existing build tests to verify no regressions**

Run: `cargo test --lib build::render -- --nocapture`

Expected: All existing tests pass (the config defaults to `enabled: false`, so critical CSS is a no-op).

- [ ] **Step 6: Add integration test for the full pipeline**

Add to the existing test module in `src/build/render.rs`:

```rust
    #[test]
    fn test_build_with_critical_css() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "Critical CSS Test"
base_url = "https://test.com"

[build]
fragments = false
minify = false

[build.critical_css]
enabled = true
"#,
        );

        write(
            root,
            "templates/index.html",
            r#"<!DOCTYPE html>
<html>
<head>
  <link rel="stylesheet" href="/css/style.css">
</head>
<body>
  <div class="hero">Hello World</div>
</body>
</html>"#,
        );

        write(
            root,
            "static/css/style.css",
            r#"
.hero { color: red; font-size: 2em; }
.sidebar { color: blue; }
.footer { color: gray; }
"#,
        );

        build(root).unwrap();

        let html = fs::read_to_string(root.join("dist/index.html")).unwrap();

        // Should have inlined <style> with .hero but not .sidebar or .footer.
        assert!(html.contains("<style>"));
        assert!(html.contains(".hero"));
        assert!(!html.contains(".sidebar"));
        assert!(!html.contains(".footer"));

        // Should have a preload <link> for the full stylesheet.
        assert!(html.contains(r#"rel="preload""#));
        assert!(html.contains(r#"as="style""#));
        assert!(html.contains("<noscript>"));
    }

    #[test]
    fn test_build_critical_css_disabled_by_default() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(
            root,
            "site.toml",
            r#"
[site]
name = "No Critical CSS"
base_url = "https://test.com"

[build]
fragments = false
minify = false
"#,
        );

        write(
            root,
            "templates/index.html",
            r#"<!DOCTYPE html>
<html>
<head>
  <link rel="stylesheet" href="/css/style.css">
</head>
<body>
  <div class="hero">Hello</div>
</body>
</html>"#,
        );

        write(root, "static/css/style.css", ".hero { color: red; }");

        build(root).unwrap();

        let html = fs::read_to_string(root.join("dist/index.html")).unwrap();

        // No inlined <style> -- critical CSS is disabled by default.
        assert!(!html.contains("<style>"));
        // Original <link> should be intact.
        assert!(html.contains(r#"rel="stylesheet""#));
    }
```

- [ ] **Step 7: Run the new integration tests**

Run: `cargo test --lib build::render::tests::test_build_with_critical_css -- --nocapture`

and:

Run: `cargo test --lib build::render::tests::test_build_critical_css_disabled_by_default -- --nocapture`

Expected: Both tests pass.

- [ ] **Step 8: Run the full test suite**

Run: `cargo test -- --nocapture`

Expected: All tests pass with no regressions.

- [ ] **Step 9: Commit**

```bash
git add src/build/render.rs
git commit -m "feat(critical-css): integrate into build pipeline

Wire inline_critical_css into render_static_page and
render_dynamic_page, between plugin post_render_html hooks and
HTML minification. Create StylesheetCache per-build for efficient
stylesheet reuse across pages. Feature defaults to disabled."
```

---

## Task 7: Write feature documentation

**Depends on:** Task 6 (feature must be complete)

**Files:**
- Create: `docs/critical_css.md`

- [ ] **Step 1: Write the documentation**

Create `docs/critical_css.md`:

```markdown
# Critical CSS Inlining

## What it does

Critical CSS inlining extracts the subset of CSS rules actually used on each page, inlines them in a `<style>` block in `<head>`, and defers the full stylesheet load. This eliminates render-blocking CSS, improving First Contentful Paint (FCP) and Largest Contentful Paint (LCP).

## How it works

1. After rendering, eigen finds all `<link rel="stylesheet">` tags pointing to local CSS files.
2. It parses each stylesheet with lightningcss and the rendered HTML with scraper.
3. For each CSS rule, it tests whether its selector matches any element in the HTML.
4. Matching rules (plus transitively referenced @font-face, @keyframes, and custom properties) are serialized into a `<style>` block.
5. The original `<link>` tags are rewritten to load asynchronously using the preload pattern.

## Pipeline position

```
render template -> strip fragments -> localize assets -> optimize images
  -> rewrite CSS backgrounds -> plugin post_render_html
  -> critical CSS inlining  <-- HERE
  -> minify HTML -> write to disk
```

Critical CSS runs after plugins (which may generate CSS files) and before minification (so the inlined CSS benefits from minify-html's CSS minifier).

## Configuration

In `site.toml`:

```toml
[build.critical_css]
enabled = true              # default: false (opt-in)
max_inline_size = 50000     # default: 50KB, skip inlining if exceeded
preload_full = true         # default: true, async-load full stylesheet
exclude = ["**/vendor/**"]  # glob patterns to skip
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Master switch |
| `max_inline_size` | usize | `50000` | Max bytes for inlined CSS before falling back |
| `preload_full` | bool | `true` | Keep async `<link>` for full stylesheet |
| `exclude` | Vec<String> | `[]` | Glob patterns for hrefs to skip |

## Deferred loading pattern

When `preload_full = true`, each processed `<link>` becomes:

```html
<style>/* critical CSS */</style>
<link rel="preload" href="/css/style.css" as="style"
      onload="this.onload=null;this.rel='stylesheet'">
<noscript><link rel="stylesheet" href="/css/style.css"></noscript>
```

When `preload_full = false`, the `<link>` is removed entirely (tree-shaking mode).

## What is skipped

- External stylesheets (http/https URLs)
- `<link>` tags with a `media` attribute (already non-blocking)
- Stylesheets matching `exclude` glob patterns
- Existing inline `<style>` blocks (already inline)
- Fragments (partial HTML loaded via HTMX)

## Error handling

Critical CSS is a pure optimization. All failures fall back to returning the original HTML unchanged:

- Missing CSS file: warning logged, stylesheet skipped
- CSS parse error: warning logged, stylesheet skipped
- Size limit exceeded: info logged, no inlining for that page
- HTML parse error: warning logged, original HTML returned
- Selector parse error: rule included conservatively

## Caching

A `StylesheetCache` is created per build to avoid re-reading the same CSS file for every page. The raw CSS text (after reading from disk) is cached. The CSS is re-parsed per page by lightningcss since parsing is fast and the AST contains page-specific matching state.

## Dev server

Critical CSS inlining is disabled during `eigen dev` regardless of configuration. The dev server render functions do not invoke it.

## Module structure

```
src/build/critical_css/
  mod.rs       -- public API, StylesheetCache, orchestration
  extract.rs   -- CSS parsing, rule walking, selector matching
  rewrite.rs   -- lol_html HTML mutation
```

- [ ] **Step 2: Commit**

```bash
git add docs/critical_css.md
git commit -m "docs: add critical CSS feature documentation"
```

---

## Final Verification

After completing all tasks:

- [ ] **Run the full test suite**

```bash
cargo test -- --nocapture
```

Expected: All tests pass.

- [ ] **Run a manual build with the example site (if available)**

```bash
cd example_site && cargo run -- build
```

Verify that `dist/` output is correct and no regressions.

- [ ] **Verify the feature works end-to-end**

Create a test project with critical CSS enabled and verify:
1. The `dist/` HTML contains an inlined `<style>` block with only matching rules.
2. The original `<link>` tags are rewritten to preload.
3. Unused CSS rules are not present in the inlined block.
4. The full stylesheet is still accessible at its original path.

---

## Task Summary

| Task | Description | Files | Depends On |
|------|-------------|-------|------------|
| 1 | Dependencies and config types | `Cargo.toml`, `src/config/mod.rs` | -- |
| 2 | Pseudo-selector stripping and selector matching | `src/build/critical_css/extract.rs`, `mod.rs` (stub), `rewrite.rs` (stub), `src/build/mod.rs` | 1 |
| 3 | CSS rule extraction with DOM matching | `src/build/critical_css/extract.rs` | 2 |
| 4 | HTML rewriting (inject style, rewrite links) | `src/build/critical_css/rewrite.rs` | 1 |
| 5 | Orchestrator with StylesheetCache | `src/build/critical_css/mod.rs` | 3, 4 |
| 6 | Build pipeline integration | `src/build/render.rs` | 5 |
| 7 | Feature documentation | `docs/critical_css.md` | 6 |

Tasks 3 and 4 can be done in parallel after Task 2.
