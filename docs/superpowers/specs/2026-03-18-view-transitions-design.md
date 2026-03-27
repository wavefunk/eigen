# View Transitions API Integration -- Design Spec

Date: 2026-03-18

## Motivation and Goals

Eigen generates HTMX-powered sites where navigations swap fragment
partials into the page without a full reload. These swaps are instant
but visually jarring -- content pops in with no animation. The
View Transitions API lets the browser animate DOM changes with
cross-fade, slide, or morph effects, making partial loads feel app-like.

**Goals:**

- Inject `<meta name="view-transition" content="same-origin">` for
  cross-document transitions (full page loads, fallback navigations).
- Inject a small inline script that enables HTMX's built-in View
  Transitions support (`htmx.config.globalViewTransitions = true`) for
  same-document transitions (partial swaps).
- Auto-assign `view-transition-name` to elements whose `id` matches
  a fragment block name, so the browser knows which regions to animate.
- Progressive enhancement: browsers without View Transitions support
  get the same instant swaps as today. No polyfill.
- Zero template changes required. Opt-in via config.

**Non-goals:**

- Custom transition CSS (users can add their own `::view-transition-*`
  rules in their stylesheets).
- Per-page or per-link transition control.
- Polyfilling older browsers.

## Configuration

### site.toml Schema

A new field on `BuildConfig`:

```toml
[build.view_transitions]
enabled = false
```

**Default: disabled.** This is a visual feature that should be a
conscious choice. Follows the same pattern as `critical_css` and
`bundling`.

### Config Struct

```rust
/// View Transitions API configuration.
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

Add to `BuildConfig`:

```rust
pub struct BuildConfig {
    // ... existing fields ...
    /// View Transitions API configuration.
    #[serde(default)]
    pub view_transitions: ViewTransitionsConfig,
}
```

Update `BuildConfig::default()` to include
`view_transitions: ViewTransitionsConfig::default()`.

## What Gets Injected

Three things are injected into every full page (not fragments):

### 1. Meta Tag (cross-document transitions)

```html
<meta name="view-transition" content="same-origin">
```

Appended inside `<head>`. Enables the browser's native cross-document
view transitions for full page navigations (initial load, non-HTMX
links, browser back/forward).

### 2. Inline Script (same-document transitions via HTMX)

```html
<script>
document.addEventListener("DOMContentLoaded", function() {
  if (typeof htmx !== "undefined" && document.startViewTransition) {
    htmx.config.globalViewTransitions = true;
  }
});
</script>
```

Appended inside `<head>`. HTMX 1.9+ has built-in View Transitions
support via `htmx.config.globalViewTransitions`. When enabled, HTMX
wraps all swaps (including OOB swaps) in
`document.startViewTransition()` automatically.

The script waits for `DOMContentLoaded` to ensure HTMX is loaded
before accessing `htmx.config` (HTMX may be included via a `<script>`
tag later in the document). The `typeof htmx !== "undefined"` guard
handles pages where HTMX is not loaded. The
`document.startViewTransition` check is progressive enhancement:
browsers without View Transitions support skip the config change and
get instant swaps as they do today.

### 3. Transition Names on Fragment Targets

For each fragment block extracted from the page, elements with a
matching `id` attribute get `view-transition-name` injected into their
`style` attribute:

```html
<div id="content" style="view-transition-name: content;">
```

This tells the browser which DOM regions to animate between
navigations. Without transition names, the browser can only do a
whole-page cross-fade. With names, it can independently animate each
region (content morphs, nav stays put, sidebar slides).

**Rules:**

- If the element already has a `style` attribute, trim trailing
  whitespace and semicolons, then append
  `; view-transition-name: <name>` to avoid double semicolons.
- If the element's `style` already contains `view-transition-name`,
  skip it (respect author intent).
- Fragment block names come from the rendered HTML's fragment markers
  (extracted before marker stripping in the pipeline). Only names
  actually found in fragment markers are used.
- When `config.build.fragments` is disabled, no block names are
  extracted and only the meta tag + script are injected (no
  `view-transition-name` attributes). This is correct behavior:
  without fragments, there are no HTMX swap targets to animate
  independently.

## Architecture

### New Module: `src/build/view_transitions.rs`

A single file following the pattern of `seo.rs` and `minify.rs`.

**Public API:**

```rust
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
) -> String
```

**Internal functions:**

```rust
/// Check if HTML already contains a view-transition meta tag.
///
/// Uses lol_html with a `meta[name='view-transition']` selector,
/// matching the convention in `seo::has_canonical_link` and
/// `json_ld::has_existing_json_ld`.
fn has_view_transition_meta(html: &str) -> bool

/// Build the HTML string to inject into <head>.
/// Returns the meta tag + script tag.
fn build_head_injection() -> String

/// Inject content into <head> and add view-transition-name to
/// elements with matching IDs.
///
/// Converts `block_names` to a `HashSet` once for O(1) lookup
/// per element.
fn rewrite_html(
    html: &str,
    head_html: &str,
    block_names: &HashSet<String>,
) -> Result<String, lol_html::errors::RewritingError>
```

### Pipeline Position

After JSON-LD injection, before minification:

```
render template
  -> strip fragment markers
  -> localize assets
  -> optimize images
  -> rewrite CSS background images
  -> plugin post_render_html
  -> critical CSS inlining        (build::critical_css)
  -> preload/prefetch hints       (build::hints)
  -> SEO meta tag injection       (build::seo)
  -> JSON-LD structured data      (build::json_ld)
  -> view transitions injection   (build::view_transitions)  <-- NEW
  -> minify HTML                  (build::minify)
  -> write to disk
