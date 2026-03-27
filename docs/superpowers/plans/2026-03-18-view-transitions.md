# View Transitions API Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Inject View Transitions API meta tag, HTMX config script, and `view-transition-name` styles into rendered pages so HTMX partial swaps animate smoothly.

**Architecture:** New `build::view_transitions` module using `lol_html` to inject into `<head>` and rewrite element `style` attributes. Follows the same infallible injection pattern as `build::seo`. Config under `[build.view_transitions]` with `enabled: false` default.

**Tech Stack:** Rust, lol_html (already in Cargo.toml), regex (already in Cargo.toml)

**Spec:** `docs/superpowers/specs/2026-03-18-view-transitions-design.md`

---

### Task 1: Add ViewTransitionsConfig to site config

**Files:**
- Modify: `src/config/mod.rs:52-98` (BuildConfig struct and Default impl)

- [ ] **Step 1: Write the failing test**

Add at the end of the `#[cfg(test)]` block in `src/config/mod.rs`:

```rust
#[test]
fn test_view_transitions_default_disabled() {
    let toml_str = r#"
        [site]
        name = "Test"
        base_url = "https://example.com"
    "#;
    let config: SiteConfig = toml::from_str(toml_str).unwrap();
    assert!(!config.build.view_transitions.enabled);
}

#[test]
fn test_view_transitions_enabled() {
    let toml_str = r#"
        [site]
        name = "Test"
        base_url = "https://example.com"
        [build.view_transitions]
        enabled = true
    "#;
    let config: SiteConfig = toml::from_str(toml_str).unwrap();
    assert!(config.build.view_transitions.enabled);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_view_transitions -- --nocapture`
Expected: compilation error — `view_transitions` field does not exist on `BuildConfig`

- [ ] **Step 3: Write the ViewTransitionsConfig struct and wire it in**

Add the struct after the existing config structs (e.g., after `BundlingConfig`):

```rust
/// View Transitions API configuration.
///
/// When enabled, injects a `<meta name="view-transition">` tag and an
/// inline script that enables `htmx.config.globalViewTransitions` for
/// smooth animated transitions between HTMX partial swaps.
#[derive(Debug, Clone, Deserialize)]
pub struct ViewTransitionsConfig {
    /// Whether to inject view transition meta tag, script, and
    /// transition names into rendered pages.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for ViewTransitionsConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}
```

Add field to `BuildConfig` (after `bundling`):

```rust
    /// View Transitions API configuration.
    #[serde(default)]
    pub view_transitions: ViewTransitionsConfig,
```

Add to `BuildConfig::default()`:

```rust
    view_transitions: ViewTransitionsConfig::default(),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_view_transitions -- --nocapture`
Expected: both tests PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: all tests pass (no breakage from new field — `#[serde(default)]` handles existing configs, `..Default::default()` handles test helpers)

- [ ] **Step 6: Commit**

```
feat(config): add ViewTransitionsConfig to build config
```

---

### Task 2: Create view_transitions module with core injection logic

**Files:**
- Create: `src/build/view_transitions.rs`
- Modify: `src/build/mod.rs` (add `pub mod view_transitions;`)

- [ ] **Step 1: Add module declaration**

In `src/build/mod.rs`, add `pub mod view_transitions;` in alphabetical order (after `pub mod sitemap;`).

- [ ] **Step 2: Write the failing tests**

Create `src/build/view_transitions.rs` with tests first:

