# CSS/JS Bundling and Tree-Shaking -- Design Spec

Date: 2026-03-17

## Motivation and Goals

Static sites commonly accumulate multiple CSS and JS files across
templates: a reset stylesheet, a layout stylesheet, a component
stylesheet, utility scripts, and so on. Each `<link>` or `<script>` tag
in the HTML produces a separate HTTP request. Even with HTTP/2
multiplexing, the overhead of multiple small files is measurable:

1. **Request overhead.** Each file requires headers, TLS record framing,
   and connection scheduling. A single bundled file eliminates this per-
   file cost entirely.

2. **Compression efficiency.** Compressors (gzip, brotli) work better on
   larger inputs. A 50 KB merged CSS file compresses more efficiently
   than five 10 KB files compressed individually.

3. **Dead CSS.** Site-wide stylesheets typically contain rules for
   elements that appear on some pages but not others. A universal
   stylesheet downloaded by every page carries the weight of rules no
   page uses. Tree-shaking removes selectors that never match any
   rendered element across the entire site, reducing the CSS payload for
   every visitor.

Eigen already has a per-page critical CSS inlining feature that extracts
only the selectors used on each individual page and inlines them. CSS
bundling and tree-shaking complements this: it produces a smaller
site-wide stylesheet that critical CSS then draws from. With both
features enabled, the page-load sequence is:

1. Browser receives HTML with critical CSS inlined in `<style>`.
2. Browser begins rendering immediately (no blocking requests).
3. The full stylesheet loads asynchronously via `<link rel="preload">`.
4. That full stylesheet is now a bundled, tree-shaken file -- smaller
   than the original set of stylesheets.

**Goals:**

- Merge multiple `<link rel="stylesheet">` references to local CSS
  files into a single bundled CSS file in `dist/`.
- Resolve `@import` chains during bundling so the output is
  self-contained.
- Remove CSS selectors that do not match any element in any rendered
  HTML page across the entire site (site-wide tree-shaking).
- Merge multiple `<script src="...">` references to local JS files
  into a single bundled JS file in `dist/`.
- Rewrite HTML to reference the bundled files instead of the originals.
- Integrate cleanly with existing features: critical CSS, content
  hashing, resource hints, and the dev server.
- Be opt-in and default to disabled for backward compatibility.

**Non-goals (see "Out of Scope"):**

- JavaScript tree-shaking (requires semantic analysis of JS code).
- CSS or JS module resolution beyond `@import` and file concatenation.
- Processing of CDN-hosted stylesheets or scripts.
- Bundling across different `<head>` / `<body>` insertion points.

## Architecture

### Approach: Two-Phase Post-Render Bundling

CSS/JS bundling is a **site-wide** operation: it must see all rendered
pages to determine which CSS selectors are used globally. This
fundamentally differs from per-page transforms like critical CSS
inlining. Therefore, bundling runs as a **post-render phase** in the
build pipeline, after all pages have been rendered and written to disk,
but before content hashing rewrites references.

**Phase overview:**

1. **All pages rendered and written to disk** (existing Phase 2).
2. **CSS/JS bundling and tree-shaking** (NEW Phase 2.5):
   a. Scan all rendered HTML files to collect stylesheet and script
      references.
   b. For CSS: merge referenced files, parse with lightningcss, match
      selectors against the union of all rendered HTML DOMs, serialize
      only the matched rules to a single output file.
   c. For JS: concatenate referenced files in document order, separated
      by semicolons and newlines, to a single output file.
   d. Rewrite all HTML files to replace original `<link>`/`<script>`
      tags with references to the bundled files.
3. **Content hashing** (existing Phase 3) -- hashes the bundled files
   along with everything else.

**Why post-render, not pre-render?**

Three reasons:

1. **Template rendering produces the HTML that determines which CSS
   selectors are used.** We cannot know which selectors to keep until
   all pages are rendered. Running bundling before rendering would
   require a speculative approach (keep everything, or maintain a
   separate selector inventory).

2. **Asset localization and plugin hooks may modify stylesheets.** The
   per-page pipeline localizes external assets and runs plugin hooks
   that can create or modify CSS files. Bundling must operate on the
   final state of CSS files in `dist/`.

3. **Content hashing is already a post-render phase.** The existing
   `build()` function has a clear Phase 3 that walks `dist/` and
   rewrites references. Bundling slots naturally before this as
   Phase 2.5. Both phases walk dist/ and rewrite HTML files, so
   ordering them is trivial.

**Why not inside the per-page pipeline?**

The per-page pipeline (`render_static_page` / `render_dynamic_page`)
processes one page at a time. CSS bundling requires a global view:
to tree-shake, you need the union of all selectors used across all
pages. A per-page approach could bundle but not tree-shake, and would
need to re-read and re-merge the same CSS files for every page. The
post-render approach reads each CSS file once, builds the selector
union once, and outputs a single bundled file.

**Why not a plugin?**

Bundling is a core build optimization that requires filesystem access
to `dist/`, knowledge of the build pipeline ordering, and tight
integration with content hashing. The plugin `post_build` hook runs
after content hashing, which is too late. A new plugin hook could be
added, but this would be artificial complexity for what is fundamentally
a core feature. The same reasoning applied to critical CSS (see its
design spec).

### Pipeline Position

The current `build()` function in `src/build/render.rs`:

```
Phase 1: Copy static assets (+ content hash manifest if enabled)
Phase 2: Render all pages (per-page pipeline: template -> fragment
          markers -> localize -> images -> CSS backgrounds -> plugins
          -> critical CSS -> hints -> minify -> write to disk)
Phase 2 (cont): Generate sitemap, run post_build plugins
Phase 3: Content hash rewrite (walk dist/, string-replace references)
```

With bundling:

```
Phase 1: Copy static assets (+ content hash manifest if enabled)
Phase 2: Render all pages (per-page pipeline unchanged)
Phase 2 (cont): Generate sitemap, run post_build plugins
Phase 2.5: CSS/JS bundling and tree-shaking  <-- NEW
Phase 3: Content hash rewrite
```