```

**Rationale:**

- Must run before minify so injected tags get minified.
- No dependency on SEO/JSON-LD/hints; placed last among injections
  for organization (all "inject into `<head>`" steps together).
- Must run after fragment marker stripping (markers are gone from
  `full_html`), but we extract block names from the pre-stripped
  `rendered` string earlier.

### Integration into `render.rs`

**Block name extraction:** Before the pipeline runs, extract fragment
block names from the pre-stripped `rendered` HTML. This is a cheap
regex scan for `<!--FRAG:(\w+):START-->` patterns. The same regex
already exists in `fragments::extract_fragments`.

Add a lightweight helper:

```rust
/// Extract just the block names from fragment markers without
/// extracting the full content.
fn extract_fragment_block_names(rendered: &str) -> Vec<String>
```

This is called once after rendering, before marker stripping. The
resulting names are passed to `inject_view_transitions` later in the
pipeline.

**In `render_static_page`** (after JSON-LD, before minify):

```rust
// Extract block names before marker stripping (for view transitions).
let block_names = extract_fragment_block_names(&rendered);

// ... (existing pipeline: strip markers, localize, optimize, etc.) ...

// View transitions injection (after JSON-LD, before minify).
let full_html = if config.build.view_transitions.enabled {
    view_transitions::inject_view_transitions(&full_html, &block_names)
} else {
    full_html
};
```

**In `render_dynamic_page`:** Same pattern, inside the per-item loop.

### Dev Server

View transitions run in dev builds too. Transitions are useful during
development to preview the UX. The `enabled` config flag controls
both production and dev builds uniformly.

### Fragments Are NOT Modified

Fragment partials have no `<head>` and are swapped by HTMX inside the
transition callback. No injection needed on fragments.

## Impact on Existing Code

### Files Modified

| File | Change |
|---|---|
| `src/config/mod.rs` | Add `ViewTransitionsConfig`, `view_transitions` field on `BuildConfig`, update `Default` impl |
| `src/build/mod.rs` | Add `pub mod view_transitions;` |
| `src/build/render.rs` | Extract block names, call `view_transitions::inject_view_transitions` in both render functions |

### Files Created

| File | Description |
|---|---|
| `src/build/view_transitions.rs` | View transitions injection module |

### Test Helper Updates

Adding `ViewTransitionsConfig` to `BuildConfig` with `#[serde(default)]`
means it defaults to `ViewTransitionsConfig::default()` (disabled).
The manual `BuildConfig` constructors in test helpers use
`..Default::default()` already, so no test helper changes are needed.

## Error Handling

| Scenario | Behavior |
|---|---|
| lol_html rewriting error | Log warning, return original HTML |
| No `<head>` element | lol_html selector doesn't match; return HTML unchanged |
| No fragment blocks in page | Meta tag + script still injected; no transition names added |
| Element has existing `view-transition-name` in style | Skip that element |
| Element has existing `style` attribute | Append `; view-transition-name: <name>` |
| View-transition meta tag already in HTML | Skip meta tag injection |

**Principle:** View transitions are a visual enhancement. Failures
must never break the build or degrade output.

## Test Plan

### Unit Tests (in `src/build/view_transitions.rs`)

| Test | What it verifies |
|---|---|
| `test_inject_meta_tag` | `<meta name="view-transition">` appears in `<head>` |
| `test_inject_script` | `htmx.config.globalViewTransitions` script appears in `<head>` |
| `test_inject_transition_names` | Elements with matching IDs get `style="view-transition-name: ..."` |
| `test_existing_inline_style` | Appends to existing `style` attribute without clobbering |
| `test_existing_view_transition_name` | Skips element that already has `view-transition-name` in style |
| `test_no_head` | HTML without `<head>` returns unchanged |
| `test_no_matching_ids` | No fragment names match any element; meta+script still injected |
| `test_multiple_fragment_names` | Multiple blocks all get transition names |
| `test_existing_meta_tag` | Existing view-transition meta tag is not duplicated |
| `test_fragments_disabled` | When no block names provided, meta+script injected but no transition names |

### Config Tests (in `src/config/mod.rs`)

| Test | What it verifies |
|---|---|
| `test_view_transitions_default_disabled` | Default config has `enabled: false` |
| `test_view_transitions_enabled` | `[build.view_transitions] enabled = true` parses correctly |

## What Is NOT In Scope

1. **Custom transition CSS.** Users can add `::view-transition-*`
   rules in their own stylesheets. Eigen provides the plumbing,
   not the visual design.

2. **Per-page or per-link transition control.** A `view_transition`
   frontmatter field or `link_to` parameter could be added later.
   The initial implementation is global on/off.

3. **Default transition CSS.** No built-in cross-fade or slide
   animation. The browser's default (cross-fade) is a reasonable
   starting point. Custom animations are the user's domain.

4. **Polyfill.** The View Transitions API is supported in
   Chrome/Edge 111+ and Safari 18+. Firefox support is in progress.
   Progressive enhancement means unsupported browsers work fine.

5. **`view-transition-class` attribute.** A newer spec addition for
   grouping transitions. Out of scope for v1.

## Performance Considerations

- The injected meta tag and script add ~120 bytes pre-minification.
- The `lol_html` rewrite pass adds negligible build time (sub-ms per
  page, same as other injection steps).
- No new dependencies required.
- No runtime performance impact on browsers without View Transitions
  support (the script guard prevents the config change).
