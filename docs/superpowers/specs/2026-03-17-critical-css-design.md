# Critical CSS Inlining -- Design Spec

Date: 2026-03-17

## Motivation and Goals

External CSS stylesheets are render-blocking: the browser cannot paint anything
until every `<link rel="stylesheet">` has been fetched and parsed. For sites
with even modest CSS files, this directly harms Largest Contentful Paint (LCP)
and First Contentful Paint (FCP).

Critical CSS inlining solves this by:

1. Extracting the subset of CSS rules that are actually used on each page.
2. Inlining those rules in a `<style>` block inside `<head>`.
3. Deferring the full stylesheet load so it does not block rendering.

**Goals:**

- Reduce render-blocking CSS to zero for the initial paint.
- Produce correct output for all valid CSS (media queries, @font-face,
  @keyframes, pseudo-selectors, combinators, nesting).
- Integrate cleanly into the existing build pipeline with minimal disruption.
- Be configurable and opt-in, defaulting to disabled for backward
  compatibility.
- Handle the common case well: a small number of local stylesheets referenced
  via `<link>` tags in `<head>`.

**Non-goals (see "Out of Scope" section):**

- True viewport-based critical CSS (requires a headless browser).
- CSS bundling or tree-shaking across pages.
- Processing of CDN-hosted stylesheets that are not localized.

## Architecture

### Approach: Selector-Matched "Used CSS" Extraction

A true "above-the-fold" critical CSS extractor needs a browser to determine
which elements are visible in the viewport. This is impractical for a Rust
static site generator -- it would require a headless browser dependency (e.g.
Chromium via puppeteer), massively increasing build time and complexity.

Instead, we use **selector matching against rendered HTML**. For each page:

1. Parse the rendered HTML into a DOM tree.
2. Parse each referenced local stylesheet into a CSS AST.
3. For every CSS rule, test whether its selector matches any element in the
   HTML.
4. Collect all matching rules (plus their transitive dependencies like
   @font-face, @keyframes, custom properties).
5. Serialize the matching rules into a `<style>` block.
6. Rewrite the original `<link>` tag to load asynchronously.

This approach extracts "used CSS" rather than "above-the-fold CSS." For most
static sites, these are nearly identical because pages typically only use a
fraction of a site-wide stylesheet. The result eliminates unused rules (which
is strictly better than the current behavior of loading everything) while
keeping all rules that any element on the page could trigger.

**Why not the plugin system?** The plugin `post_render_html` hook receives
rendered HTML as a `String` and returns a `String`. This is technically
sufficient, but critical CSS extraction needs access to the `dist_dir`
filesystem to read stylesheet files, and it is a core performance
optimization rather than an extension. It follows the same pattern as
`build::minify` -- a build-pipeline transformation that every site benefits
from. Implementing it as a core module in `build::critical_css` keeps the
code discoverable and avoids forcing users to configure a plugin for a
standard optimization.

### Pipeline Position

The current per-page build pipeline (from `render_static_page` and
`render_dynamic_page` in `src/build/render.rs`) is:

```
render template
  -> strip fragment markers
  -> localize assets
  -> optimize images (img -> picture)
  -> rewrite CSS background images
  -> plugin post_render_html
  -> minify HTML
  -> write to disk
```

Critical CSS inlining slots in **after plugin post_render_html and before
minify**:

```
  -> plugin post_render_html
  -> critical CSS inlining  <-- NEW
  -> minify HTML
  -> write to disk
```

**Rationale:**

- It must run after asset localization because stylesheet URLs may have been
  rewritten from remote to local paths.
- It must run after plugin hooks because plugins (e.g. Tailwind) may generate
  or modify CSS files on disk that critical CSS needs to read.
- It must run before minification so that the inlined `<style>` block
  benefits from `minify-html`'s CSS minifier (already configured with
  `cfg.minify_css = true` in `src/build/minify.rs`).
- Fragments do NOT get critical CSS inlining. Fragments are partial HTML
  snippets loaded via HTMX into an already-rendered page that has its
  stylesheets loaded. Inlining CSS into fragments would be wasteful and
  semantically wrong.

### Dependencies

