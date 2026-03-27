# Lazy Loading for Below-Fold Images -- Design Spec

Date: 2026-03-17

## Motivation and Goals

The browser eagerly loads every `<img>` on a page by default, even images
that are thousands of pixels below the fold. On image-heavy pages (blogs
with inline photos, product listings, galleries), this wastes bandwidth,
delays Time to Interactive (TTI), and competes with above-fold resources
for network and CPU time.

The `loading="lazy"` attribute tells the browser to defer fetching an
image until it is near the viewport. The `decoding="async"` attribute
tells the browser it may decode the image off the main thread, avoiding
jank during scrolling. Together, these two attributes are the
lowest-effort, highest-impact improvement for pages with multiple images.

Eigen already rewrites `<img>` tags into `<picture>` elements with
responsive `<source srcset>` during the build. Adding `loading` and
`decoding` attributes at the same time is a natural extension of this
existing pipeline step -- no new HTML parsing pass is needed for
optimized images. Non-optimized images require the lol_html pass to
be run regardless, as described in "Non-optimized Images" below.

**Goals:**

- Default all images to `loading="lazy" decoding="async"`.
- Automatically treat the first qualifying image on each page as eager
  (above-fold), matching the heuristic that browsers and Lighthouse
  recommend.
- Respect the existing `hero_image` frontmatter field: if a hero image is
  declared, it is always eager.
- Allow template authors to opt out of lazy loading on any image via a
  `data-eager` attribute.
- Preserve any explicit `loading` or `decoding` attribute already present
  on an `<img>` tag (do not override author intent).
- Apply to fragments as well as full pages.
- Integrate into the existing `img -> picture` rewrite where possible,
  falling back to a standalone lol_html pass when the optimization path
  is not taken.

**Non-goals (see "Out of Scope"):**

- Lazy loading for CSS `background-image`.
- Intersection Observer polyfills or JavaScript-based lazy loading.
- Lazy loading for `<video>` or `<iframe>`.

## Architecture

### Approach: Inline in the Existing `img -> picture` Rewrite

The function `rewrite_img_to_picture` in `src/assets/html_rewrite.rs`
already iterates over every `<img>` element using `lol_html`, collects
its attributes, and builds the replacement `<picture>` HTML string via
`build_picture_html`. This is the natural place to inject `loading` and
`decoding` attributes because:

1. We already have access to all attributes on the original `<img>`.
2. We are already constructing the replacement `<img>` inside `<picture>`.
3. No additional HTML parsing pass is needed for optimized images.
4. Images that are NOT optimized (no variants) still enter the
   `lol_html` element handler -- the closure currently returns `Ok(())`
   for them. We modify the closure to set attributes on these images
   in-place via `el.set_attribute()`.

**Why not a separate `lol_html` pass?** For the optimization-enabled
path, a separate pass would parse the entire HTML document a second time
just to set two attributes. Since we are already inside a
`lol_html::rewrite_str` call that touches every `<img>`, doing the work
inline avoids the overhead and keeps the logic co-located with the
`<picture>` construction.

**Why not in the hints module?** The hints module (`build/hints/`) deals
with `<link>` tags in `<head>`, not with `<img>` attributes in `<body>`.
Lazy loading is an image-rendering concern, not a resource-hint concern.
Keeping it in `assets/html_rewrite.rs` follows the principle of
co-locating related transformations.

### Pipeline Position

The current per-page pipeline order (from `render_static_page` and
`render_dynamic_page` in `src/build/render.rs`):

```
render template
  -> strip fragment markers
  -> localize assets
  -> optimize images (img -> picture)   <-- lazy loading applied HERE
  -> rewrite CSS background images
  -> plugin post_render_html
  -> critical CSS inlining
  -> preload/prefetch hints
  -> minify HTML
  -> write to disk
```

Content hash rewriting is a separate post-build phase (Phase 3 in
`build()`) that runs after ALL pages have been rendered. It is not
part of the per-page pipeline.