```rust
//! View Transitions API injection.
//!
//! Injects a `<meta name="view-transition">` tag, an inline script that
//! enables `htmx.config.globalViewTransitions`, and `view-transition-name`
//! styles on elements whose `id` matches a fragment block name.
//!
//! This makes HTMX partial swaps animate smoothly via the browser's
//! View Transitions API. Progressive enhancement: browsers without
//! support get instant swaps as before.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Inject view transition meta tag, HTMX config script, and
/// transition names on fragment target elements.
///
/// `fragment_block_names` is the list of block names extracted from
/// the page's fragment markers. Elements with matching `id`
/// attributes get `view-transition-name` added to their `style`.
///
/// Infallible by design: returns original HTML on any error.
pub fn inject_view_transitions(
    html: &str,
    fragment_block_names: &[String],
) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_meta_tag() {
        let html = "<html><head><title>Test</title></head><body></body></html>";
        let result = inject_view_transitions(html, &[]);
        assert!(
            result.contains(r#"<meta name="view-transition" content="same-origin">"#),
            "Should inject view-transition meta tag"
        );
    }

    #[test]
    fn test_inject_script() {
        let html = "<html><head><title>Test</title></head><body></body></html>";
        let result = inject_view_transitions(html, &[]);
        assert!(
            result.contains("htmx.config.globalViewTransitions"),
            "Should inject HTMX globalViewTransitions script"
        );
        assert!(
            result.contains("DOMContentLoaded"),
            "Should use DOMContentLoaded to wait for HTMX"
        );
        assert!(
            result.contains("document.startViewTransition"),
            "Should guard with startViewTransition check"
        );
    }

    #[test]
    fn test_inject_transition_names() {
        let html = r#"<html><head></head><body><div id="content">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains(r#"style="view-transition-name: content;""#),
            "Should add view-transition-name to element with matching id. Got: {}", result
        );
    }

    #[test]
    fn test_existing_inline_style() {
        let html = r#"<html><head></head><body><div id="content" style="color: red;">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains("color: red; view-transition-name: content;"),
            "Should append to existing style. Got: {}", result
        );
    }

    #[test]
    fn test_existing_inline_style_trailing_semicolon() {
        let html = r#"<html><head></head><body><div id="content" style="color: red;">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            !result.contains(";;"),
            "Should not produce double semicolons. Got: {}", result
        );
    }

    #[test]
    fn test_existing_view_transition_name() {
        let html = r#"<html><head></head><body><div id="content" style="view-transition-name: custom;">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains("view-transition-name: custom;"),
            "Should preserve existing view-transition-name"
        );
        // Should not have two view-transition-name declarations.
        let count = result.matches("view-transition-name").count();
        assert_eq!(count, 1, "Should not duplicate view-transition-name. Got: {}", result);
    }

    #[test]
    fn test_no_head() {
        let html = "<div>Fragment content</div>";
        let result = inject_view_transitions(html, &["content".into()]);
        assert_eq!(result, html, "Should return unchanged when no <head>");
    }

    #[test]
    fn test_no_matching_ids() {
        let html = r#"<html><head></head><body><div id="other">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains(r#"<meta name="view-transition""#),
            "Meta tag should still be injected"
        );
        assert!(
            !result.contains("view-transition-name"),
            "No transition name should be added when no IDs match"
        );
    }

    #[test]
    fn test_multiple_fragment_names() {
        let html = r#"<html><head></head><body><div id="content">Main</div><nav id="nav_header">Nav</nav></body></html>"#;
        let result = inject_view_transitions(
            html,
            &["content".into(), "nav_header".into()],
        );
        assert!(
            result.contains(r#"view-transition-name: content;"#),
            "Should add transition name to content. Got: {}", result
        );
        assert!(
            result.contains(r#"view-transition-name: nav_header;"#),
            "Should add transition name to nav_header. Got: {}", result
        );
    }

    #[test]
    fn test_existing_meta_tag() {
        let html = r#"<html><head><meta name="view-transition" content="same-origin"></head><body></body></html>"#;
        let result = inject_view_transitions(html, &[]);
        let count = result.matches(r#"<meta name="view-transition""#).count();
        assert_eq!(count, 1, "Should not duplicate meta tag. Got: {}", result);
    }

    #[test]
    fn test_fragments_disabled() {
        let html = r#"<html><head></head><body><div id="content">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &[]);
        assert!(
            result.contains(r#"<meta name="view-transition""#),
            "Meta tag should be injected even without block names"
        );
        assert!(
            result.contains("globalViewTransitions"),
            "Script should be injected even without block names"
        );
        assert!(
            !result.contains("view-transition-name"),
            "No transition names when no block names provided"
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test view_transitions -- --nocapture`
Expected: FAIL — `todo!()` panics

- [ ] **Step 4: Implement `has_view_transition_meta`**

Add above the public function:

```rust
/// Check if the HTML already contains a `<meta name="view-transition">` tag.
///
/// Uses lol_html selector matching, following the convention in
/// `seo::has_canonical_link` and `json_ld::has_existing_json_ld`.
fn has_view_transition_meta(html: &str) -> bool {
    let found: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_clone = found.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!(
                "meta[name='view-transition']",
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

- [ ] **Step 5: Implement `build_head_injection`**

```rust
/// The meta tag for cross-document view transitions.
const VIEW_TRANSITION_META: &str =
    r#"<meta name="view-transition" content="same-origin">"#;

