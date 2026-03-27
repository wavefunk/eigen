# Lazy Loading for Below-Fold Images Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatically add `loading="lazy" decoding="async"` to all below-fold `<img>` tags, while treating the first qualifying image (and hero/data-eager images) as eager, improving page load performance without any configuration.

**Architecture:** All changes are in two existing files. `src/assets/html_rewrite.rs` gains a `LazyLoadContext` struct for first-image tracking, helper functions (`is_qualifying_image`, `resolve_loading_attrs`, `apply_lazy_loading`), and modifications to `rewrite_img_to_picture` and `build_picture_html` to set `loading`/`decoding` attributes on both optimized (picture-wrapped) and non-optimized images. `src/build/render.rs` gains a `hero_image` parameter at each call site. No new files, no new dependencies, no new configuration types.

**Tech Stack:** Rust, lol_html (already a dependency, used for HTML element rewriting)

**Design spec:** `docs/superpowers/specs/2026-03-17-lazy-loading-design.md`

---

## File Structure

### New files to create

None. All changes are in existing files.

### Existing files to modify

| File | Change |
|------|--------|
| `src/assets/html_rewrite.rs` | Add `LazyLoadContext` struct, `is_qualifying_image`, `resolve_loading_attrs`, `apply_lazy_loading` functions; modify `optimize_and_rewrite_images` signature to accept `hero_image: Option<&str>`, remove early return on empty `variant_map`; modify `rewrite_img_to_picture` to handle lazy loading for all images; modify `build_picture_html` to include `loading`/`decoding` attributes |
| `src/assets/mod.rs` | No changes needed (re-export stays the same, signature change is transparent) |
| `src/build/render.rs` | Pass `hero_image` argument to all `optimize_and_rewrite_images` calls in `render_static_page`, `render_dynamic_page`, and `optimize_fragment_images` |

---

## Task 1: Add `LazyLoadContext` and helper functions

**Depends on:** Nothing (starting point)

**Files:**
- Modify: `src/assets/html_rewrite.rs`

This task adds the core data structures and helper functions that determine whether each image should be eager or lazy. These are internal (non-public) and will be used by later tasks.

- [ ] **Step 1: Add `LazyLoadContext` struct**

Add the following after the `type VariantMap` declaration (around line 20 of `src/assets/html_rewrite.rs`). Note: `Rc` and `RefCell` are already imported at lines 8 and 11.

```rust
/// Determines whether an image should be eagerly or lazily loaded.
///
/// Passed through the image rewriting pipeline to track first-image
/// state and hero image matching.
///
/// Must be wrapped in `Rc<RefCell<LazyLoadContext>>` for use inside
/// `lol_html` closures, which require `'static + FnMut` and cannot
/// take `&mut self` references.
struct LazyLoadContext {
    /// Path of the hero image from frontmatter, if any.
    hero_image: Option<String>,
    /// Whether the first qualifying image has been seen.
    /// Set to true after the first qualifying image is processed.
    first_seen: bool,
}

impl LazyLoadContext {
    fn new(hero_image: Option<&str>) -> Self {
        Self {
            hero_image: hero_image.map(str::to_string),
            first_seen: false,
        }
    }