Lazy loading is applied during the "optimize images" step because that is
where `<img>` tags are rewritten to `<picture>` elements. The `loading`
and `decoding` attributes are set on the inner `<img>` of the `<picture>`
element.

**Critical ordering with hints:** The preload/prefetch hints step runs
AFTER image optimization. The `auto_detect_hero_image` function in
`hints/preload.rs` already skips images with `loading="lazy"`. This means
the auto-detection will naturally find the first eager image (the first
image on the page, or the hero image), which is the correct behavior. The
ordering is already correct and no changes to the hints module are needed.

### Non-optimized Images

Images that are not optimized (external URLs, excluded patterns, data
URIs, images with no variants) pass through `rewrite_img_to_picture`
unchanged -- the closure returns `Ok(())` without modifying the element.
These images still need `loading`/`decoding` attributes.

There are three sub-cases:

1. **Images that enter `rewrite_img_to_picture` and have variants:**
   These are rewritten to `<picture>` elements. The `loading` and
   `decoding` attributes are included in the replacement HTML built by
   `build_picture_html`.

2. **Images that enter `rewrite_img_to_picture` but have no variants:**
   The `lol_html` element handler already has access to these elements.
   Instead of returning `Ok(())` immediately, we set attributes on
   them in-place using `el.set_attribute()`. This handles the common
   case (local images that could not be decoded, excluded images,
   external URLs).

3. **The `rewrite_img_to_picture` pass is skipped entirely:** This
   happens when `variant_map` is empty (no images on the page had
   variants produced). Currently, `optimize_and_rewrite_images` returns
   the HTML unchanged and never calls `rewrite_img_to_picture`. To
   handle lazy loading in this case, we must ensure the lol_html pass
   is still run. The fix: remove the early return on empty `variant_map`
   and always call `rewrite_img_to_picture`, which now also handles
   lazy loading for all images regardless of whether they have variants.

4. **Image optimization is disabled (`config.optimize = false`):**
   `optimize_and_rewrite_images` currently returns `html.to_string()`
   immediately without invoking `lol_html` at all. We redirect this
   path to `apply_lazy_loading`, a lightweight `lol_html::rewrite_str`
   that only sets `loading` and `decoding` on `<img>` tags without
   doing any `<picture>` conversion.

We introduce a new internal function `apply_lazy_loading` that handles
case 4. Case 3 is handled by removing the empty variant_map early
return. Cases 1 and 2 are handled inside the existing
`rewrite_img_to_picture` closure.

### First-Image Detection

The first qualifying image on a page should be eager (not lazy) because
it is most likely above the fold and is often the LCP element. Marking it
lazy would harm Largest Contentful Paint.

**Definition of "first qualifying image":** The first `<img>` encountered
in document order that:

- Is not `role="presentation"` or `alt=""`  (decorative)
- Does not have `width` AND `height` both < 100  (small icon)
- Is not a data URI

This is similar to the criteria used in `auto_detect_hero_image` in
`hints/preload.rs`, keeping the two systems consistent. Note that
`auto_detect_hero_image` additionally skips images with `loading="lazy"`,
which is not relevant here since we are the ones setting `loading` -- at
the point `is_qualifying_image` runs, no `loading` attribute has been
set yet.

**Implementation:** A `LazyLoadContext` struct tracks whether the first
qualifying image has been seen. Since `lol_html` closures require shared
ownership, the struct is wrapped in `Rc<RefCell<LazyLoadContext>>`. The
first qualifying image gets `loading="eager"`. All subsequent images get
`loading="lazy"`.

**Interaction with `hero_image` frontmatter:** If a `hero_image` path is
specified in frontmatter, any `<img>` whose `src` matches that path is
always treated as eager, regardless of its position in the document. This
is checked before the first-image counter, so if the hero image happens
to be the first image, it correctly gets eager treatment and the counter
is still consumed.

**Interaction with `data-eager`:** If an `<img>` has a `data-eager`
attribute, it is always treated as eager. The `data-eager` attribute is
removed from the output (it is a build-time signal, not a valid HTML
attribute). This does NOT consume the first-image counter -- only the
actual first qualifying image in document order consumes it.