**Rationale for placing 2.5 after post_build plugins:**

Post-build plugins may generate or modify CSS/JS files in `dist/`.
For example, a Tailwind plugin might generate a CSS file during
`post_build`. Bundling must see these files. Placing it after
`post_build` ensures all CSS/JS files are in their final state.

**Rationale for placing 2.5 before content hashing:**

Content hashing renames files in dist/ and rewrites references. If
bundling ran after content hashing, it would need to resolve hashed
filenames back to originals -- unnecessary complexity. By running
before, bundling works with original filenames. Content hashing then
hashes the bundled output file along with everything else.

### Interaction with Critical CSS

Critical CSS inlining runs during the per-page pipeline (Phase 2),
before bundling (Phase 2.5). This means critical CSS currently operates
on the original, unbundled stylesheets. After bundling rewrites the
HTML, the `<link>` tags point to the bundled file.

Note: The `inline_critical_css` function signature (in
`src/build/critical_css/mod.rs`) takes five parameters, including an
optional `&AssetManifest` for content-hash resolution:

```rust
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
    css_cache: &mut StylesheetCache,
    manifest: Option<&AssetManifest>,
) -> String
```

This is correct and intentional:

- **With both features enabled:** Critical CSS inlines per-page used
  CSS from the original stylesheets. Bundling then replaces the
  `<link>` tags with a reference to the bundled file. If critical CSS
  has `preload_full = true`, the preload `<link>` will be rewritten to
  point to the bundled file. The preloaded file is now smaller (tree-
  shaken), which is the optimal outcome.

- **With only bundling enabled:** Pages get a single `<link>` to the
  bundled, tree-shaken stylesheet. No inlining.

- **With only critical CSS enabled:** Pages get inlined critical CSS
  from the original stylesheets. No bundling.

The bundling rewrite step must handle the preload pattern that critical
CSS produces. Specifically, when it encounters:

```html
<link rel="preload" href="/css/style.css" as="style" onload="...">
<noscript><link rel="stylesheet" href="/css/style.css"></noscript>
```

It must rewrite both the preload and the noscript `<link>` hrefs to
point to the bundled file. The `<style>` block with inlined critical
CSS is left untouched.

**Implementation detail for the rewriter:** The `lol_html` element
selectors must match both `link[rel='stylesheet']` and
`link[rel='preload'][as='style']` to handle the critical CSS preload
pattern. The noscript fallback `<link>` is inside a `<noscript>` tag,
which `lol_html` processes as text content, not as parsed HTML. The
rewrite step must use string replacement within noscript content rather
than element-level rewriting for these tags.

### Interaction with Content Hashing

Content hashing is a Phase 3 post-render string replacement. The
bundled output file is written to `dist/` during Phase 2.5. During
Phase 3, content hashing:

1. Sees the bundled CSS/JS files as new files in `dist/` (they were
   created by bundling, not copied from `static/`).
2. Hashes and renames them.
3. Rewrites references in HTML files.

However, the current `build_manifest` only walks files that exist in
both `static/` and `dist/`. Bundled files are created in `dist/` but
have no counterpart in `static/`. We need to extend the manifest
building to also include files from a "generated assets" list provided
by the bundling step.

**Solution:** The bundling step returns a list of generated file paths
(relative to dist/). The content hash phase includes these files in
its manifest. This is a small addition to `build_manifest`'s signature.

### Interaction with Resource Hints

Resource hints (`<link rel="prefetch">` for fragments) are injected
during the per-page pipeline. These reference fragment files, not CSS
or JS files, so bundling does not affect them.

Hero image preload hints (`<link rel="preload" as="image">`) are also
unaffected -- they reference image files.

The only interaction is if a hint references a CSS file. This does not
happen in the current hints implementation.

### Dependencies

**`lightningcss`** is already a dependency (version `1.0.0-alpha.71`
in `Cargo.toml`). It provides CSS parsing, AST walking, selector
serialization, and `@import` bundling. The critical CSS feature
already uses it. Note: The version is a pre-release alpha; API
changes between alpha releases are possible. The implementation
should be tested against the exact version in `Cargo.toml`.

**`scraper`** is already a dependency (`Cargo.toml` line 37). It
provides HTML parsing and CSS selector matching against a DOM tree.
The critical CSS feature already uses it.

**`lol_html`** is already a dependency. It provides streaming HTML
rewriting for replacing `<link>` and `<script>` tags.

**No new dependencies are needed.** This feature reuses the same
libraries as critical CSS.

## Data Models and Types

### Configuration

Add to `src/config/mod.rs`:

```rust
/// Configuration for CSS/JS bundling and tree-shaking.
///
/// Located under `[build.bundling]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct BundlingConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Whether to bundle CSS files. Default: true (when bundling is
    /// enabled).
    #[serde(default = "default_true")]
    pub css: bool,

    /// Whether to tree-shake CSS (remove unused selectors). Default:
    /// true. Only applies when css bundling is enabled.
    /// When false, CSS files are merged but no selectors are removed.
    #[serde(default = "default_true")]
    pub tree_shake_css: bool,

    /// Whether to bundle JS files. Default: true (when bundling is
    /// enabled).
    #[serde(default = "default_true")]
    pub js: bool,

    /// Output filename for the bundled CSS file.
    /// Written to `dist/{css_output}`.
    /// Default: "css/bundle.css".
    #[serde(default = "default_css_output")]
    pub css_output: String,

    /// Output filename for the bundled JS file.
    /// Written to `dist/{js_output}`.
    /// Default: "js/bundle.js".
    #[serde(default = "default_js_output")]
    pub js_output: String,

    /// Glob patterns for stylesheet paths to exclude from bundling.
    /// Matched against the href value. Excluded stylesheets remain
    /// as separate `<link>` tags in the HTML.
    /// Example: ["**/vendor/**", "**/print.css"]
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

Add the field to `BuildConfig`:

```rust
pub struct BuildConfig {
    // ... existing fields ...

    /// CSS/JS bundling and tree-shaking configuration.
    #[serde(default)]
    pub bundling: BundlingConfig,
}
```

Update the `Default` impl for `BuildConfig` to include:

```rust
impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            // ... existing fields ...
            bundling: BundlingConfig::default(),
        }
    }
}
```

### Internal Types

These types live inside `src/build/bundling/` and are not part of the
public API.

```rust
/// Collected references from all rendered HTML files.
///
/// Built by scanning dist/ for HTML files and extracting `<link>` and
/// `<script>` references.
struct CollectedRefs {
    /// Ordered list of unique local CSS hrefs found across all HTML
    /// files, in first-encounter order.
    /// Example: ["/css/reset.css", "/css/style.css", "/css/components.css"]
    css_hrefs: Vec<String>,

    /// Ordered list of unique local JS srcs found across all HTML
    /// files, in first-encounter order.
    js_srcs: Vec<String>,

    /// Paths to all HTML files in dist/ (both full pages and
    /// fragments). Full pages are scanned for CSS/JS references
    /// during collection. All files (including fragments) are
    /// rewritten during the rewrite phase. Fragments typically
    /// do not contain `<link>` tags but may contain `<script>` tags.
    html_files: Vec<std::path::PathBuf>,
}

/// Result of the bundling operation.
struct BundleResult {
    /// Path to the generated CSS bundle file (relative to dist/).
    /// None if no CSS was bundled.
    css_bundle: Option<String>,

    /// Path to the generated JS bundle file (relative to dist/).
    /// None if no JS was bundled.
    js_bundle: Option<String>,

    /// Mapping from original href/src to bundled href/src.
    /// Used for rewriting HTML references.
    /// Example: {"/css/reset.css" => "/css/bundle.css",
    ///           "/css/style.css" => "/css/bundle.css"}
    rewrite_map: HashMap<String, String>,

    /// Paths of generated bundle files (relative to dist/) that
    /// should be included in content hashing.
    generated_files: Vec<String>,
}
```

The design deliberately avoids creating intermediate CSS AST types.
We work directly with lightningcss's `StyleSheet` and `CssRule` types,
reusing the same approach as the critical CSS module in
`src/build/critical_css/extract.rs`.

## API Surface

### Public Function

The module `build::bundling` exposes a single public function:

```rust
/// Bundle CSS and JS files in dist/ and rewrite HTML references.
///
/// This is the main entry point called from the build pipeline after
/// all pages have been rendered and written to disk.
///
/// Steps:
/// 1. Scan all HTML files in dist/ for <link> and <script> tags.
/// 2. Collect and deduplicate referenced local CSS/JS files.
/// 3. For CSS: merge, tree-shake, write bundled file.
/// 4. For JS: concatenate, write bundled file.
/// 5. Rewrite all HTML files to reference bundled files.
///
/// `minify_css` controls whether the bundled CSS output is minified
/// via lightningcss's `PrinterOptions { minify: true }`. This is
/// separate from the per-page `minify-html` pass: lightningcss
/// handles standalone CSS minification (the bundled file), while
/// `minify-html` handles inline CSS/JS within HTML.
///
/// Typically set to `config.build.minify` (the same flag that
/// controls HTML minification), since sites that want minified HTML
/// also want minified CSS bundles.
///
/// Returns the list of generated file paths (relative to dist/)
/// for content hashing integration.
pub fn bundle_assets(
    dist_dir: &Path,
    config: &BundlingConfig,
    minify_css: bool,
) -> Result<Vec<String>>
```

**Return type rationale:** Returns `Result<Vec<String>>` rather than
being infallible. Unlike critical CSS (which is a per-page
optimization where failure just skips one page), bundling is a site-
wide transformation. If bundling partially fails (e.g., writes the
bundled file but cannot rewrite HTML), the site is in an inconsistent
state. Returning `Result` lets the caller decide whether to fail the
build or continue. The returned `Vec<String>` contains relative paths
to generated bundle files, needed by content hashing.

### Internal Functions

```rust
/// Scan all HTML files in dist/ and collect <link rel="stylesheet">
/// hrefs and <script src="..."> srcs.
///
/// Returns deduplicated lists in first-encounter order across all
/// pages. Also returns the list of HTML file paths for rewriting.
///
/// Skips:
/// - External URLs (http://, https://)
/// - `<link>` tags with a `media` attribute (e.g. print stylesheets,
///   consistent with critical CSS behavior)
/// - `<script>` tags with `defer`, `async`, or `type="module"`
/// - `<script>` tags without a `src` attribute (inline scripts)
/// - Paths matching the `exclude` glob patterns
///
/// HTML files are sorted by path before processing to ensure
/// deterministic merge order (walkdir does not sort by default).
fn collect_references(
    dist_dir: &Path,
    exclude: &[String],
) -> Result<CollectedRefs>

/// Read and merge multiple CSS files into a single string.
///
/// Files are concatenated in the order given (which is the order
/// they were first encountered in HTML). @import directives within
/// each file are resolved and inlined.
///
/// Note: The `@import` resolution logic currently lives in
/// `critical_css/mod.rs` as the private functions `load_stylesheet`
/// and `resolve_imports`. To reuse this logic, either:
/// (a) extract the import resolution into a shared utility module
///     (e.g. `build/css_utils.rs`), or
/// (b) make `load_stylesheet` public in `critical_css/mod.rs`.
/// Option (a) is preferred because `load_stylesheet` also handles
/// manifest-based path resolution, which bundling does not need
/// (bundling runs before content hashing).
///
/// Each file's content is preceded by a comment marking its origin:
/// `/* Source: /css/reset.css */`
fn merge_css_files(
    hrefs: &[String],
    dist_dir: &Path,
) -> Result<String>

/// Tree-shake CSS: remove selectors that do not match any element
/// in any rendered HTML page.
///
/// Steps:
/// 1. Parse every HTML file into a scraper::Html DOM.
/// 2. Parse the merged CSS with lightningcss.
/// 3. Walk all CSS rules. For each style rule, test its selectors
///    against every DOM. If a selector matches in at least one
///    DOM, keep the rule.
/// 4. Include transitively referenced @font-face, @keyframes,
///    and custom property rules (same logic as critical CSS
///    extract.rs).
/// 5. Serialize the surviving rules to a string (minified if
///    `minify` is true).
///
/// Returns the tree-shaken CSS.
fn tree_shake_css(
    merged_css: &str,
    html_files: &[std::path::PathBuf],
    minify: bool,
) -> Result<String>

/// Read and concatenate multiple JS files into a single string.
///
/// Files are concatenated in order, each terminated with a semicolon
/// and newline to prevent ASI issues. Each file is preceded by a
/// source comment: `// Source: /js/utils.js`
fn merge_js_files(
    srcs: &[String],
    dist_dir: &Path,
) -> Result<String>

/// Write the bundled CSS/JS files to dist/ and rewrite all HTML
/// files to reference the bundled versions.
///
/// For each HTML file:
/// 1. Replace the FIRST <link> / <script> tag that references a
///    bundled file with a reference to the bundle.
/// 2. Remove subsequent <link> / <script> tags that reference
///    other bundled files (they are now in the bundle).
/// 3. Handle the critical CSS preload pattern: rewrite both the
///    preload <link> and the <noscript> fallback <link>.
///
/// Returns the BundleResult with rewrite mapping and generated
/// file paths.
fn rewrite_html_for_bundles(
    html_files: &[std::path::PathBuf],
    dist_dir: &Path,
    css_bundle_href: Option<&str>,
    js_bundle_src: Option<&str>,
    css_hrefs: &[String],
    js_srcs: &[String],
) -> Result<()>
```

### Reference Collection Strategy

The collector scans HTML files in dist/ and extracts references using
`lol_html`. The ordering is important: stylesheets and scripts must
be merged in the order they are referenced, because CSS cascade order
and JS execution order matter.

**Ordering rule:** The first HTML file that references a given href
determines its position in the merge order. Subsequent references to
the same href in other HTML files are deduplicated. The order of HTML
files themselves must be deterministic: `walkdir` does NOT sort by
default (iteration order depends on the filesystem), so the collector
must explicitly sort HTML file paths before processing. Use
`walkdir::WalkDir::new(dist_dir).sort_by_file_name()` or collect and
sort the paths after walking.

This ordering is correct because in a typical static site, all pages
share the same base template with the same `<link>` and `<script>`
ordering. If page A has `[reset.css, style.css]` and page B has
`[reset.css, style.css, components.css]`, the merge order is
`[reset.css, style.css, components.css]`.

**Edge case: conflicting order.** If page A has `[a.css, b.css]` and
page B has `[b.css, a.css]`, we use the order from whichever page is
encountered first (sorted by path). This is a degenerate case that
indicates a template inconsistency. A warning is logged.

### CSS Tree-Shaking Strategy

The tree-shaking approach reuses the selector-matching infrastructure
from `critical_css/extract.rs`. The key difference is scope:

- **Critical CSS:** matches selectors against ONE page's DOM.
- **Tree-shaking:** matches selectors against the UNION of ALL pages'
  DOMs. A selector is kept if it matches in ANY page.

This means the tree-shaken output is the union of all pages' critical
CSS, which is strictly smaller than or equal to the original merged CSS,
but larger than any single page's critical CSS.

**Implementation approach:**

1. Parse all HTML files in dist/ into `scraper::Html` DOMs.
2. Parse the merged CSS with `lightningcss` (error recovery enabled).
3. Walk rules using the same recursive `walk_rules` pattern from
   `critical_css/extract.rs`, but instead of matching against one
   document, match against all documents.
4. A style rule is kept if ANY of its selectors (after pseudo-
   stripping) matches ANY element in ANY document.
5. Global rules (@font-face, @keyframes, custom properties) are kept
   based on transitive references from kept style rules, using the
   same `GlobalDependencies` tracking from `extract.rs`.
6. @layer statements, @import, @namespace are always kept.
7. @media, @supports, @layer blocks are walked recursively: kept if
   any child rule matches.

**Memory consideration:** Step 1 loads all HTML files into memory as
parsed DOM trees simultaneously. For a site with thousands of pages,
this could consume significant memory. If memory becomes a problem,
the implementation can be restructured to:
(a) Collect all unique CSS selectors from the stylesheet first,
(b) Stream HTML files one at a time, marking selectors as "used" when
    they match,
(c) After all HTML files are processed, remove rules with no "used"
    selectors.
This streaming approach trades CPU time (re-parsing selectors per
file) for memory. The initial implementation should use the simpler
"parse all DOMs" approach and optimize only if profiling shows memory
is a concern for real-world sites.

**Reuse of critical CSS code:** Rather than duplicating the selector-
matching and rule-walking logic, we factor out shared functions from
`critical_css/` that both modules can call. Specifically:

- `strip_pseudo_for_matching` -- already public in `extract.rs`.
- `selector_matches` -- already public in `extract.rs`.
- `collect_global_deps` -- already public in `extract.rs`.
- `GlobalDependencies` -- already public in `extract.rs`.
- `extract_stylesheet_hrefs` -- already public in `rewrite.rs` (not
  `extract.rs`; the spec must reference the correct file).
- The `walk_rules` and `handle_style_rule` pattern -- these are
  private in `extract.rs`. Rather than making them public (they are
  tightly coupled to critical CSS's data flow), the bundling module
  implements its own `walk_rules` variant that calls the shared public
  functions but matches against multiple documents instead of one.

This avoids both code duplication (shared primitives) and tight
coupling (each module has its own orchestration logic).

### JS Concatenation Strategy

JS bundling is deliberately simple: file concatenation with semicolons.
This is not a module bundler -- it does not resolve `import` statements
or create a module graph. It is equivalent to the manual process of
maintaining a single concatenated JS file.

Each file is wrapped in an IIFE (Immediately Invoked Function
Expression) to prevent variable scope pollution:

```js
// Source: /js/utils.js
;(function(){
/* original file content */
})();
```

The leading semicolon is defensive -- it prevents issues if a previous
file's content ends unexpectedly. The IIFE prevents `var` and function
declarations from leaking into the global scope.

If a file uses `'use strict';` at the top level, the strict mode
applies within the IIFE, which is correct behavior.

**When not to bundle JS:** Files loaded with `defer`, `async`, or
`type="module"` attributes should NOT be bundled because:

- `defer` / `async`: the file is already non-blocking; bundling it
  with blocking scripts changes its execution semantics.
- `type="module"`: ES modules have their own scoping; wrapping them
  in an IIFE is incorrect.

These are excluded from collection.

### HTML Rewriting Strategy

After writing the bundled files, all HTML files in dist/ (including
fragments in `_fragments/`) must be rewritten. The rewriting uses
`lol_html` for streaming HTML processing.

For each HTML file:

**CSS rewriting:**

1. Track whether the bundle `<link>` has been injected.
2. For each `<link rel="stylesheet">` whose href is in the set of
   bundled hrefs:
   - If this is the first such link: replace its href with the bundle
     href.
   - If this is a subsequent link: remove it.
3. For each `<link rel="preload" as="style">` whose href is in the
   bundled set (critical CSS preload pattern):
   - Rewrite the href to the bundle href (first occurrence) or remove
     (subsequent).
4. For each `<noscript>` containing a `<link rel="stylesheet">` whose
   href is in the bundled set:
   - Rewrite the href to the bundle href (first) or remove (subsequent).
   - **Important:** `lol_html` treats `<noscript>` content as raw text,
     not as parsed HTML elements. The `<link>` tags inside `<noscript>`
     cannot be matched via element selectors. Instead, use a `text!`
     handler on `noscript` elements and perform string replacement on
     the text content. This is the same approach used by the critical
     CSS rewrite module (`critical_css/rewrite.rs`).

**JS rewriting:**

1. Track whether the bundle `<script>` has been injected.
2. For each `<script src="...">` whose src is in the bundled set:
   - If first: replace its src with the bundle src.
   - If subsequent: remove it.
3. Skip `<script>` tags with `defer`, `async`, or `type="module"`.

**Fragment rewriting:**

Fragments are partial HTML that do not contain `<head>` or `<link>`
tags for CSS. However, they may contain `<script>` tags. The rewriting
applies to all HTML files uniformly. If a fragment does not reference
any bundled files, the rewriting is a no-op.

**Caveat for fragment `<script>` tags:** If a fragment references
`<script src="/js/utils.js">` and that file is bundled, the rewrite
replaces it with `<script src="/js/bundle.js">`. The full page that
hosts the fragment likely already loaded `bundle.js` via its own
`<script>` tag, so the browser serves it from cache. However, this
means the fragment triggers a cache lookup for the full bundle even
if it only needed `utils.js`. For typical static sites with small JS
payloads, this is acceptable. Sites with large JS payloads should
exclude fragment-specific scripts from bundling via the `exclude`
config.

## Error Handling

| Scenario | Behavior |
|---|---|
| CSS file referenced in HTML not found in dist/ | Log warning, skip that file, continue with others |
| CSS parse error in lightningcss | Log warning with file path and error, skip that file |
| JS file referenced in HTML not found in dist/ | Log warning, skip that file, continue with others |
| HTML parse error in lol_html rewriting | Return error (site is in inconsistent state) |
| All CSS files fail to load | Skip CSS bundling entirely, log warning |
| All JS files fail to load | Skip JS bundling entirely, log warning |
| Conflicting script/link ordering across pages | Log warning, use first-encountered order |
| Circular @import in CSS | Handled by the shared `resolve_imports` depth limit (max 10 levels, from `critical_css/mod.rs`, to be extracted into shared utility) |
| Empty merged CSS after tree-shaking | Write empty bundle file (valid CSS), log info |
| dist/ directory is empty (no HTML files) | No-op, return empty Vec |
| Cannot write bundle file to dist/ | Return error (filesystem issue) |
| No local CSS/JS references found in any HTML | No-op, return empty Vec |

**Principle:** CSS/JS bundling is a site-wide transformation. Partial
failures (one CSS file missing) are handled gracefully by skipping the
problematic file. Complete failures (cannot write to dist/) propagate
as errors because the site would be in an inconsistent state. This
differs from critical CSS (which is infallible) because bundling modifies
shared output files rather than individual pages.

## Edge Cases and Failure Modes

### Pages with different stylesheet sets

Page A may reference `[reset.css, style.css]` while page B references
`[reset.css, style.css, blog.css]`. The bundled CSS file contains the
union of all referenced stylesheets: `[reset.css, style.css, blog.css]`.
Tree-shaking ensures that selectors from `blog.css` that are only used
on page B are kept (they match elements in page B's DOM), while
selectors used by no page at all are removed.

### Single CSS/JS file across the site

If only one CSS file (e.g., `style.css`) is referenced across all
pages, bundling produces a single-file bundle. This is still valuable
because tree-shaking removes unused selectors from that file. The
rewrite replaces the original `<link>` href with the bundle href.
If tree-shaking is disabled (`tree_shake_css = false`), the bundle is
a copy of the original file with no benefit -- but this is harmless
(the `css_output` path is different from the original, so no data is
lost). An optimization could detect this case and skip writing, but
the added complexity is not justified.

### Pages with no `<link>` or `<script>` tags

Such pages are scanned but produce no references. The rewriting step
is a no-op for them.

### External stylesheets and scripts

`<link>` tags with `http://` or `https://` hrefs are ignored by the
collector. They remain in the HTML unchanged. Only local files (paths
starting with `/` that resolve to files in dist/) are bundled.

External scripts (`<script src="https://...">`) are similarly ignored.

### Inline `<style>` and `<script>` blocks

Existing inline `<style>` blocks are left unchanged. They are not
bundled. Inline `<script>` blocks (no `src` attribute) are also left
unchanged.

If a `<script>` tag has both a `src` attribute AND inline content,
per the HTML spec the inline content is ignored and only the external
file is loaded. The collector considers this a normal external script
(collects the `src`). During rewriting, the `src` is replaced with
the bundle path; the inline content (if any) is left as-is (browsers
ignore it anyway).

### CSS with `@charset` and `@namespace`

If multiple CSS files declare `@charset`, only the first one is kept
(per CSS spec, `@charset` must be the very first thing in a stylesheet).
`@namespace` rules are kept as-is. lightningcss handles these correctly
during parsing.

### Minification of bundled files

Per-page HTML minification runs during Phase 2 (per-page pipeline),
before bundling (Phase 2.5). The per-page `minify-html` pass minifies
inline `<style>` and `<script>` blocks within each page, but it does
not process external CSS/JS files.

The bundled CSS/JS files written during Phase 2.5 are NOT automatically
minified by the per-page pipeline. However, lightningcss can produce
minified output by setting `PrinterOptions { minify: true }` when
serializing the tree-shaken CSS. We use this to produce minified CSS
bundles when `config.build.minify` is true.

For JS bundles, each file is wrapped in an IIFE (adding ~20 bytes of
overhead per file: `;(function(){\n` and `\n})();\n`). The file content
itself is passed through as-is. `minify-html` includes a JS minifier,
but it is designed for inline script blocks, not standalone files. We
do NOT add a separate JS minification step. Users who want minified JS
should pre-minify their source files or use an external tool. This
matches the project's philosophy of using existing libraries rather
than building a JS toolchain.

After bundling rewrites the HTML files, the `<link>` and `<script>`
tags reference the bundled files. These rewritten HTML files were
already minified during Phase 2. The `lol_html` rewriting largely
preserves the minified state -- it does not add unnecessary whitespace.
However, element removal (dropping subsequent `<link>`/`<script>` tags)
may leave minor whitespace artifacts. This is acceptable: the size
impact is negligible (a few bytes) and a second `minify-html` pass
would be wasteful for this marginal improvement.

### CSS source maps

Bundled CSS does not include source maps. The source comments
(`/* Source: /css/style.css */`) provide basic traceability for
debugging. These comments are stripped when CSS minification is
enabled (lightningcss's minifier removes comments). Full source map
support is out of scope.

### Duplicate selectors across files

If `reset.css` and `style.css` both define `body { margin: 0; }`, both
rules appear in the merged CSS. lightningcss does not deduplicate
identical rules (doing so could change cascade behavior when
specificity differs). The existing `minify-html` CSS minifier may
merge identical adjacent rules during minification.

### Interaction with `asset()` template function

The `asset()` template function resolves paths through the content hash
manifest at render time. Since bundling runs after rendering, `asset()`
references in rendered HTML still point to original filenames.

The bundling step does not affect `asset()` behavior. Content hashing
(Phase 3) will hash the bundled files and rewrite the references that
bundling inserted.

### Interaction with critical CSS stylesheet cache

The critical CSS `StylesheetCache` caches raw CSS content keyed by
href. Since critical CSS runs during Phase 2 (per-page rendering) and
bundling runs during Phase 2.5 (post-render), the cache is not shared.
The bundling module reads CSS files directly from dist/ without using
the critical CSS cache. This is correct: the cache is a per-render
optimization, and bundling is a separate phase.

### Dev server

CSS/JS bundling is **disabled during dev server** regardless of config.
The dev server (`src/dev/rebuild.rs`) has its own render functions that
do not call the post-render phases. This is consistent with critical
CSS and content hashing, both of which are also disabled during dev.

### Files referenced by only some pages

If `blog.css` is referenced by blog pages but not the homepage, it is
still included in the bundle (it is part of the site's CSS). Tree-
shaking removes selectors from `blog.css` that are not used by any
page. The homepage downloads the full bundle, but the unused `blog.css`
selectors have been removed by tree-shaking, so the extra weight is
minimal.

This is a tradeoff: one bundle file means one HTTP request and better
caching, but pages download CSS they don't use. With tree-shaking, the
unused portion is minimized. For sites where this tradeoff is
unacceptable (e.g., vastly different CSS per section), users can
disable bundling or exclude certain stylesheets.

### Output path conflict with existing static files

If `css_output` or `js_output` matches a file that already exists in
`dist/` (e.g., the user has `static/css/bundle.css`), the bundling
step overwrites it. This is intentional: the user has configured
bundling to output to that path. A warning should be logged if the
output path already exists before writing, to alert the user that
their static file is being replaced. If this is undesirable, the user
should change `css_output` or `js_output` to a non-conflicting path.

### Original CSS/JS files in dist/

After bundling, the original CSS/JS files remain in dist/. They are
not deleted because:

1. They may be referenced by other means (e.g., JavaScript dynamic
   imports, CSS `@import` in inline styles).
2. Content hashing will hash them if they came from `static/`.
3. Deleting them could break other post-build tools.

This means some unused files may remain in dist/. This is acceptable
for a static site generator -- the deploy process can use standard
tools to prune unreferenced files if needed.

### Multiple bundle groups

The current design produces one CSS bundle and one JS bundle. All
local stylesheets and scripts are merged into these two files. There
is no concept of "entry points" or "code splitting."

For the vast majority of static sites, a single bundle is optimal:
one request, maximum compression, trivial caching. Code splitting is
a concern for large SPAs, not static sites.

If a future need arises (e.g., per-section bundles), the architecture
supports it via the exclude patterns: excluding certain files from
bundling lets users manually create logical groups.

## Configuration Examples

### Minimal (opt-in)

```toml
[build.bundling]
enabled = true
```

### CSS only (no JS bundling)

```toml
[build.bundling]
enabled = true
js = false
```

### CSS bundling without tree-shaking

```toml
[build.bundling]
enabled = true
tree_shake_css = false
```

### Custom output paths

```toml
[build.bundling]
enabled = true
css_output = "assets/styles.css"
js_output = "assets/scripts.js"
```

### Exclude vendor files from bundling

```toml
[build.bundling]
enabled = true
exclude = ["**/vendor/**", "**/print.css"]
```

### Full configuration with all features

```toml
[build.bundling]
enabled = true
css = true
tree_shake_css = true
js = true
css_output = "css/bundle.css"
js_output = "js/bundle.js"
exclude = ["**/vendor/**"]

[build.critical_css]
enabled = true

[build.content_hash]
enabled = true
```

## What Is NOT In Scope

1. **JavaScript tree-shaking.** True JS tree-shaking requires
   understanding module exports, side effects, and the JS dependency
   graph. Tools like esbuild, rollup, and webpack implement this with
   tens of thousands of lines of code. There is no Rust-native JS
   tree-shaking library of lightningcss's quality. JS concatenation
   (our approach) is simple, correct, and sufficient for static sites
   that use small utility scripts.

2. **ES Module bundling.** `<script type="module">` tags are excluded
   from bundling because modules have their own scoping and import
   resolution. Wrapping them in IIFEs would break module semantics.

3. **Standalone CSS/JS minification.** The existing `minify-html`
   step minifies inline CSS and JS within HTML (in `<style>` and
   `<script>` blocks). It does NOT process external CSS/JS files --
   only the HTML string it receives. The bundled CSS file IS
   minified via lightningcss's `PrinterOptions { minify: true }` (as
   described in "Minification of bundled files" above). No separate
   JS minification step is added; users who want minified JS should
   pre-minify their source files.

4. **Source map generation.** Source maps add complexity for a feature
   primarily useful in development. The dev server does not run
   bundling, so source maps would only exist in production builds
   where they are less needed.

5. **Dynamic `import()` resolution.** Dynamic imports in JS are
   runtime constructs that cannot be statically analyzed in a simple
   concatenation bundler.

6. **Per-page bundles or code splitting.** The design produces one
   CSS and one JS bundle per site. Per-page bundles would require
   tracking which files each page uses and generating N bundle files,
   which is unnecessary complexity for static sites. Critical CSS
   already provides per-page CSS optimization.

7. **Processing of CDN-hosted assets.** Only local files in dist/ are
   bundled. CDN URLs are left as-is.

8. **CSS `@import` in inline `<style>` blocks.** Inline styles are
   left unchanged. Only external `<link>` stylesheets are bundled.

9. **Watching for CSS/JS changes during dev.** Bundling is disabled
   in dev mode. The dev server serves original files directly.

## Module Structure

```
src/build/
  bundling/
    mod.rs          -- public API (bundle_assets), orchestration
    collect.rs      -- HTML scanning, reference collection
    css.rs          -- CSS merging, tree-shaking (uses lightningcss,
                       scraper, and shared functions from
                       critical_css/extract.rs)
    js.rs           -- JS concatenation with IIFE wrapping
    rewrite.rs      -- HTML rewriting (lol_html-based, replaces
                       <link> and <script> tags)
```

Five files because the concerns are distinct:
- `mod.rs`: orchestration, config checks, file writing (~100 lines)
- `collect.rs`: lol_html-based HTML scanning for references (~80 lines)
- `css.rs`: CSS merging and tree-shaking (~200 lines, reusing
  primitives from critical_css/extract.rs)
- `js.rs`: JS concatenation with IIFE wrapping (~50 lines)
- `rewrite.rs`: lol_html-based HTML rewriting (~120 lines)

This follows the pattern established by `src/build/critical_css/`
which splits concerns across submodules.

## Integration Points

### build/mod.rs

Add module declaration:

```rust
pub mod bundling;     // <-- NEW
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

### build/render.rs

In `build()`, after post-build plugin hooks and before content hashing:

```rust
// Phase 2 (cont): post-build plugins.
plugin_registry.post_build(&dist_dir, project_root)?;

// Phase 2.5: CSS/JS bundling and tree-shaking.
let bundled_files = if config.build.bundling.enabled {
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

// Phase 3: Content hash rewrite.
// Note: The current code checks `config.build.content_hash.enabled &&
// !manifest.is_empty()`. With bundling, we must also trigger the
// rewrite when bundle files need hashing, even if no static assets
// were hashed (manifest is empty but bundled_files is not).
if config.build.content_hash.enabled {
    // Hash bundled files (generated, not from static/).
    let bundle_manifest = if !bundled_files.is_empty() {
        Some(content_hash::hash_additional_files(
            &dist_dir, &bundled_files,
        )?)
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

Note: The existing `manifest` remains in its `Arc` (shared with the
template engine). The `bundle_manifest` is a separate, short-lived
`AssetManifest` created after rendering is complete.

### build/content_hash.rs

The current `build_manifest` walks `static/` and hashes corresponding
files in `dist/`. Bundled files exist only in `dist/` (no `static/`
counterpart). The existing manifest is wrapped in `Arc` at Phase 1
and cannot be mutated after that point (the `Arc` is shared with the
template engine's `asset()` function closure, which lives until the
end of `build()`'s scope).

The current `rewrite_references` signature takes only two parameters:

```rust
// CURRENT signature (will be changed by this feature):
pub fn rewrite_references(dist_dir: &Path, manifest: &AssetManifest) -> Result<()>
```

**Solution:** Add a new `hash_additional_files` function that creates a
separate `AssetManifest` for generated files. Change the existing
`rewrite_references` function to accept an additional optional manifest
for generated files. This is a **breaking change** to the function
signature; the call site in `build()` in `render.rs` must be updated.

```rust
/// NEW function: Hash and rename additional generated files (e.g.,
/// CSS/JS bundles) and return a manifest of their original-to-hashed
/// path mappings.
///
/// These files exist only in dist/ (no static/ counterpart). The
/// returned manifest is used alongside the main manifest during
/// reference rewriting.
pub fn hash_additional_files(
    dist_dir: &Path,
    relative_paths: &[String],
) -> Result<AssetManifest>
```

Update `rewrite_references` to accept an optional additional manifest:

```rust
// CHANGED signature (new `additional` parameter):
pub fn rewrite_references(
    dist_dir: &Path,
    manifest: &AssetManifest,
    additional: Option<&AssetManifest>,
) -> Result<()>
```

The existing call site in `build()` changes from:

```rust
// BEFORE:
content_hash::rewrite_references(&dist_dir, &manifest)?;

// AFTER:
content_hash::rewrite_references(&dist_dir, &manifest, bundle_manifest.as_ref())?;
```

The function merges the two manifests' `pairs_longest_first()` outputs
for replacement ordering. This keeps the main manifest immutable in
its `Arc` while still supporting generated files.

### config/mod.rs

Add `BundlingConfig` struct and the `bundling` field to `BuildConfig`.
Add config tests for defaults and custom values following the existing
pattern.

### Cargo.toml

No new dependencies needed.

## Test Plan

Tests follow the project convention: test behavior, not library
internals. Do not use `unwrap`/`expect` unless it is an invariant.

### Unit tests in `collect.rs`

| Test | What it verifies |
|---|---|
| `test_collect_single_css` | Single `<link>` href is collected |
| `test_collect_multiple_css` | Multiple `<link>` hrefs in order |
| `test_collect_dedup_css` | Same href across two HTML files is deduplicated |
| `test_collect_skips_external` | `http://` and `https://` links are skipped |
| `test_collect_skips_media` | `<link>` with `media` attribute is skipped |
| `test_collect_skips_excluded` | Excluded patterns are honored |
| `test_collect_single_js` | Single `<script src>` is collected |
| `test_collect_skips_module_js` | `type="module"` scripts are skipped |
| `test_collect_skips_async_defer` | `async` and `defer` scripts are skipped |
| `test_collect_skips_inline_script` | `<script>` without `src` is skipped |
| `test_collect_no_html_files` | Empty dist/ returns empty CollectedRefs |
| `test_collect_deterministic_order` | Same dist/ contents produce same href/src ordering regardless of filesystem iteration order |

### Unit tests in `css.rs`

| Test | What it verifies |
|---|---|
| `test_merge_single_file` | One CSS file produces its content with source comment |
| `test_merge_multiple_files` | Multiple files concatenated in order |
| `test_merge_resolves_import` | `@import` in CSS is inlined |
| `test_merge_missing_file` | Missing file is skipped with warning |
| `test_tree_shake_removes_unused` | Selector not in any HTML is removed |
| `test_tree_shake_keeps_used` | Selector used in any page is kept |
| `test_tree_shake_keeps_pseudo` | `:hover` rule is kept if base matches |
| `test_tree_shake_media_query` | Matching rule inside `@media` is kept |
| `test_tree_shake_font_face` | `@font-face` kept when font-family is used |
| `test_tree_shake_keyframes` | `@keyframes` kept when animation is used |
| `test_tree_shake_empty_result` | No matching selectors produces empty CSS |
| `test_tree_shake_multiple_docs` | Selector in doc B but not doc A is kept |

### Unit tests in `js.rs`

| Test | What it verifies |
|---|---|
| `test_merge_single_js` | Single JS file wrapped in IIFE |
| `test_merge_multiple_js` | Multiple files each wrapped in IIFE |
| `test_merge_missing_js` | Missing file is skipped with warning |
| `test_iife_wrapping` | Output starts with `;(function(){` and ends with `})();` |

### Unit tests in `rewrite.rs`

| Test | What it verifies |
|---|---|
| `test_rewrite_css_single_link` | Single `<link>` href replaced with bundle href |
| `test_rewrite_css_multiple_links` | First link rewritten, subsequent removed |
| `test_rewrite_css_preload_pattern` | Critical CSS preload pattern rewritten correctly |
| `test_rewrite_css_noscript_pattern` | Noscript fallback link rewritten (via text replacement, not element matching) |
| `test_rewrite_js_single_script` | Single `<script>` src replaced |
| `test_rewrite_js_multiple_scripts` | First rewritten, subsequent removed |
| `test_rewrite_preserves_external` | External links/scripts are untouched |
| `test_rewrite_preserves_module` | `type="module"` scripts are untouched |
| `test_rewrite_preserves_async_defer` | `async`/`defer` scripts untouched |
| `test_rewrite_no_bundled_refs` | HTML with no bundled refs is unchanged |

### Unit tests in `mod.rs`

| Test | What it verifies |
|---|---|
| `test_bundle_assets_disabled` | Returns empty Vec when disabled |
| `test_bundle_assets_css_only` | Only CSS bundled when `js = false` |
| `test_bundle_assets_js_only` | Only JS bundled when `css = false` |
| `test_bundle_assets_no_refs` | No CSS/JS refs returns empty Vec |
| `test_bundle_assets_end_to_end` | Full pipeline: merge, tree-shake, rewrite |
| `test_bundle_assets_output_path_conflict` | Existing file at `css_output` path is overwritten with warning logged |

### Integration tests

| Test | What it verifies |
|---|---|
| `test_build_with_bundling` | Full build with bundling enabled: output HTML references bundle files, bundle files exist in dist/, tree-shaking removed unused selectors |
| `test_build_bundling_with_critical_css` | Both bundling and critical CSS enabled: inlined `<style>` is present, preload `<link>` points to bundle file, noscript fallback also points to bundle file |
| `test_build_bundling_with_content_hash` | Both bundling and content hashing enabled: bundle file is hashed, HTML references hashed bundle filename |

### What we do NOT test

- lightningcss parsing correctness (library's job).
- scraper selector matching correctness (library's job).
- lol_html rewriting correctness (library's job).
- lightningcss CSS minification correctness (library's job; we test
  that minification is enabled when `minify_css = true`, not that the
  output is correctly minified).
- Content hashing of bundled files (content_hash module tests cover this).