**`lightningcss`** for CSS parsing, AST walking, and serialization:

- Written in Rust (no FFI or subprocess).
- Fast: used by Parcel, Tailwind, and other production tools.
- Complete: handles media queries, @supports, @font-face, @keyframes,
  @layer, custom properties, nesting, and all modern CSS features.
- Well-maintained by the Parcel team.
- Provides native `@import` bundling via its bundler API.

**`scraper`** for HTML parsing and CSS selector matching:

- Wraps `html5ever` + `selectors` (Servo's implementation).
- Provides `Html::parse_document`, `Selector::parse`, `html.select(&selector)`.
- Builds a full DOM tree needed for selector matching (unlike `lol_html`
  which is a streaming rewriter and cannot build a tree).

Note: `lol_html` is still used for the HTML rewriting step (injecting
`<style>`, rewriting `<link>` tags) since it excels at streaming HTML
mutation. `scraper` is only used for DOM querying.

**New dependencies to add to Cargo.toml:**

```toml
lightningcss = "1"
scraper = "0.22"
```

## Data Models and Types

### Configuration

Add to `src/config/mod.rs`:

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
    /// `<link>` tag (no inlining for that page). Prevents bloating
    /// HTML for pages that use most of the stylesheet.
    /// Default: 50_000 (50 KB).
    #[serde(default = "default_max_inline_size")]
    pub max_inline_size: usize,

    /// Whether to keep the original `<link>` tag for async loading of
    /// the full stylesheet (for styles not in the critical set, like
    /// hover states on below-fold elements). Default: true.
    /// When false, the `<link>` is removed entirely (pure tree-shaking
    /// mode -- only matched CSS is delivered).
    #[serde(default = "default_true")]
    pub preload_full: bool,

    /// Glob patterns for stylesheet paths to exclude from critical CSS
    /// processing. Matched against the href value.
    /// Example: ["**/vendor/**", "**/print.css"]
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

Note: `default_true` already exists in `config/mod.rs` and can be reused
directly. The `Default` impl is required because `BuildConfig` uses
`#[serde(default)]` on the `critical_css` field.

Add the field to `BuildConfig`:

```rust
pub struct BuildConfig {
    // ... existing fields (fragments, fragment_dir, content_block,
    //     oob_blocks, minify) ...

    /// Critical CSS inlining configuration.
    #[serde(default)]
    pub critical_css: CriticalCssConfig,
}
```

Update the `Default` impl for `BuildConfig` to include:

```rust
impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            // ... existing fields ...
            critical_css: CriticalCssConfig::default(),
        }
    }
}
```

### Internal Types

These types live inside `src/build/critical_css/` and are not part of the
public API surface. They are implementation details of the extraction logic.

```rust
/// Selectors extracted from a CSS rule, pre-processed for DOM matching.
/// Pseudo-classes and pseudo-elements are stripped for matching purposes,
/// but the original selector text is preserved for output.
struct MatchableSelector {
    /// The original selector text (e.g. ".btn:hover::after").
    original: String,
    /// The stripped selector for DOM matching (e.g. ".btn").
    /// None if stripping produces an empty/unmatchable selector
    /// (e.g. "::selection" -> nothing). In that case, include
    /// the rule unconditionally since it may apply to any text.
    matchable: Option<String>,
}

/// Tracks which global rules (font-face, keyframes, custom properties)
/// are transitively referenced by matched style rules.
struct GlobalDependencies {
    /// Font family names referenced by matched rules.
    font_families: HashSet<String>,
    /// Animation names referenced by matched rules.
    animation_names: HashSet<String>,
    /// CSS custom property names (e.g. "--color-primary") referenced
    /// via var() in matched rules.
    custom_properties: HashSet<String>,
}
```

The design deliberately avoids creating a custom `CssRule` intermediate
representation. Instead, we work directly with `lightningcss`'s
`StyleSheet` and `CssRule` types. `lightningcss` provides:

- `StyleSheet::parse()` to get a parsed AST.
- `CssRuleList` to iterate rules.
- `StyleRule` with `.selectors` and `.declarations`.
- `MediaRule`, `SupportsRule`, `LayerBlockRule` for conditional rules.
- `FontFaceRule`, `KeyframesRule` for global rules.
- `ToCss` trait for serializing any rule back to a string.

Working with the library's own types avoids redundant allocation and
keeps the code aligned with lightningcss updates.

## API Surface

### Public Function

The module `build::critical_css` exposes a single public function:

```rust
/// Extract critical CSS and inline it into the HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
///
/// Steps:
/// 1. Parse the HTML to find `<link rel="stylesheet">` tags.
/// 2. Read each referenced local stylesheet from dist_dir.
/// 3. Parse the CSS and match selectors against the HTML DOM.
/// 4. Inline the matched CSS as a `<style>` block in `<head>`.
/// 5. Rewrite the `<link>` tags to load asynchronously (if
///    preload_full is true) or remove them (if false).
///
/// Returns the (possibly rewritten) HTML string.
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
) -> String
```

**Return type rationale:** The function returns `String`, not
`Result<String>`. Critical CSS is a pure optimization -- if anything goes
wrong (file not found, parse error, size limit exceeded), the correct
behavior is always to return the original HTML unchanged. Making the
return type infallible ensures callers cannot accidentally propagate an
error that breaks the build. Internally, the function uses `Result` for
control flow and logs warnings on failure paths.

### Internal Functions

```rust
/// Extract `<link rel="stylesheet" href="...">` hrefs from HTML.
/// Uses lol_html for reliable HTML parsing.
/// Skips external URLs (http/https) and excluded patterns.
fn extract_stylesheet_hrefs(
    html: &str,
    exclude: &[String],
) -> Vec<String>

/// Read and parse a CSS file from dist_dir.
/// Resolves @import directives using lightningcss's bundler.
fn load_stylesheet(
    href: &str,
    dist_dir: &Path,
) -> Result<String>

/// Match CSS rules against an HTML document.
/// Returns the critical CSS as a serialized string containing only
/// the rules whose selectors match at least one element, plus any
/// transitively referenced global rules.
fn extract_critical_css(
    css: &str,
    document: &scraper::Html,
) -> Result<String>

/// Strip pseudo-classes and pseudo-elements from a selector string
/// to produce a version that can be matched against a static DOM.
///
/// Examples:
///   ".btn:hover"       -> Some(".btn")
///   ".btn::before"     -> Some(".btn")
///   "a:visited > span" -> Some("a > span")
///   "::selection"      -> None  (include unconditionally)
///   ":root"            -> Some(":root")  (structural, not stripped)
fn strip_pseudo_for_matching(selector: &str) -> Option<String>

/// Test whether a CSS selector matches any element in the document.
/// Returns true if the selector matches, false otherwise.
/// If the selector cannot be parsed by scraper, returns true
/// (conservative: include the rule rather than risk dropping it).
fn selector_matches(
    selector: &str,
    document: &scraper::Html,
) -> bool

/// Scan CSS declarations for references to font families, animation
/// names, and CSS custom properties. Returns the set of dependencies.
fn collect_global_deps(declarations: &str) -> GlobalDependencies

/// Rewrite HTML to inline critical CSS and defer full stylesheets.
/// Uses lol_html for streaming HTML rewriting.
fn rewrite_html(
    html: &str,
    critical_css: &str,
    processed_hrefs: &[String],
    preload_full: bool,
) -> Result<String>
```

### Stylesheet Caching

Many pages on a site reference the same stylesheet(s). Parsing and
extracting CSS is the most expensive part of this feature. Without
caching, a site with 500 pages and one shared `style.css` would parse
that file 500 times.

We introduce a per-build stylesheet cache scoped to the `build()` call:

```rust
/// Cache for parsed stylesheets, keyed by href.
/// Passed through the build pipeline to avoid re-reading and
/// re-parsing the same CSS file for every page.
struct StylesheetCache {
    /// href -> raw CSS content (after @import resolution).
    cache: HashMap<String, String>,
}
```

The cache stores the resolved CSS text (after @import bundling). It is
created at the start of `build()` and passed to `inline_critical_css`.
The CSS text is then re-parsed per page by lightningcss -- this is
intentional because lightningcss parsing is fast (microseconds for
typical stylesheets) and the parsed AST contains mutable state for
rule matching. The expensive part that we cache is the filesystem I/O
and @import resolution.

Updated public signature to support caching:

```rust
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
    css_cache: &mut StylesheetCache,
) -> String
```

### Deferred Loading Pattern

The `<link>` tag rewriting uses the standard pattern recommended by
web.dev and Google:

**Before:**
```html
<link rel="stylesheet" href="/css/style.css">
```

**After (preload_full = true):**
```html
<style>/* critical CSS inlined here */</style>
<link rel="preload" href="/css/style.css" as="style"
      onload="this.onload=null;this.rel='stylesheet'">
<noscript><link rel="stylesheet" href="/css/style.css"></noscript>
```

**After (preload_full = false):**
```html
<style>/* critical CSS inlined here */</style>
```

The `onload` trick is the standard approach used by loadCSS and recommended
by Lighthouse. When the preloaded stylesheet finishes loading, the `onload`
handler switches it from `preload` to `stylesheet`, applying it without
blocking render. The `<noscript>` fallback ensures the stylesheet loads
normally when JavaScript is disabled.

When multiple `<link>` tags are present, all critical CSS from all
stylesheets is combined into a single `<style>` block placed before the
first rewritten `<link>`. Each original `<link>` is rewritten in place.

## Error Handling

| Scenario | Behavior |
|---|---|
| Stylesheet file not found in dist_dir | Log warning, skip that stylesheet, proceed with others |
| CSS parse error in lightningcss | Log warning with file path and error, skip that stylesheet |
| HTML parse error in scraper | Log warning, return original HTML unchanged |
| Critical CSS exceeds max_inline_size | Log info, return original HTML unchanged (no inlining for that page) |
| @import references external URL | Skip the import (lightningcss bundler handles this gracefully) |
| @import references non-existent local file | Log warning, skip the import |
| Circular @import | lightningcss bundler detects and reports this; log warning, skip stylesheet |
| Selector parse error in scraper | Log debug, treat as matching (conservative: include the rule) |
| Empty critical CSS (no selectors match) | Skip inlining, return original HTML unchanged |
| lol_html rewriting error | Log warning, return original HTML unchanged |
| No `<link rel="stylesheet">` tags found | Return original HTML unchanged (no-op) |

**Principle:** Critical CSS is a pure optimization. Failures must never break
the build or degrade the output. Every error path falls back to returning the
original HTML unchanged, preserving correctness. Warnings are logged so users
can investigate, but the build always succeeds.

**Why selector parse failures are treated as "matching":** If scraper cannot
parse a selector (e.g., very new CSS syntax), the conservative choice is to
include the rule. Excluding it risks breaking page rendering. Including an
unnecessary rule only costs a few extra bytes.

## Edge Cases and Failure Modes

### Pseudo-selectors

CSS selectors like `:hover`, `:focus`, `:active`, `::before`, `::after` never
match in a static DOM. However, they are critical for the user experience and
must be included.

**Strategy:** Before matching a selector against the DOM, strip pseudo-classes
and pseudo-elements from the selector string using `strip_pseudo_for_matching`.

Concrete rules for stripping:

- **Dynamic pseudo-classes** (`:hover`, `:focus`, `:active`, `:visited`,
  `:focus-visible`, `:focus-within`): Strip, match base selector.
- **Structural pseudo-classes** (`:root`, `:first-child`, `:nth-child(...)`,
  `:empty`, `:not(...)`, `:is(...)`, `:where(...)`): Do NOT strip. `scraper`
  supports these natively.
- **Pseudo-elements** (`::before`, `::after`, `::placeholder`,
  `::first-line`, `::first-letter`, `::marker`, `::backdrop`): Strip, match
  base selector.
- **Selector-less pseudos** (`::selection`, `::view-transition`): If stripping
  the pseudo produces an empty selector, return `None` to signal the rule
  should be included unconditionally.
- **`:root`**: Matches `html` in scraper. Do not strip.

The stripping is done via regex on the serialized selector string. This is
simpler and more robust than trying to manipulate the parsed selector AST,
because lightningcss and scraper use different selector parser
implementations.

```
// Regex pattern for strippable pseudo-classes/elements:
// Match ::pseudo-element or :pseudo-class (but not :root, :first-child,
// :nth-child, :not, :is, :where, :has, :empty, :only-child, etc.)
::{identifier}(\([^)]*\))?     -> strip
:(?!root|first-child|last-child|nth-child|nth-last-child|nth-of-type|
   nth-last-of-type|first-of-type|last-of-type|only-child|only-of-type|
   empty|not|is|where|has){identifier}(\([^)]*\))?   -> strip
```

### Media queries

`@media` blocks contain style rules that should be selector-matched
independently. If any child rule matches, include the `@media` wrapper with
only the matching children. This preserves responsive behavior without
including unused rules inside media queries.

Implementation: Walk `lightningcss::rules::CssRuleList` recursively. When
encountering a `MediaRule`, process its child rules. If any child matches,
serialize the `@media` prelude with only matching children.

### CSS nesting

Modern CSS supports nesting (e.g., `.parent { .child { color: red } }`).
`lightningcss` parses nested rules into `NestingRule` variants within a
`StyleRule`'s child rules. The implementation must recursively walk child
rules of style rules, not just top-level rules.

### @font-face and @keyframes

These are "global" rules not associated with any selector. We include them
based on transitive reference:

- `@font-face { font-family: "Inter" }` is included if any matched rule uses
  `font-family: Inter` (or shorthand `font` that references it).
- `@keyframes spin` is included if any matched rule uses
  `animation-name: spin` or `animation: spin ...`.

We scan the serialized declaration text of matched rules for font-family and
animation-name references using simple string/regex matching rather than
fully parsing CSS values. This is sufficient because:

1. Font family names in `font-family` and `font` shorthand are always
   quoted strings or keyword identifiers.
2. Animation names in `animation-name` and `animation` shorthand follow
   the same pattern.

### Custom properties (CSS variables)

Rules defining `--custom-property` on `:root`, `html`, or `body` are
included if any matched rule references `var(--custom-property)`. We scan
matched rule declarations for `var(--name)` patterns and include the
corresponding custom property definitions.

This is a single-level scan: if a custom property references another custom
property (`--a: var(--b)`), we do NOT transitively resolve the chain. The
`:root` / `html` / `body` selectors will match via normal selector matching
anyway, so properties defined there are typically included. The `var()`
scanning is a safety net for cases where the property is defined on an
element that doesn't appear on the page but its value is consumed by an
element that does.

### Multiple stylesheets

Pages may reference multiple `<link rel="stylesheet">` tags. We process each
independently, extract critical rules from each, and combine them into a
single `<style>` block. The `max_inline_size` limit applies to the combined
result.

All processed `<link>` tags are rewritten. Unprocessed ones (excluded by
glob, external URLs, etc.) are left unchanged.

### Inline `<style>` blocks

Existing `<style>` blocks in the HTML are left unchanged. They are already
inline and not render-blocking in the same way external stylesheets are.
We do not attempt to tree-shake them.

### CDN / external stylesheets

Stylesheets with `http://` or `https://` hrefs are skipped. We only process
local stylesheets (paths starting with `/` that resolve to files in
dist_dir). CDN stylesheets that were localized by the asset localization
step will have local paths by this point and will be processed.

### Very large stylesheets

If the combined critical CSS for a page exceeds `max_inline_size`, we skip
inlining for that page entirely and return the original HTML. This prevents
pathological cases where a page uses most of a large CSS framework, and
inlining would double the HTML size without meaningful performance benefit.

### @layer rules

CSS `@layer` rules define cascade layer ordering. If the original stylesheet
uses `@layer`, the critical CSS must preserve the layer declarations and
ordering. lightningcss represents these as `LayerBlockRule` (with child
rules) and `LayerStatementRule` (order declarations). `LayerStatementRule`
rules are always included in the critical CSS output. `LayerBlockRule` rules
are treated like `@media` -- walk children, include if any child matches.

### Interaction with content hashing (future)

The proposals doc mentions content hashing (proposal #3) as a future feature.
Critical CSS inlining is compatible: the `<link>` href will point to the
hashed filename, and we read the file from dist_dir by resolving the href.
The preload `<link>` will also use the hashed filename. No special handling
needed.

### Dev server

Critical CSS inlining is **disabled during dev server** regardless of config.
The dev server in `src/dev/rebuild.rs` has its own render functions
(`render_static_page_dev`, `render_dynamic_page_dev`) that do not call the
minification step. Similarly, they will not call `inline_critical_css`. The
`enabled` flag check happens at the call site in `render_static_page` and
`render_dynamic_page` in `src/build/render.rs`, so the dev server render
functions simply never invoke it.

### Stylesheets loaded via `<link>` with `media` attribute

If a `<link>` tag has a `media` attribute (e.g., `media="print"`), we skip
critical CSS processing for that stylesheet. Print stylesheets and other
media-specific stylesheets are already non-render-blocking for the default
media type.

## Configuration Examples

### Minimal (opt-in)

```toml
[build.critical_css]
enabled = true
```

### Full configuration

```toml
[build.critical_css]
enabled = true
max_inline_size = 30000        # 30 KB limit
preload_full = true            # async-load full stylesheet
exclude = ["**/vendor/**"]     # skip vendor CSS
```

### Tree-shaking mode (no async preload)

```toml
[build.critical_css]
enabled = true
preload_full = false  # only deliver matched CSS, remove <link> entirely
```

## What Is NOT In Scope

1. **True above-the-fold detection.** This requires a headless browser to
   determine which elements are in the viewport. Our selector-matching
   approach extracts "used CSS" which is a superset of above-the-fold CSS
   but strictly better than loading the full stylesheet.

2. **CSS bundling.** Combining multiple stylesheet files into one. This is
   proposal #5 in proposals.md and is a separate feature.

3. **Cross-page CSS optimization.** Analyzing CSS usage across all pages to
   produce a single optimized stylesheet. Each page is processed
   independently.

4. **Processing CDN stylesheets.** We only process local files. External
   stylesheets must first be localized by the existing asset localization
   step.

5. **Minification of the inlined CSS.** The `minify-html` step that runs
   after critical CSS inlining already minifies inline `<style>` blocks
   (`cfg.minify_css = true` in `build_cfg()`). We do not add a separate
   CSS minification pass.

6. **Source map support.** Critical CSS is a production optimization. Source
   maps are not generated for the inlined CSS.

7. **Per-page or per-template CSS configuration.** The feature is
   site-wide. There is no frontmatter-level control. If a page's critical
   CSS exceeds `max_inline_size`, it automatically falls back to the
   original behavior.

8. **Processing of `<style>` attributes (inline styles on elements).**
   These are already inline and do not block rendering.

9. **Transitive custom property resolution.** We do not follow chains like
   `--a: var(--b)` to also include `--b`. This is acceptable because
   `:root`/`html`/`body` rules are matched via normal selector matching.

## Module Structure

```
src/build/
  critical_css/
    mod.rs          -- public API (inline_critical_css, StylesheetCache)
    extract.rs      -- CSS parsing, rule walking, selector matching,
                       global dependency collection
    rewrite.rs      -- HTML rewriting (inject <style>, rewrite <link>)
```

Three files because the concerns are distinct and each is non-trivial:
- `mod.rs`: orchestration and caching (~80 lines)
- `extract.rs`: CSS parsing and selector matching (~250 lines)
- `rewrite.rs`: lol_html-based HTML mutation (~100 lines)

This follows the pattern of `src/assets/` which splits concerns across
submodules (mod.rs for re-exports, html_rewrite.rs for lol_html usage,
rewrite.rs for URL scanning, etc.).

## Integration Points

### build/render.rs

In `render_static_page` and `render_dynamic_page`, add the critical CSS
step after plugin `post_render_html` and before minification. The
`StylesheetCache` must be created in `build()` and threaded through:

```rust
// In build():
let mut css_cache = critical_css::StylesheetCache::new();

// Then pass &mut css_cache to render_static_page / render_dynamic_page.
```

In each render function, between plugin hooks and minify:

```rust
// Critical CSS inlining (after plugins, before minify).
let full_html = if config.build.critical_css.enabled {
    critical_css::inline_critical_css(
        &full_html,
        &config.build.critical_css,
        dist_dir,
        &mut css_cache,
    )
} else {
    full_html
};
```

Note: No `?` operator -- `inline_critical_css` returns `String`, not
`Result<String>`. It handles all errors internally.

### build/mod.rs

Add `pub mod critical_css;` to the module declarations:

```rust
pub mod context;
pub mod critical_css;  // <-- NEW
pub mod fragments;
pub mod minify;
pub mod output;
pub mod render;
pub mod sitemap;
```

### config/mod.rs

Add `CriticalCssConfig` struct (with its `Default` impl) and the
`critical_css` field to `BuildConfig`. The existing `default_true` function
is reused for `preload_full`.

### Cargo.toml

Add dependencies:

```toml
lightningcss = "1"
scraper = "0.22"
```

## Test Plan

Tests follow the project convention: write tests immediately after the
feature, avoid "ceremony" tests that just test the library, do not use
`unwrap` or `expect` unless it is an invariant.

### Unit tests in `extract.rs`

| Test | What it verifies |
|---|---|
| `test_strip_pseudo_hover` | `.btn:hover` -> `Some(".btn")` |
| `test_strip_pseudo_before` | `.icon::before` -> `Some(".icon")` |
| `test_strip_pseudo_preserves_structural` | `:root`, `:first-child`, `:nth-child(2n)` are not stripped |
| `test_strip_pseudo_selection` | `::selection` -> `None` (include unconditionally) |
| `test_strip_pseudo_compound` | `a:hover > .icon::after` -> `Some("a > .icon")` |
| `test_selector_matches_basic` | `.exists` matches `<div class="exists">` |
| `test_selector_matches_absent` | `.missing` does not match any element |
| `test_selector_matches_unparseable` | Invalid selector returns true (conservative) |
| `test_extract_simple` | Given HTML + CSS, extracts only matching rules |
| `test_extract_media_query` | Matching rule inside `@media` is included with wrapper |
| `test_extract_media_query_no_match` | Non-matching rule inside `@media` is excluded |
| `test_extract_font_face_transitive` | `@font-face` for "Inter" included when matched rule uses `font-family: Inter` |
| `test_extract_keyframes_transitive` | `@keyframes spin` included when matched rule uses `animation: spin` |
| `test_extract_custom_property` | `:root { --color: red }` included when matched rule uses `var(--color)` |
| `test_extract_layer` | `@layer` statement rules are always included |
| `test_extract_empty_result` | No selectors match -> empty string |

### Unit tests in `rewrite.rs`

| Test | What it verifies |
|---|---|
| `test_rewrite_single_link` | Single `<link>` is replaced with `<style>` + preload |
| `test_rewrite_multiple_links` | Multiple `<link>` tags, single combined `<style>` |
| `test_rewrite_no_preload` | With `preload_full = false`, `<link>` is removed entirely |
| `test_rewrite_noscript_fallback` | `<noscript>` fallback is present when `preload_full = true` |
| `test_rewrite_preserves_other_links` | Non-stylesheet `<link>` tags (favicon, etc.) are untouched |
| `test_rewrite_external_link_untouched` | `<link href="https://...">` is not modified |

### Unit tests in `mod.rs`

| Test | What it verifies |
|---|---|
| `test_inline_critical_css_disabled` | Returns input unchanged when `enabled = false` |
| `test_inline_critical_css_no_links` | HTML without `<link>` tags returns unchanged |
| `test_inline_critical_css_file_not_found` | Missing CSS file logs warning, returns original HTML |
| `test_inline_critical_css_exceeds_max_size` | Large critical CSS triggers fallback |
| `test_inline_critical_css_excluded_pattern` | Excluded stylesheet is skipped |

### Integration tests in `build/render.rs`

| Test | What it verifies |
|---|---|
| `test_build_with_critical_css` | Full build with `critical_css.enabled = true`, output HTML contains inlined `<style>` and preload `<link>` |
| `test_build_critical_css_disabled_by_default` | Default config does not inline critical CSS |

### What we do NOT test

- lightningcss parsing correctness (that is the library's job)
- scraper selector matching correctness (that is the library's job)
- minify-html's CSS minification of `<style>` blocks (existing tests cover this)