### Attribute Precedence

When deciding what `loading` and `decoding` values to set on an `<img>`:

1. **Explicit `loading` attribute already present AND no `data-eager`:**
   Do not override. Template authors who write `loading="eager"` or
   `loading="lazy"` explicitly have expressed intent.

2. **`data-eager` attribute present:** Set `loading="eager"`. Remove
   `data-eager` from output. This overrides any explicit `loading`
   attribute because `data-eager` is a build-time directive that
   explicitly opts into eager loading. If `loading="lazy"` is also
   present alongside `data-eager`, the template likely has a conflict;
   log a debug message and honor `data-eager`.

3. **`src` matches `hero_image` from frontmatter:** Set `loading="eager"`.

4. **First qualifying image on the page (counter not consumed):** Set
   `loading="eager"`. Consume the counter.

5. **All other images:** Set `loading="lazy"`.

For `decoding`:

1. **Explicit `decoding` attribute already present:** Do not override.
2. **Image is eager (rules 2-4 above):** Do not set `decoding` (let the
   browser decide; eager images should decode synchronously for LCP).
3. **Image is lazy:** Set `decoding="async"`.

**Rationale for not setting `decoding="async"` on eager images:**
`decoding="async"` can cause a brief blank frame before the image
renders. For LCP/hero images, this is undesirable. The browser's default
decoding behavior (`auto`) is better for above-fold images. For
below-fold images, `async` is safe because the user has not scrolled
there yet.

### Fragments

Fragments get their own image optimization pass via
`optimize_fragment_images` in `build/render.rs`. This calls
`optimize_and_rewrite_images` on each fragment's HTML independently.

For fragments, the first-image heuristic should still apply within the
fragment's own HTML, but the semantics are different: a fragment is loaded
into an already-rendered page via HTMX, so it is "above the fold" from
the user's perspective (they just navigated to it). However, images below
the visible portion of the fragment content are still below-fold.

**Decision:** Apply the same first-image-is-eager heuristic to fragments.
The first image in each fragment is treated as eager, the rest as lazy.
This is a reasonable approximation: the first image in a content fragment
is likely visible immediately after the HTMX swap.

## Data Models and Types

### Configuration

No new configuration struct is needed. Lazy loading is always enabled
(there is no scenario where you want images but NOT lazy loading). The
opt-out mechanism is per-image via `data-eager`, not site-wide.

This follows the principle that the right default should not require
configuration. Every modern browser supports `loading="lazy"`, and the
first-image-is-eager heuristic prevents LCP regressions.

When image optimization is disabled (`assets.images.optimize = false`),
lazy loading is still applied via the `apply_lazy_loading` fallback
function, because lazy loading is valuable even without responsive image
variants. The only way to disable lazy loading entirely is to set
`loading="eager"` on individual images in templates.

### Eagerness Context

A small struct wrapped in `Rc<RefCell<...>>` and shared across the
`lol_html` element handler closure to track first-image state:

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
    let width = attrs.iter()
        .find(|(k, _)| k == "width")
        .and_then(|(_, v)| v.parse::<u32>().ok());
    let height = attrs.iter()
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

### Updated Function Signatures

The `optimize_and_rewrite_images` function signature changes to accept
an optional hero image path:

```rust
pub fn optimize_and_rewrite_images(
    html: &str,
    config: &ImageOptimConfig,
    cache: &ImageCache,
    dist_dir: &Path,
    hero_image: Option<&str>,  // NEW
) -> Result<String>
```

The `hero_image` parameter is `Option<&str>` rather than a reference to
the `Frontmatter` struct, keeping the function decoupled from the
frontmatter module. The caller in `render.rs` passes
`page.frontmatter.hero_image.as_deref()`.

The `rewrite_img_to_picture` internal function also gains the
`hero_image` parameter (passed through to `LazyLoadContext`):

```rust
fn rewrite_img_to_picture(
    html: &str,
    variant_map: &VariantMap,
    hero_image: Option<&str>,  // NEW
) -> Result<String>
```

### Internal Function: `apply_lazy_loading`