/// The inline script that enables HTMX's built-in view transitions.
///
/// Waits for DOMContentLoaded to ensure HTMX is loaded. Guards on
/// both `htmx` existing and `document.startViewTransition` being
/// supported (progressive enhancement).
const VIEW_TRANSITION_SCRIPT: &str = r#"<script>
document.addEventListener("DOMContentLoaded", function() {
  if (typeof htmx !== "undefined" && document.startViewTransition) {
    htmx.config.globalViewTransitions = true;
  }
});
</script>"#;

/// Build the HTML to inject into `<head>`.
fn build_head_injection(has_meta: bool) -> String {
    let mut out = String::new();
    if !has_meta {
        out.push_str(VIEW_TRANSITION_META);
        out.push('\n');
    }
    out.push_str(VIEW_TRANSITION_SCRIPT);
    out.push('\n');
    out
}
```

- [ ] **Step 6: Implement `rewrite_html`**

```rust
/// Inject content into `<head>` and add `view-transition-name` to
/// elements whose `id` matches a fragment block name.
fn rewrite_html(
    html: &str,
    head_html: &str,
    block_names: &HashSet<String>,
) -> Result<String, lol_html::errors::RewritingError> {
    let head_owned = head_html.to_string();
    let names = block_names.clone();

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                // Inject meta + script into <head>.
                lol_html::element!("head", move |el| {
                    el.append(
                        &head_owned,
                        lol_html::html_content::ContentType::Html,
                    );
                    Ok(())
                }),
                // Add view-transition-name to elements with matching IDs.
                lol_html::element!("*[id]", move |el| {
                    if let Some(id) = el.get_attribute("id") {
                        if names.contains(&id) {
                            let existing_style =
                                el.get_attribute("style").unwrap_or_default();

                            // Skip if already has a view-transition-name.
                            if existing_style.contains("view-transition-name") {
                                return Ok(());
                            }

                            let new_style = if existing_style.is_empty() {
                                format!("view-transition-name: {};", id)
                            } else {
                                let trimmed =
                                    existing_style.trim_end().trim_end_matches(';');
                                format!(
                                    "{}; view-transition-name: {};",
                                    trimmed, id
                                )
                            };

                            if let Err(e) = el.set_attribute("style", &new_style) {
                                tracing::warn!(
                                    "Failed to set style on #{}: {}",
                                    id, e
                                );
                            }
                        }
                    }
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    )
}
```

- [ ] **Step 7: Implement the public `inject_view_transitions` function**

Replace the `todo!()` body:

```rust
pub fn inject_view_transitions(
    html: &str,
    fragment_block_names: &[String],
) -> String {
    let has_meta = has_view_transition_meta(html);
    let head_html = build_head_injection(has_meta);

    let block_names: HashSet<String> =
        fragment_block_names.iter().cloned().collect();

    match rewrite_html(html, &head_html, &block_names) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("Failed to inject view transitions: {}", e);
            html.to_string()
        }
    }
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test view_transitions -- --nocapture`
Expected: all 10 tests PASS

- [ ] **Step 9: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 10: Commit**

```
feat(view-transitions): add build::view_transitions module with injection logic
```

---

### Task 3: Wire view transitions into the build pipeline

**Files:**
- Modify: `src/build/fragments.rs` (add `extract_block_names` public function)
- Modify: `src/build/render.rs:383-396` (static page, add block name extraction)
- Modify: `src/build/render.rs:462-476` (static page, add injection step)
- Modify: `src/build/render.rs:674-687` (dynamic page, add block name extraction)
- Modify: `src/build/render.rs:761-775` (dynamic page, add injection step)

- [ ] **Step 1: Add `extract_block_names` to `fragments.rs`**

Add this public function to `src/build/fragments.rs` (after `strip_fragment_markers`). This keeps the fragment marker regex in one place rather than duplicating it in `render.rs`:

```rust
/// Extract fragment block names from rendered HTML (before marker stripping).
///
/// Returns the block names found in `<!--FRAG:name:START-->` markers.
/// Used by view transitions to know which element IDs to add
/// `view-transition-name` to.
pub fn extract_block_names(html: &str) -> Vec<String> {
    let re = Regex::new(r"<!--FRAG:(\w+):START-->").unwrap();
    re.captures_iter(html)
        .map(|cap| cap[1].to_string())
        .collect()
}
```

- [ ] **Step 2: Wire into `render_static_page`**

After the template render (line ~383 `let rendered = ...`) and before marker stripping (line ~396 `let full_html = fragments::strip_fragment_markers`), add:

```rust
    // Extract fragment block names (before marker stripping) for view transitions.
    let block_names = if config.build.view_transitions.enabled {
        fragments::extract_block_names(&rendered)
    } else {
        Vec::new()
    };