    /// Determine whether this image should be eager.
    ///
    /// Returns true if the image should NOT be lazy-loaded.
    /// Side effect: consumes the first-image slot if this is the
    /// first qualifying image.
    fn is_eager(&mut self, src: &str, attrs: &[(String, String)]) -> bool {
        // Check data-eager attribute.
        if attrs.iter().any(|(k, _)| k == "data-eager") {
            return true;
        }

        // Check hero_image match.
        if let Some(ref hero) = self.hero_image {
            if src == hero {
                // Consume first-image slot too, if not consumed.
                self.first_seen = true;
                return true;
            }
        }

        // Check first-image heuristic.
        if !self.first_seen && is_qualifying_image(attrs) {
            self.first_seen = true;
            return true;
        }

        false
    }
}
```

- [ ] **Step 2: Add `is_qualifying_image` function**

Add after `LazyLoadContext`:

```rust
/// Check if an image qualifies as a potential hero/above-fold image.
///
/// Excludes decorative images and small icons. Uses the same criteria
/// as `auto_detect_hero_image` in `hints/preload.rs`, minus the
/// `loading="lazy"` check (which is not relevant here since we have
/// not set `loading` yet at this point in the pipeline).
fn is_qualifying_image(attrs: &[(String, String)]) -> bool {
    // Skip decorative images.
    for (k, v) in attrs {
        if k == "role" && v == "presentation" {
            return false;
        }
        if k == "alt" && v.is_empty() {
            return false;
        }
    }

    // Skip small icons (both dimensions < 100px).
    let width = attrs
        .iter()
        .find(|(k, _)| k == "width")
        .and_then(|(_, v)| v.parse::<u32>().ok());
    let height = attrs
        .iter()
        .find(|(k, _)| k == "height")
        .and_then(|(_, v)| v.parse::<u32>().ok());

    if let (Some(w), Some(h)) = (width, height) {
        if w < 100 && h < 100 {
            return false;
        }
    }

    // Skip data URIs.
    if let Some((_, src)) = attrs.iter().find(|(k, _)| k == "src") {
        if src.starts_with("data:") {
            return false;
        }
    }

    true
}
```

- [ ] **Step 3: Add `resolve_loading_attrs` function**

Add after `is_qualifying_image`:

```rust
/// Determine what loading/decoding attributes to set on an image.
///
/// Returns `(loading_value, decoding_value, should_remove_data_eager)`.
///
/// - `loading_value`: `None` if the attribute should not be set (explicit
///   already present and no `data-eager`), `Some("eager")` or
///   `Some("lazy")` otherwise.
/// - `decoding_value`: `None` if not set (explicit already present, or
///   image is eager), `Some("async")` for lazy images.
/// - `should_remove_data_eager`: `true` if `data-eager` was present and
///   should be stripped from output.
fn resolve_loading_attrs(
    src: &str,
    attrs: &[(String, String)],
    ctx: &mut LazyLoadContext,
) -> (Option<&'static str>, Option<&'static str>, bool) {
    let has_data_eager = attrs.iter().any(|(k, _)| k == "data-eager");
    let has_explicit_loading = attrs.iter().any(|(k, _)| k == "loading");
    let has_explicit_decoding = attrs.iter().any(|(k, _)| k == "decoding");

    // Log conflicting data-eager + loading="lazy".
    if has_data_eager && has_explicit_loading {
        let loading_val = attrs.iter().find(|(k, _)| k == "loading").map(|(_, v)| v.as_str());
        if loading_val == Some("lazy") {
            tracing::debug!(
                "Image has both data-eager and loading=\"lazy\"; honoring data-eager: {}",
                src,
            );
        }
    }

    let eager = ctx.is_eager(src, attrs);

    // Determine loading attribute.
    let loading = if has_data_eager {
        // data-eager always forces eager, overriding any explicit loading.
        Some("eager")
    } else if has_explicit_loading {
        // Explicit loading attribute: do not override.
        None
    } else if eager {
        Some("eager")
    } else {
        Some("lazy")
    };

    // Determine decoding attribute.
    let decoding = if has_explicit_decoding {
        None
    } else if loading == Some("lazy") {
        Some("async")
    } else {
        // Eager or explicit: do not set decoding (let browser decide).
        None
    };

    (loading, decoding, has_data_eager)
}
```

- [ ] **Step 4: Write unit tests for `LazyLoadContext` and helpers**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `src/assets/html_rewrite.rs`:

```rust
    // --- LazyLoadContext tests ---

    #[test]
    fn test_lazy_context_first_image_is_eager() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        assert!(ctx.is_eager("/assets/photo.jpg", &attrs));
        assert!(ctx.first_seen);
    }

    #[test]
    fn test_lazy_context_second_image_is_lazy() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs1 = vec![
            ("src".to_string(), "/assets/first.jpg".to_string()),
            ("alt".to_string(), "First".to_string()),
        ];
        let attrs2 = vec![
            ("src".to_string(), "/assets/second.jpg".to_string()),
            ("alt".to_string(), "Second".to_string()),
        ];
        assert!(ctx.is_eager("/assets/first.jpg", &attrs1));
        assert!(!ctx.is_eager("/assets/second.jpg", &attrs2));
    }

    #[test]
    fn test_lazy_context_hero_image_is_eager() {
        let mut ctx = LazyLoadContext::new(Some("/assets/hero.jpg"));
        let attrs = vec![
            ("src".to_string(), "/assets/hero.jpg".to_string()),
            ("alt".to_string(), "Hero".to_string()),
        ];
        assert!(ctx.is_eager("/assets/hero.jpg", &attrs));
        assert!(ctx.first_seen); // hero consumes first-image slot
    }

    #[test]
    fn test_lazy_context_data_eager_does_not_consume_first() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs_eager = vec![
            ("src".to_string(), "/assets/promo.jpg".to_string()),
            ("alt".to_string(), "Promo".to_string()),
            ("data-eager".to_string(), String::new()),
        ];
        let attrs_first = vec![
            ("src".to_string(), "/assets/first.jpg".to_string()),
            ("alt".to_string(), "First".to_string()),
        ];
        // data-eager returns true but does NOT consume first-image slot.
        assert!(ctx.is_eager("/assets/promo.jpg", &attrs_eager));
        assert!(!ctx.first_seen);
        // The actual first qualifying image should still be eager.
        assert!(ctx.is_eager("/assets/first.jpg", &attrs_first));
        assert!(ctx.first_seen);
    }

    #[test]
    fn test_is_qualifying_image_normal() {
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        assert!(is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_decorative_alt() {
        let attrs = vec![
            ("src".to_string(), "/assets/divider.png".to_string()),
            ("alt".to_string(), String::new()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_decorative_role() {
        let attrs = vec![
            ("src".to_string(), "/assets/bg.jpg".to_string()),
            ("role".to_string(), "presentation".to_string()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_small_icon() {
        let attrs = vec![
            ("src".to_string(), "/assets/icon.png".to_string()),
            ("alt".to_string(), "Icon".to_string()),
            ("width".to_string(), "32".to_string()),
            ("height".to_string(), "32".to_string()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_data_uri() {
        let attrs = vec![
            ("src".to_string(), "data:image/png;base64,abc".to_string()),
            ("alt".to_string(), "Inline".to_string()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_large_dimensions() {
        let attrs = vec![
            ("src".to_string(), "/assets/banner.jpg".to_string()),
            ("alt".to_string(), "Banner".to_string()),
            ("width".to_string(), "800".to_string()),
            ("height".to_string(), "400".to_string()),
        ];
        assert!(is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_one_dimension_large() {
        // Only both < 100 triggers skip; one dimension >= 100 is fine.
        let attrs = vec![
            ("src".to_string(), "/assets/strip.jpg".to_string()),
            ("alt".to_string(), "Strip".to_string()),
            ("width".to_string(), "200".to_string()),
            ("height".to_string(), "50".to_string()),
        ];
        assert!(is_qualifying_image(&attrs));
    }

    #[test]
    fn test_resolve_loading_attrs_lazy() {
        let mut ctx = LazyLoadContext::new(None);
        // Consume first-image slot.
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        let (loading, decoding, remove) = resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("lazy"));
        assert_eq!(decoding, Some("async"));
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_first_eager() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        let (loading, decoding, remove) = resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("eager"));
        assert_eq!(decoding, None); // eager images don't get decoding="async"
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_data_eager() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/promo.jpg".to_string()),
            ("alt".to_string(), "Promo".to_string()),
            ("data-eager".to_string(), String::new()),
        ];
        let (loading, decoding, remove) = resolve_loading_attrs("/assets/promo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("eager"));
        assert_eq!(decoding, None);
        assert!(remove); // data-eager should be stripped
    }

    #[test]
    fn test_resolve_loading_attrs_explicit_preserved() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
            ("loading".to_string(), "eager".to_string()),
        ];
        let (loading, decoding, remove) = resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, None); // explicit: do not override
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_explicit_decoding_preserved() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
            ("decoding".to_string(), "sync".to_string()),
        ];
        let (loading, decoding, remove) = resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("lazy"));
        assert_eq!(decoding, None); // explicit decoding preserved
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_data_eager_overrides_lazy() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
            ("loading".to_string(), "lazy".to_string()),
            ("data-eager".to_string(), String::new()),
        ];
        let (loading, decoding, remove) = resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("eager")); // data-eager wins
        assert!(remove);
    }
```

- [ ] **Step 5: Run tests to verify helpers compile and pass**

Run: `cargo test --lib assets::html_rewrite -- --nocapture`

Expected: All existing tests still pass, plus the new helper tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/assets/html_rewrite.rs
git commit -m "feat(lazy-loading): add LazyLoadContext and helper functions

Add LazyLoadContext struct for tracking first-image state across
the lol_html streaming pass. Add is_qualifying_image (consistent
criteria with hints/preload.rs) and resolve_loading_attrs for
attribute precedence logic."
```

---

## Task 2: Add `apply_lazy_loading` fallback function

**Depends on:** Task 1 (needs LazyLoadContext and helpers)

**Files:**
- Modify: `src/assets/html_rewrite.rs`

This task adds the standalone `apply_lazy_loading` function used when image optimization is disabled (`config.optimize = false`). It is an internal function called only from `optimize_and_rewrite_images`.

- [ ] **Step 1: Add `apply_lazy_loading` function**

Add after `resolve_loading_attrs`, before `optimize_and_rewrite_images`:

```rust
/// Apply lazy loading attributes to all `<img>` tags.
///
/// This is the fallback path used when image optimization is disabled.
/// When optimization IS enabled, lazy loading is applied inline during
/// the img -> picture rewrite and this function is not called.
///
/// Sets `loading="lazy" decoding="async"` on all images except:
/// - The first qualifying image (gets `loading="eager"`)
/// - Images with `data-eager` attribute (gets `loading="eager"`)
/// - Images matching `hero_image` (gets `loading="eager"`)
/// - Images with an explicit `loading` attribute (left unchanged)
fn apply_lazy_loading(html: &str, hero_image: Option<&str>) -> Result<String> {
    let ctx = Rc::new(RefCell::new(LazyLoadContext::new(hero_image)));
    let ctx_clone = ctx.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img[src]", move |el| {
                let src = match el.get_attribute("src") {
                    Some(s) => s,
                    None => return Ok(()),
                };

                // Collect attributes for the resolver.
                let attrs: Vec<(String, String)> = el
                    .attributes()
                    .iter()
                    .map(|a| (a.name(), a.value()))
                    .collect();

                let (loading, decoding, remove_data_eager) =
                    resolve_loading_attrs(&src, &attrs, &mut ctx_clone.borrow_mut());

                if let Some(val) = loading {
                    // AttributeNameError implements Error + Send + Sync,
                    // so ? auto-boxes into HandlerResult's Box<dyn Error>.
                    el.set_attribute("loading", val)?;
                }
                if let Some(val) = decoding {
                    el.set_attribute("decoding", val)?;
                }
                if remove_data_eager {
                    el.remove_attribute("data-eager");
                }

                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| eyre::eyre!("lol_html error applying lazy loading: {}", e))?;

    Ok(output)
}
```

- [ ] **Step 2: Write unit tests for `apply_lazy_loading`**

Add to the `#[cfg(test)] mod tests` block:

```rust
    // --- apply_lazy_loading tests ---

    #[test]
    fn test_apply_lazy_loading_default() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B"><img src="/c.jpg" alt="C">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // First image: eager (no loading="lazy").
        assert!(result.contains(r#"src="/a.jpg""#));
        assert!(result.contains(r#"loading="eager""#));
        // Second and third: lazy.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"decoding="async""#));
    }

    #[test]
    fn test_apply_lazy_loading_no_images() {
        let html = "<div><p>Hello</p></div>";
        let result = apply_lazy_loading(html, None).unwrap();
        assert_eq!(result, html);
    }

    #[test]
    fn test_apply_lazy_loading_hero_image() {
        let html = r#"<img src="/b.jpg" alt="B"><img src="/hero.jpg" alt="Hero"><img src="/c.jpg" alt="C">"#;
        let result = apply_lazy_loading(html, Some("/hero.jpg")).unwrap();
        // /b.jpg is first qualifying: eager.
        // /hero.jpg matches hero_image: also eager.
        // /c.jpg: lazy.
        assert!(result.contains(r#"src="/hero.jpg""#));
        // Count lazy occurrences: only the last image.
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(lazy_count, 1);
    }

    #[test]
    fn test_apply_lazy_loading_data_eager_stripped() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B" data-eager>"#;
        let result = apply_lazy_loading(html, None).unwrap();
        assert!(!result.contains("data-eager"));
    }

    #[test]
    fn test_apply_lazy_loading_explicit_loading_preserved() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B" loading="eager">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // Second image has explicit loading="eager": preserved.
        // The function should not add a second loading attribute.
        let eager_count = result.matches(r#"loading="eager""#).count();
        assert_eq!(eager_count, 2); // first (auto) + second (explicit)
    }

    #[test]
    fn test_apply_lazy_loading_all_decorative() {
        let html = r#"<img src="/a.png" alt=""><img src="/b.png" alt="">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // All decorative: all lazy, none eager.
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(lazy_count, 2);
    }

    #[test]
    fn test_apply_lazy_loading_external_images() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="https://cdn.example.com/photo.jpg" alt="External">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // External images still get lazy attributes.
        assert!(result.contains(r#"loading="lazy""#));
    }

    #[test]
    fn test_apply_lazy_loading_first_skips_decorative() {
        let html = r#"<img src="/decorative.png" alt=""><img src="/real.jpg" alt="Real"><img src="/other.jpg" alt="Other">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // Decorative image: lazy.
        // /real.jpg is first qualifying: eager.
        // /other.jpg: lazy.
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(lazy_count, 2);
        let eager_count = result.matches(r#"loading="eager""#).count();
        assert_eq!(eager_count, 1);
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib assets::html_rewrite -- --nocapture`

Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/assets/html_rewrite.rs
git commit -m "feat(lazy-loading): add apply_lazy_loading fallback function

Standalone lol_html pass that sets loading/decoding attributes on
img tags. Used when image optimization is disabled but lazy loading
should still be applied."
```

---

## Task 3: Modify `build_picture_html` to include loading/decoding attributes

**Depends on:** Task 1 (needs `resolve_loading_attrs`)

**Files:**
- Modify: `src/assets/html_rewrite.rs`

This task modifies the `build_picture_html` function so that when an `<img>` is rewritten to `<picture>`, the inner `<img>` gets the correct `loading` and `decoding` attributes. It also handles stripping `data-eager` from the output attributes.

- [ ] **Step 1: Change `build_picture_html` signature and implementation**

The current `build_picture_html` signature (line 187):

```rust
fn build_picture_html(
    attrs: &[(String, String)],
    variants: &ImageVariants,
) -> String {
```

Change to:

```rust
fn build_picture_html(
    attrs: &[(String, String)],
    variants: &ImageVariants,
    loading: Option<&str>,
    decoding: Option<&str>,
    strip_data_eager: bool,
) -> String {
```

In the fallback `<img>` construction section (currently lines 218-226), replace:

```rust
    // Build the fallback <img> with all original attributes preserved.
    html.push_str("  <img");

    for (name, value) in attrs {
        html.push_str(&format!(" {}=\"{}\"", name, escape_attr(value)));
    }

    html.push_str(">\n</picture>");
```

With:

```rust
    // Build the fallback <img> with all original attributes preserved.
    html.push_str("  <img");

    for (name, value) in attrs {
        // Strip data-eager: it is a build-time signal, not valid HTML.
        if strip_data_eager && name == "data-eager" {
            continue;
        }
        // If we are setting loading/decoding, skip the original attribute
        // so we emit ours instead (data-eager overrides explicit loading).
        if name == "loading" && loading.is_some() {
            continue;
        }
        if name == "decoding" && decoding.is_some() {
            continue;
        }
        html.push_str(&format!(" {}=\"{}\"", name, escape_attr(value)));
    }

    // Append loading and decoding attributes.
    if let Some(val) = loading {
        html.push_str(&format!(" loading=\"{}\"", val));
    }
    if let Some(val) = decoding {
        html.push_str(&format!(" decoding=\"{}\"", val));
    }

    html.push_str(">\n</picture>");
```

- [ ] **Step 2: Update the call site in `rewrite_img_to_picture`**

The current call to `build_picture_html` inside `rewrite_img_to_picture` (line 169):

```rust
let picture_html = build_picture_html(&attrs, variants);
```

This will be updated in Task 4 when the full `rewrite_img_to_picture` changes are made. For now, temporarily pass the new parameters as no-ops to keep the code compiling:

```rust
let picture_html = build_picture_html(&attrs, variants, None, None, false);
```

- [ ] **Step 3: Update existing test `test_build_picture_html_structure`**

The existing test at line 426 calls `rewrite_img_to_picture(html, &map)` which will now use the temporary no-op parameters. No test changes are needed yet -- the test verifies the `<picture>` structure which is unchanged.

Verify that all existing tests still pass.

- [ ] **Step 4: Run tests**

Run: `cargo test --lib assets::html_rewrite -- --nocapture`

Expected: All tests pass (build_picture_html emits same output as before with `None, None, false`).

- [ ] **Step 5: Commit**

```bash
git add src/assets/html_rewrite.rs
git commit -m "feat(lazy-loading): extend build_picture_html with loading/decoding params

Add loading, decoding, and strip_data_eager parameters to
build_picture_html. When set, these attributes are injected into the
inner <img> of the <picture> element, and data-eager is stripped."
```

---

## Task 4: Modify `rewrite_img_to_picture` to handle lazy loading for all images

**Depends on:** Task 1, Task 3 (needs helpers and updated `build_picture_html`)

**Files:**
- Modify: `src/assets/html_rewrite.rs`

This is the core change: the `rewrite_img_to_picture` function gains a `hero_image` parameter, creates a `LazyLoadContext`, and applies lazy loading to both optimized images (via `build_picture_html`) and non-optimized images (via `el.set_attribute`).

- [ ] **Step 1: Update `rewrite_img_to_picture` signature**

Change the signature from:

```rust
fn rewrite_img_to_picture(html: &str, variant_map: &VariantMap) -> Result<String> {
```

To:

```rust
fn rewrite_img_to_picture(
    html: &str,
    variant_map: &VariantMap,
    hero_image: Option<&str>,
) -> Result<String> {
```

- [ ] **Step 2: Add `LazyLoadContext` to the closure**

Replace the closure body (currently lines 144-182) with:

```rust
fn rewrite_img_to_picture(
    html: &str,
    variant_map: &VariantMap,
    hero_image: Option<&str>,
) -> Result<String> {
    // We need to clone data into the closure.
    let map = variant_map.clone();
    let ctx = Rc::new(RefCell::new(LazyLoadContext::new(hero_image)));
    let ctx_clone = ctx.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img[src]", move |el| {
                let src = match el.get_attribute("src") {
                    Some(s) => s,
                    None => return Ok(()),
                };

                // Collect attributes for both picture building and lazy loading.
                let attrs: Vec<(String, String)> = el
                    .attributes()
                    .iter()
                    .map(|a| (a.name(), a.value()))
                    .collect();

                // Resolve loading/decoding for this image.
                let (loading, decoding, strip_data_eager) =
                    resolve_loading_attrs(&src, &attrs, &mut ctx_clone.borrow_mut());

                match map.get(&src) {
                    Some(variants) => {
                        // Has variants: build <picture> replacement with loading attrs.
                        let picture_html = build_picture_html(
                            &attrs,
                            variants,
                            loading,
                            decoding,
                            strip_data_eager,
                        );
                        el.replace(&picture_html, lol_html::html_content::ContentType::Html);
                    }
                    None => {
                        // No variants: set loading/decoding in-place via lol_html.
                        // AttributeNameError implements Error + Send + Sync,
                        // so ? auto-boxes into HandlerResult's Box<dyn Error>.
                        if let Some(val) = loading {
                            el.set_attribute("loading", val)?;
                        }
                        if let Some(val) = decoding {
                            el.set_attribute("decoding", val)?;
                        }
                        if strip_data_eager {
                            el.remove_attribute("data-eager");
                        }
                    }
                }

                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| eyre::eyre!("lol_html error rewriting images: {}", e))?;

    Ok(output)
}
```

- [ ] **Step 3: Update existing tests that call `rewrite_img_to_picture` directly**

The following tests call `rewrite_img_to_picture` directly and need the third argument added:

- `test_build_picture_html_structure` (line 474): `rewrite_img_to_picture(html, &map)` -> `rewrite_img_to_picture(html, &map, None)`. After Task 4, the inner `<img>` in the generated `<picture>` will also have `loading="eager"` (it is the first qualifying image). Add an assertion:
    ```rust
    assert!(result.contains(r#"loading="eager""#));
    ```
- `test_rewrite_leaves_unmatched_imgs` (line 492): `rewrite_img_to_picture(html, &map)` -> `rewrite_img_to_picture(html, &map, None)`
  - **Also update the comment and assertion:** The unmatched image will now get `loading="eager"` added (it is the first qualifying image). Change the comment from "Should be unchanged" to "Should still contain original attrs but now also has loading=\"eager\"". The existing `contains` assertion still passes (substring match), but add an additional assertion:
    ```rust
    assert!(result.contains(r#"loading="eager""#));
    ```
- `test_rewrite_preserves_non_img_html` (line 501): `rewrite_img_to_picture(html, &map)` -> `rewrite_img_to_picture(html, &map, None)`

- [ ] **Step 4: Add new tests for lazy loading in the rewrite path**

Add to the `#[cfg(test)] mod tests` block:

```rust
    // --- rewrite_img_to_picture lazy loading tests ---

    #[test]
    fn test_rewrite_non_optimized_gets_lazy() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B">"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        // First image: eager.
        assert!(result.contains(r#"loading="eager""#));
        // Second image: lazy.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"decoding="async""#));
    }

    #[test]
    fn test_rewrite_svg_gets_lazy() {
        let html = r#"<img src="/first.jpg" alt="First"><img src="/icon.svg" alt="SVG icon">"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        // SVG (no variants) still gets lazy loading.
        assert!(result.contains(r#"loading="lazy""#));
    }

    #[test]
    fn test_rewrite_data_eager_removed_non_optimized() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B" data-eager>"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        assert!(!result.contains("data-eager"));
        // Both should be eager (first by heuristic, second by data-eager).
        let eager_count = result.matches(r#"loading="eager""#).count();
        assert_eq!(eager_count, 2);
    }

    #[test]
    fn test_rewrite_empty_variant_map_still_applies_lazy() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B"><img src="/c.jpg" alt="C">"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        // Even with empty variant_map, lazy loading should be applied.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"loading="eager""#));
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib assets::html_rewrite -- --nocapture`

Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/assets/html_rewrite.rs
git commit -m "feat(lazy-loading): integrate lazy loading into rewrite_img_to_picture

The lol_html closure now uses LazyLoadContext to set loading/decoding
attributes on every <img>: both optimized images (via build_picture_html)
and non-optimized images (via el.set_attribute). data-eager is stripped."
```

---

## Task 5: Update `optimize_and_rewrite_images` signature and remove early return

**Depends on:** Task 2, Task 4 (needs `apply_lazy_loading` and updated `rewrite_img_to_picture`)

**Files:**
- Modify: `src/assets/html_rewrite.rs`

This task changes the public API of `optimize_and_rewrite_images` to accept `hero_image`, removes the early return on empty `variant_map`, and routes the optimization-disabled path through `apply_lazy_loading`.

- [ ] **Step 1: Update `optimize_and_rewrite_images` signature**

Change (currently lines 32-37):

```rust
pub fn optimize_and_rewrite_images(
    html: &str,
    config: &ImageOptimConfig,
    cache: &ImageCache,
    dist_dir: &Path,
) -> Result<String> {
```

To:

```rust
pub fn optimize_and_rewrite_images(
    html: &str,
    config: &ImageOptimConfig,
    cache: &ImageCache,
    dist_dir: &Path,
    hero_image: Option<&str>,
) -> Result<String> {
```

- [ ] **Step 2: Update the optimization-disabled early return**

Change (currently lines 38-40):

```rust
    if !config.optimize {
        return Ok(html.to_string());
    }
```

To:

```rust
    if !config.optimize {
        // Still apply lazy loading even without optimization.
        return apply_lazy_loading(html, hero_image);
    }
```

- [ ] **Step 3: Remove the empty `variant_map` early return**

Remove (currently lines 97-99):

```rust
    if variant_map.is_empty() {
        return Ok(html.to_string());
    }
```

This ensures the `lol_html` pass always runs, so non-optimized images still get `loading`/`decoding` attributes via the modified `rewrite_img_to_picture` closure.

- [ ] **Step 4: Pass `hero_image` to `rewrite_img_to_picture`**

Change the call at the end of `optimize_and_rewrite_images` (currently line 102):

```rust
    rewrite_img_to_picture(html, &variant_map)
```

To:

```rust
    rewrite_img_to_picture(html, &variant_map, hero_image)
```

- [ ] **Step 5: Update existing tests that call `optimize_and_rewrite_images`**

The following tests call `optimize_and_rewrite_images` and need the `hero_image` argument added:

- `test_full_optimize_and_rewrite` (line 507): add `None` as the last argument. **Note:** after this change the inner `<img>` in the generated `<picture>` will also have `loading="eager"` since it is the first qualifying image. Existing assertions still pass (they test for `<picture>`, srcset, format, alt, class) but verify the output also contains `loading="eager"`.
- `test_optimize_disabled` (line 544): add `None` as the last argument. **ALSO** update the assertion: the result is no longer identical to the input because `apply_lazy_loading` adds `loading="eager"`. Change:

```rust
assert_eq!(result, html);
```

To:

```rust
// Optimization is off but lazy loading is still applied.
assert!(result.contains(r#"loading="eager""#));
assert!(result.contains(r#"src="/assets/photo.jpg""#));
assert!(!result.contains("<picture>"));
```

- `test_excludes_svg_and_gif` (line 560): add `None` as the last argument. **ALSO** update the assertions. With lazy loading now applied, SVGs/GIFs will have `loading` attributes even though they are not wrapped in `<picture>`. The test currently only asserts no `<picture>`, which remains true. But verify no regression:

```rust
let result = optimize_and_rewrite_images(html, &config, &cache, &dist_dir, None).unwrap();
// Both should remain as plain <img> — no <picture> wrapping.
assert!(!result.contains("<picture>"));
// But they should have lazy loading attributes.
// First image (logo.svg) gets eager, second (anim.gif) gets lazy.
assert!(result.contains(r#"loading="eager""#));
assert!(result.contains(r#"loading="lazy""#));
```

- [ ] **Step 6: Add test for full pipeline with lazy loading**

Add to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn test_full_optimize_with_lazy_loading() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist_dir = tmp.path().join("dist");
        std::fs::create_dir_all(dist_dir.join("assets")).unwrap();

        // Create a 200x100 blue JPEG.
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 100, |_, _| {
            image::Rgb([0, 0, 255])
        }));
        let src_path = dist_dir.join("assets/hero.jpg");
        img.save(&src_path).unwrap();

        let config = ImageOptimConfig {
            optimize: true,
            formats: vec!["webp".to_string()],
            quality: 75,
            widths: vec![100],
            exclude: vec![],
        };

        let cache = ImageCache::open(tmp.path()).unwrap();

        // Two images: first has variants, second does not.
        let html = r#"<html><body><img src="/assets/hero.jpg" alt="Hero"><img src="/assets/other.jpg" alt="Other"></body></html>"#;

        let result = optimize_and_rewrite_images(html, &config, &cache, &dist_dir, None).unwrap();

        // First image (hero.jpg) is wrapped in <picture> and is eager.
        assert!(result.contains("<picture>"));
        assert!(result.contains(r#"loading="eager""#));
        // Second image (other.jpg) has no variants, remains <img>, gets lazy.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"decoding="async""#));
    }

    #[test]
    fn test_optimize_with_hero_image_frontmatter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist_dir = tmp.path().join("dist");
        std::fs::create_dir_all(dist_dir.join("assets")).unwrap();

        let config = ImageOptimConfig {
            optimize: true,
            formats: vec![],
            quality: 75,
            widths: vec![],
            exclude: vec![],
        };

        let cache = ImageCache::open(tmp.path()).unwrap();

        let html = r#"<img src="/assets/a.jpg" alt="A"><img src="/assets/hero.jpg" alt="Hero"><img src="/assets/c.jpg" alt="C">"#;

        let result = optimize_and_rewrite_images(
            html, &config, &cache, &dist_dir,
            Some("/assets/hero.jpg"),
        ).unwrap();

        // /assets/a.jpg: first qualifying image, eager.
        // /assets/hero.jpg: matches hero_image, eager.
        // /assets/c.jpg: lazy.
        let eager_count = result.matches(r#"loading="eager""#).count();
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(eager_count, 2);
        assert_eq!(lazy_count, 1);
    }
```

- [ ] **Step 7: Run tests**

Run: `cargo test --lib assets::html_rewrite -- --nocapture`

Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/assets/html_rewrite.rs
git commit -m "feat(lazy-loading): update optimize_and_rewrite_images public API

Add hero_image parameter. Route optimization-disabled path through
apply_lazy_loading. Remove empty variant_map early return so lazy
loading is applied even when no images have variants."
```

---

## Task 6: Update call sites in `render.rs`

**Depends on:** Task 5 (public API change)

**Files:**
- Modify: `src/build/render.rs`

This task updates all call sites of `optimize_and_rewrite_images` and `optimize_fragment_images` in the render pipeline to pass the `hero_image` parameter.

- [ ] **Step 1: Update `render_static_page` call site**

In `render_static_page` (around line 331), change:

```rust
    let full_html = assets::optimize_and_rewrite_images(
        &full_html,
        &config.assets.images,
        image_cache,
        dist_dir,
    ).wrap_err_with(|| format!("Failed to optimize images for '{}'", tmpl_name))?;
```

To:

```rust
    let full_html = assets::optimize_and_rewrite_images(
        &full_html,
        &config.assets.images,
        image_cache,
        dist_dir,
        page.frontmatter.hero_image.as_deref(),
    ).wrap_err_with(|| format!("Failed to optimize images for '{}'", tmpl_name))?;
```

- [ ] **Step 2: Update `render_dynamic_page` call site**

In `render_dynamic_page` (around line 591), change:

```rust
        let full_html = assets::optimize_and_rewrite_images(
            &full_html,
            &config.assets.images,
            image_cache,
            dist_dir,
        ).wrap_err_with(|| {
            format!("Failed to optimize images for '{}' slug '{}'", tmpl_name, slug)
        })?;
```

To:

```rust
        let full_html = assets::optimize_and_rewrite_images(
            &full_html,
            &config.assets.images,
            image_cache,
            dist_dir,
            page.frontmatter.hero_image.as_deref(),
        ).wrap_err_with(|| {
            format!("Failed to optimize images for '{}' slug '{}'", tmpl_name, slug)
        })?;
```

- [ ] **Step 3: Update `optimize_fragment_images` function**

In `optimize_fragment_images` (around line 753), change the function body's call to `optimize_and_rewrite_images`:

```rust
        let optimized_html = assets::optimize_and_rewrite_images(
            &frag.html,
            image_config,
            image_cache,
            dist_dir,
        )?;
```

To:

```rust
        let optimized_html = assets::optimize_and_rewrite_images(
            &frag.html,
            image_config,
            image_cache,
            dist_dir,
            None, // No hero image for fragments.
        )?;
```

- [ ] **Step 4: Build and verify**

Run: `cargo build`

Expected: Compiles without errors. All call sites pass the correct number of arguments.

- [ ] **Step 5: Run full test suite**

Run: `cargo test`

Expected: All tests pass, including the existing integration tests.

- [ ] **Step 6: Commit**

```bash
git add src/build/render.rs
git commit -m "feat(lazy-loading): pass hero_image through render pipeline

Update optimize_and_rewrite_images calls in render_static_page and
render_dynamic_page to pass page.frontmatter.hero_image.as_deref().
Fragments pass None (hero image is a page-level concept)."
```

---

## Task 7: Write feature documentation

**Depends on:** Task 6 (all code changes complete)

**Files:**
- Create: `docs/lazy_loading.md`

Per project convention, create documentation for the feature.

- [ ] **Step 1: Write feature documentation**

Create `docs/lazy_loading.md` with:
- Overview of what the feature does.
- How first-image detection works.
- How `data-eager`, `hero_image`, and explicit `loading` attributes interact.
- Behavior when image optimization is disabled.
- Fragment behavior.
- Interaction with preload hints.

- [ ] **Step 2: Commit**

```bash
git add docs/lazy_loading.md
git commit -m "docs: add lazy loading feature documentation

Covers first-image detection, data-eager opt-out, hero_image
interaction, fragment behavior, and preload hints consistency."
```

---

## Task 8: Final verification and cleanup

**Depends on:** Task 7 (all tasks complete)

**Files:**
- All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo test`

Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`

Expected: No warnings.

- [ ] **Step 3: Run `/simplify`**

Per project convention, run `/simplify` to review the new code for readability and simplification opportunities before final commit.

- [ ] **Step 4: Verify build with a test site (manual)**

If a test site is available, run `just build` (or equivalent) and inspect the generated HTML:
- Verify first `<img>` on a page has `loading="eager"`.
- Verify subsequent `<img>` tags have `loading="lazy" decoding="async"`.
- Verify `<picture>` elements' inner `<img>` tags have correct attributes.
- Verify `data-eager` is stripped from output.
- Verify preload hints still correctly detect the hero image.

- [ ] **Step 5: Final commit if any simplification changes were made**

```bash
git add -A
git commit -m "refactor(lazy-loading): simplify after review"
```

---

## Summary

| Task | Description | Files | Est. Steps |
|------|-------------|-------|-----------|
| 1 | Add `LazyLoadContext` and helper functions | `html_rewrite.rs` | 6 |
| 2 | Add `apply_lazy_loading` fallback function | `html_rewrite.rs` | 4 |
| 3 | Modify `build_picture_html` for loading/decoding | `html_rewrite.rs` | 5 |
| 4 | Modify `rewrite_img_to_picture` for lazy loading | `html_rewrite.rs` | 6 |
| 5 | Update `optimize_and_rewrite_images` public API | `html_rewrite.rs` | 8 |
| 6 | Update call sites in `render.rs` | `render.rs` | 6 |
| 7 | Write feature documentation | `docs/lazy_loading.md` | 2 |
| 8 | Final verification and cleanup | All | 5 |
| **Total** | | | **42** |

### Dependency Graph

```
Task 1 (helpers)
  ├─> Task 2 (apply_lazy_loading)  ──┐
  └─> Task 3 (build_picture_html) ──┤
                                     ├─> Task 4 (rewrite_img_to_picture)
                                     │     └─> Task 5 (public API) ──> Task 6 (render.rs)
                                     │                                    └─> Task 7 (docs)
                                     │                                          └─> Task 8 (verify)
```

Tasks 2 and 3 can be done in parallel. All other tasks are sequential.