```rust
/// Apply lazy loading attributes to all <img> tags.
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
fn apply_lazy_loading(
    html: &str,
    hero_image: Option<&str>,
) -> Result<String>
```

Note: `apply_lazy_loading` is internal (`fn`, not `pub fn`). It is only
called from `optimize_and_rewrite_images` when `config.optimize` is
false. Callers always go through `optimize_and_rewrite_images`.

## API Surface

### Public Functions (changes)

**`optimize_and_rewrite_images`** -- gains a `hero_image: Option<&str>`
parameter. All existing call sites in `render.rs` and
`optimize_fragment_images` pass the hero image path.

The function's internal structure changes:

```rust
pub fn optimize_and_rewrite_images(
    html: &str,
    config: &ImageOptimConfig,
    cache: &ImageCache,
    dist_dir: &Path,
    hero_image: Option<&str>,
) -> Result<String> {
    if !config.optimize {
        // Still apply lazy loading even without optimization.
        return apply_lazy_loading(html, hero_image);
    }

    // Phase 1: Collect image srcs and optimize (unchanged).
    let img_srcs = collect_img_srcs(html)?;
    let mut variant_map: VariantMap = HashMap::new();
    // ... existing optimization logic ...

    // Phase 2: Rewrite HTML -- always run, even with empty variant_map.
    // Previously this returned early when variant_map was empty, but now
    // we must run the lol_html pass to set loading/decoding attributes
    // on all images.
    rewrite_img_to_picture(html, &variant_map, hero_image)
}
```

**Key change from current code:** The existing early return at
`if variant_map.is_empty() { return Ok(html.to_string()); }` (line 97
of `html_rewrite.rs`) is removed. The `rewrite_img_to_picture` pass now
always runs so that images without variants still get lazy loading
attributes set via `el.set_attribute()`.

This means callers do not change their call pattern at all (just add the
hero_image argument). No new public function needs to be called from
`render.rs`.

### Internal Functions (new)

```rust
/// Determine whether an image should be eagerly loaded and what
/// attributes to set.
///
/// Returns (loading_value, decoding_value, should_remove_data_eager).
fn resolve_loading_attrs(
    src: &str,
    attrs: &[(String, String)],
    ctx: &mut LazyLoadContext,
) -> (Option<&'static str>, Option<&'static str>, bool)
```

This function encapsulates the attribute precedence logic from the
"Attribute Precedence" section above. It returns:

- `loading_value`: `None` if the attribute should not be set (explicit
  already present and no `data-eager`), `Some("eager")` or
  `Some("lazy")` otherwise.
- `decoding_value`: `None` if not set (explicit already present, or
  image is eager), `Some("async")` for lazy images.
- `should_remove_data_eager`: `true` if `data-eager` was present and
  should be stripped from output.

### Template-Level API

Template authors interact with this feature through:

1. **`data-eager` attribute on `<img>`:**
   ```html
   <img src="/assets/promo.jpg" data-eager alt="Promo banner">
   ```
   This image will always be eagerly loaded, regardless of its position.
   The `data-eager` attribute is stripped from the output HTML.

2. **Explicit `loading` attribute:**
   ```html
   <img src="/assets/photo.jpg" loading="eager" alt="Photo">
   ```
   Preserved as-is. Eigen does not override explicit attributes (unless
   `data-eager` is also present, in which case `data-eager` wins).

3. **`hero_image` in frontmatter:**
   ```yaml
   ---
   hero_image: /assets/hero-banner.jpg
   ---
   ```
   Already exists. The image matching this path is always eager.

4. **No annotation (default):** The first qualifying image is eager.
   All other images are lazy with `decoding="async"`.

## Error Handling