```

After the JSON-LD injection step (line ~469) and before the minify step, add:

```rust
    // 4g. View transitions injection (after JSON-LD, before minify).
    let full_html = if config.build.view_transitions.enabled {
        view_transitions::inject_view_transitions(&full_html, &block_names)
    } else {
        full_html
    };
```

Update the minify step comment number to `4h`.

- [ ] **Step 3: Wire into `render_dynamic_page`**

Same pattern inside the per-item loop.

After the template render (line ~674 `let rendered = ...`) and before marker stripping, add:

```rust
        // Extract fragment block names (before marker stripping) for view transitions.
        let block_names = if config.build.view_transitions.enabled {
            fragments::extract_block_names(&rendered)
        } else {
            Vec::new()
        };
```

After the JSON-LD injection step (line ~768) and before the minify step, add:

```rust
        // View transitions injection (after JSON-LD, before minify).
        let full_html = if config.build.view_transitions.enabled {
            view_transitions::inject_view_transitions(&full_html, &block_names)
        } else {
            full_html
        };
```

- [ ] **Step 4: Add the imports**

At the top of `render.rs`, add the `view_transitions` module import (matching existing import style) and `regex` for the block name extraction:

```rust
use super::view_transitions;
```

(Or if `render.rs` uses `crate::build::` style imports, match that. No `regex` import needed — the regex lives in `fragments.rs`.)

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings

- [ ] **Step 7: Commit**

```
feat(view-transitions): wire injection into build pipeline
```

---

### Task 4: Write feature documentation

**Files:**
- Create: `docs/view_transitions.md`

- [ ] **Step 1: Write the docs**

```markdown
# View Transitions API

Eigen can inject the View Transitions API into your pages, making HTMX
partial swaps animate smoothly instead of instantly replacing content.

## Enabling

```toml
[build.view_transitions]
enabled = true
```

## What It Does

When enabled, three things are injected into every full page:

### 1. Cross-Document Transitions Meta Tag

```html
<meta name="view-transition" content="same-origin">
```

Enables the browser's native view transitions for full page navigations
(initial load, non-HTMX links, browser back/forward).

### 2. HTMX Integration Script

A small inline script that sets `htmx.config.globalViewTransitions = true`.
This makes HTMX wrap all partial swaps in `document.startViewTransition()`,
giving you animated transitions between pages.

### 3. Transition Names on Fragment Targets

Elements whose `id` matches a fragment block name (e.g., `content`,
`sidebar`) automatically get `view-transition-name` added. This lets
the browser animate each region independently instead of a whole-page
cross-fade.

## Progressive Enhancement

Browsers without View Transitions support (currently Firefox) get the
same instant swaps as before. No polyfill is loaded.

## Custom Animations

The browser's default transition is a cross-fade. To customize, add
CSS rules targeting the `::view-transition-*` pseudo-elements:

```css
::view-transition-old(content) {
  animation: slide-out 0.2s ease-in;
}

::view-transition-new(content) {
  animation: slide-in 0.2s ease-out;
}
```

The transition names match your fragment block names (`content`,
`sidebar`, etc.), so you can target each region independently.

## Overriding Transition Names

If you set `view-transition-name` on an element in your own CSS or
inline styles, eigen will not overwrite it.
```

- [ ] **Step 2: Commit**

```
docs: add view transitions feature documentation
```

---

### Task 5: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no warnings

- [ ] **Step 3: Test with example site (if available)**

Run: `just build` or `cargo run -- build` with a site.toml that has:

```toml
[build.view_transitions]
enabled = true
```

Verify the output HTML contains:
- `<meta name="view-transition" content="same-origin">` in `<head>`
- The `globalViewTransitions` script in `<head>`
- `view-transition-name` on elements with fragment block IDs