| Scenario | Behavior |
|---|---|
| `lol_html` rewrite error in `apply_lazy_loading` | Propagate error via `Result` (same as existing `optimize_and_rewrite_images`) |
| `data-eager` attribute with a value (e.g., `data-eager="true"`) | Treat as eager regardless of value. The attribute's presence is what matters, not its value. |
| `loading` attribute with invalid value (e.g., `loading="fast"`) | Preserve as-is. Not our job to validate HTML attribute values. |
| Image with both `data-eager` and `loading="lazy"` | `data-eager` wins. Set `loading="eager"`, strip `data-eager`. Log a debug message about the conflicting attributes. |
| Hero image path in frontmatter does not match any `<img>` | No effect on lazy loading. The hero image check simply does not match, and first-image heuristic applies normally. |
| All images on a page are decorative (no qualifying image) | All images get `loading="lazy" decoding="async"`. No image gets eager treatment. This is correct: if every image is decorative, none is the LCP element. |
| Page has zero `<img>` tags | No-op. Return HTML unchanged. |
| External images (http/https src) | Still get `loading`/`decoding` attributes. Lazy loading works for external images too. |

**Principle:** Lazy loading is additive and non-destructive. It adds
attributes that browsers handle gracefully. There are no failure modes
that should break the build.

## Edge Cases and Failure Modes

### SVG and GIF images (excluded from optimization)

SVG and GIF images are excluded from responsive image optimization by
default via `default_image_exclude()` in `config/mod.rs` (patterns
`**/*.svg` and `**/*.gif`). They remain as `<img>` tags, not wrapped
in `<picture>`, because `optimize_image` is never called for them and
they have no entries in `variant_map`.

They still get `loading`/`decoding` attributes because the
`rewrite_img_to_picture` closure now handles images without variants
(case 2 in "Non-optimized Images"). The closure detects that no
variants exist for their `src`, and instead of replacing the element
with `<picture>`, it sets `loading`/`decoding` attributes in-place
via `el.set_attribute()`. This is correct: SVGs and GIFs below the
fold should still be lazily loaded.

### Images inside `<noscript>`

The `loading="lazy"` attribute is a native browser feature and does
NOT require JavaScript. It works when JavaScript is disabled.
Therefore, setting `loading="lazy"` on images inside `<noscript>` is
harmless and correct -- the browser will still defer loading until
the image nears the viewport.

Since `lol_html` processes elements in document order without tracking
nesting context, images inside `<noscript>` are processed identically
to all other images. This is acceptable because `loading="lazy"` is a
browser-native feature that works without JS.

### Images in `<picture>` elements not created by eigen

If the template already contains `<picture>` elements with inner `<img>`
tags, the `lol_html` handler for `img[src]` will process those inner
`<img>` tags too. This is correct: those images should also get lazy
loading attributes.

### `srcset` on `<img>` (not inside `<picture>`)

Some templates may use `<img srcset="...">` directly without `<picture>`.
These are processed the same way -- `loading`/`decoding` attributes are
added based on the `src` attribute matching logic.

### Multiple images with the same `src`

If the same image URL appears multiple times (e.g., a thumbnail repeated
in a grid), only the first occurrence is treated as eager. Subsequent
occurrences get `loading="lazy"`. This is correct behavior.

### CSS `background-image`

CSS background images are not affected by this feature. The
`loading` attribute is an HTML `<img>` attribute and has no CSS
equivalent. Lazy loading CSS backgrounds would require Intersection
Observer JavaScript, which is out of scope.

### Interaction with content hashing

Content hashing is a post-build phase (Phase 3 in `build()`) that runs
via `content_hash::rewrite_references(&dist_dir, &manifest)` after ALL
pages have been rendered and written to disk. It rewrites `src` URLs in
the already-written HTML files. Since lazy loading is applied during the
per-page image optimization step (well before Phase 3), the `hero_image`
match is against the pre-hashed URL. This is correct because the
`hero_image` value in frontmatter uses the original, unhashed path.

### Interaction with preload hints

The preload hints module auto-detects the hero image by finding the first
`<img>` without `loading="lazy"`. Since our lazy loading step runs before
the hints step, and we set the first qualifying image to eager (no
`loading="lazy"`), the auto-detection will correctly identify the same
image that we marked as eager. The two systems are naturally consistent.

If `hero_image` is set in frontmatter, the hints module uses that
directly and does not rely on auto-detection. Our lazy loading also
treats the frontmatter hero image as eager. Consistent.

### Dev server

The dev server (`src/dev/rebuild.rs`) has its own render functions that
do NOT call `optimize_and_rewrite_images` (confirmed: no grep hits for
`optimize_and_rewrite_images` or `optimize_fragment` in `src/dev/`).
Images in dev mode will not have lazy loading attributes. This is
acceptable: lazy loading is a production optimization, and dev mode
prioritizes rebuild speed.

### Fragment-specific behavior

Fragments are partial HTML snippets loaded via HTMX. When a fragment is
loaded, the browser will encounter its images for the first time. The
first image in the fragment should be eager because it is likely visible
immediately after the swap.

The `optimize_fragment_images` function in `render.rs` already calls
`optimize_and_rewrite_images` for each fragment independently. Each
fragment gets its own `LazyLoadContext` with `first_seen: false`, so
the first qualifying image in each fragment is treated as eager.

For fragments, `hero_image` from frontmatter is not passed (pass `None`).
The hero image is a page-level concept, not a fragment-level concept.

## Configuration Examples

No new configuration is needed. Lazy loading is automatic.

### Default behavior (no config changes)

```toml
[assets.images]
optimize = true   # default
```

All `<img>` tags get `loading="lazy" decoding="async"` except the first
qualifying image on each page (which gets `loading="eager"`).

### Opt-out for specific images in templates

```html
<!-- This image is always eagerly loaded -->
<img src="/assets/promo-banner.jpg" data-eager alt="Promo">

<!-- This image uses explicit loading, eigen does not touch it -->
<img src="/assets/photo.jpg" loading="eager" alt="Photo">
```

### Hero image via frontmatter

```yaml
---
hero_image: /assets/hero-banner.jpg
---
```

The image with `src="/assets/hero-banner.jpg"` is always eager.

### Image optimization disabled but lazy loading still applies

```toml
[assets.images]
optimize = false
```

Images remain as `<img>` (no `<picture>` wrapping), but `loading="lazy"`
and `decoding="async"` are still added via the `apply_lazy_loading`
fallback path.

## What Is NOT In Scope

1. **Lazy loading for CSS `background-image`.** This would require
   JavaScript (Intersection Observer) which contradicts eigen's static-
   site, no-JS-required philosophy. The `loading` attribute is only
   valid on `<img>` and `<iframe>`.

2. **Lazy loading for `<video>` and `<iframe>`.** While `<iframe>`
   supports `loading="lazy"`, eigen does not currently process these
   elements. This can be added as a separate feature later.

3. **JavaScript-based lazy loading polyfills.** Native `loading="lazy"`
   has >95% browser support (caniuse). Polyfills add complexity for
   negligible benefit.

4. **Site-level configuration to disable lazy loading.** There is no
   legitimate reason to disable it globally. Per-image opt-out via
   `data-eager` or `loading="eager"` covers all valid use cases.

5. **Fade-in or placeholder effects.** These are CSS/JS presentation
   concerns, not build pipeline concerns.

6. **`fetchpriority` attribute.** While related to loading priority,
   `fetchpriority` is an optimization for the hints/preload system, not
   for lazy loading. It could be added to the hints module later.

7. **Lazy loading `<source>` elements inside `<picture>`.** The
   `loading` attribute is set on the inner `<img>`, which controls
   loading for the entire `<picture>` element. Setting it on `<source>`
   is not valid HTML.

## Module Structure

No new files. All changes are in existing files:

```
src/assets/
  html_rewrite.rs  -- add LazyLoadContext, is_qualifying_image,
                      resolve_loading_attrs, apply_lazy_loading;
                      modify rewrite_img_to_picture to handle lazy
                      loading for both optimized and non-optimized
                      images; modify build_picture_html to include
                      loading/decoding attrs; remove early return
                      on empty variant_map
```

Estimated change size:
- `html_rewrite.rs`: ~130 lines added (LazyLoadContext, resolve logic,
  apply_lazy_loading function, attribute setting in build_picture_html,
  non-optimized image handling in rewrite_img_to_picture closure)
- `render.rs`: ~10 lines changed (pass hero_image to calls)

This is a small, focused change. No new modules, no new dependencies,
no new configuration types.

## Integration Points

### build/render.rs

Update calls to `optimize_and_rewrite_images` to pass the hero image:

**In `render_static_page`:**
```rust
let full_html = assets::optimize_and_rewrite_images(
    &full_html,
    &config.assets.images,
    image_cache,
    dist_dir,
    page.frontmatter.hero_image.as_deref(),  // NEW
).wrap_err_with(|| format!("Failed to optimize images for '{}'", tmpl_name))?;
```

**In `render_dynamic_page`:**
```rust
let full_html = assets::optimize_and_rewrite_images(
    &full_html,
    &config.assets.images,
    image_cache,
    dist_dir,
    page.frontmatter.hero_image.as_deref(),  // NEW
).wrap_err_with(|| ...)?;
```

**In `optimize_fragment_images`:**
```rust
let optimized_html = assets::optimize_and_rewrite_images(
    &frag.html,
    image_config,
    image_cache,
    dist_dir,
    None,  // NEW: no hero image for fragments
)?;
```

### assets/mod.rs

No changes needed. `optimize_and_rewrite_images` is already re-exported.
The function signature changes but the re-export remains the same.

## Test Plan

Tests follow the project convention: test behavior, not library
internals. Do not use `unwrap`/`expect` unless it is an invariant.

### Unit tests in `html_rewrite.rs`

| Test | What it verifies |
|---|---|
| `test_lazy_loading_default` | All images get `loading="lazy" decoding="async"` except the first |
| `test_first_image_is_eager` | First qualifying image has no `loading="lazy"` |
| `test_first_image_skips_decorative` | Decorative images (`alt=""`, `role="presentation"`) are lazy, first real image is eager |
| `test_first_image_skips_small_icons` | Small icons (`width<100 && height<100`) are lazy, first real image is eager |
| `test_data_eager_attribute` | `data-eager` forces eager, attribute is stripped from output |
| `test_data_eager_does_not_consume_first` | `data-eager` on a non-first image does not prevent the actual first image from being eager |
| `test_hero_image_is_eager` | Image matching hero_image path is always eager |
| `test_hero_image_as_first` | Hero image that is also the first image: eager, counter consumed |
| `test_explicit_loading_preserved` | `loading="eager"` or `loading="lazy"` set by template is not overridden (when no `data-eager` present) |
| `test_explicit_decoding_preserved` | `decoding="sync"` set by template is not overridden |
| `test_lazy_with_picture_rewrite` | `<picture>` element's inner `<img>` has correct `loading`/`decoding` |
| `test_lazy_external_images` | External `<img src="https://...">` still gets lazy attributes |
| `test_lazy_no_images` | HTML with no `<img>` returns unchanged |
| `test_apply_lazy_loading_standalone` | `apply_lazy_loading` works when optimization is disabled |
| `test_conflicting_data_eager_and_loading_lazy` | `data-eager` overrides explicit `loading="lazy"` |
| `test_all_decorative_images` | When all images are decorative, all get lazy (no eager) |
| `test_fragment_first_image_eager` | First image in a fragment is treated as eager |
| `test_empty_variant_map_still_applies_lazy` | When no images have variants (empty variant_map), lazy loading attrs are still set |
| `test_svg_gets_lazy_loading` | SVG images (excluded from optimization) still get `loading`/`decoding` attrs |

### Integration test via `test_full_optimize_and_rewrite`

The existing `test_full_optimize_and_rewrite` test verifies the full
pipeline. After this change, it should additionally assert:

- The inner `<img>` in the generated `<picture>` does NOT have
  `loading="lazy"` (it is the first/only image).
- A second `<img>` on the same page WOULD have `loading="lazy"`.

### What we do NOT test

- Browser behavior of `loading="lazy"` (that is the browser's job).
- `lol_html`'s attribute setting correctness (that is the library's job).
- That `decoding="async"` actually decodes asynchronously (browser).
