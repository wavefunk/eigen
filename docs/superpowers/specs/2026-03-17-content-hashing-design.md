# Content Hashing for Static Assets -- Design Spec

Date: 2026-03-17

## Motivation and Goals

Repeat visitors to a static site currently re-download unchanged CSS, JS,
and image files on every visit because the server cannot safely set
long-lived cache headers -- the filename gives the browser no way to know
whether the content has changed. Content hashing solves this by embedding
a hash of the file's contents into the filename (e.g.
`style.css` -> `style.a1b2c3d4.css`). When the content changes, the hash
changes, producing a new URL. When it does not change, the URL is stable
and the browser serves from cache.

This enables `Cache-Control: public, max-age=31536000, immutable` on all
hashed assets -- the single most impactful caching optimization for
repeat visitors. CDNs and browsers can cache these files indefinitely
without any revalidation requests.

**Goals:**

- Fingerprint all static assets (CSS, JS, images, fonts, etc.) copied
  from `static/` to `dist/` with a content-based hash in the filename.
- Rewrite all references to those assets in rendered HTML, CSS, and JS
  files so that every URL points to the hashed filename.
- Make the `asset()` template function return hashed paths at render time
  so that well-written templates get correct URLs without post-processing.
- Be opt-in and default to disabled for backward compatibility.
- Integrate cleanly into the existing build pipeline with minimal
  disruption to other features (critical CSS, resource hints, image
  optimization).

**Non-goals (see "Out of Scope"):**

- Hashing dynamically generated assets (rendered HTML pages, fragments,
  sitemaps).
- Generating `_headers` or `.htaccess` files for CDN configuration.
- Content-addressable storage or deduplication across builds.

## Architecture

### Approach: Manifest at Copy Time + Template Function + Post-Render Rewrite

The feature operates in three coordinated phases:

**Phase 1: Build manifest during static asset copy.**

During `output::copy_static_assets()`, after copying each file from
`static/` to `dist/`, hash its contents (SHA-256, truncated to 8 bytes
= 16 hex characters), rename the file in `dist/` to include the hash,
and record the mapping `original_path -> hashed_path` in a manifest.
The manifest is returned from `copy_static_assets()` and threaded through
the build pipeline.

Example: `static/css/style.css` -> `dist/css/style.a1b2c3d4e5f67890.css`
Manifest entry: `/css/style.css` -> `/css/style.a1b2c3d4e5f67890.css`

**Phase 2: Template-time resolution via `asset()`.**

The `asset()` template function (already registered in
`src/template/functions.rs`) is enhanced to look up the manifest and
return the hashed path. Templates that use `{{ asset('/css/style.css') }}`
get the correct hashed URL at render time. This is the primary mechanism
for referencing hashed assets in templates.

**Phase 3: Post-render HTML/CSS/JS rewrite.**

After all rendering is complete, a final pass scans every rendered HTML,
CSS, and JS file in `dist/` and rewrites any remaining references to
original asset paths (ones that did not go through `asset()`). This
catches:

- Hardcoded `<link href="/css/style.css">` in templates.
- Hardcoded `<script src="/js/app.js">` in templates.
- CSS `url()` references in stylesheets (both in `dist/` CSS files and
  in inlined `<style>` blocks in HTML).
- `<img src>`, `<source srcset>`, and other asset references.

This phase is a safety net. Well-written templates using `asset()` will
have all references already correct, but the rewrite pass ensures
correctness even when authors hardcode paths.

### Why Three Phases?

- **Phase 1 alone** (just rename files) would break all references.
- **Phase 1 + Phase 3** (rename + rewrite) works but means templates
  render with wrong URLs and then get fixed up. This is wasteful for
  `asset()` users and prevents template-level logic from seeing the
  correct URL (e.g., computing a `<link rel="preload">`).
- **Phase 1 + Phase 2 + Phase 3** gives templates the correct URL at
  render time AND catches hardcoded references. The rewrite pass is
  cheap (simple string replacement on already-rendered files) and
  provides defense-in-depth.

### Hash Function and Format

**SHA-256**, truncated to the first 8 bytes (16 hex characters).

- SHA-256 is already a dependency (`sha2` crate in Cargo.toml).
- 8 bytes = 2^64 possible values. Collision probability for a site with
  10,000 assets is approximately 2.7 * 10^-12. This is well beyond
  acceptable for cache-busting purposes.
- 16 hex chars is short enough to keep filenames readable.
- This matches the truncation used by the existing image optimization
  code (`source_hash()` in `src/assets/images.rs`).

**Filename format:** `{stem}.{hash}.{ext}`

The hash is inserted between the stem and extension, separated by dots.
This follows the convention used by Webpack, Vite, Parcel, and most
modern bundlers. It keeps the extension last so that file type detection
by CDNs and web servers (which rely on the extension) continues to work.

Examples:
- `style.css` -> `style.a1b2c3d4e5f67890.css`
- `app.js` -> `app.9f8e7d6c5b4a3210.js`
- `logo.png` -> `logo.1234567890abcdef.png`
- `font.woff2` -> `font.fedcba0987654321.woff2`

Files without an extension get the hash appended: `CNAME` -> `CNAME` (not
hashed -- see exclusion rules below).

### Pipeline Position

The current build pipeline (from `build()` in `src/build/render.rs`) is:

```
1. Load config
2. Initialize plugins, global data, discovery
3. Setup output dir (clean dist/, copy static/)     <-- Phase 1 here
4. Setup template engine (register asset() fn)      <-- Phase 2 here
5. For each page (sequential loop):
   a. Resolve data queries
   b. Render template
   c. Strip fragment markers
   d. Localize assets
   e. Optimize images (img -> picture)
   f. Rewrite CSS background images
   g. Plugin post_render_html
   h. Critical CSS inlining
   i. Resource hints (preload/prefetch)
   j. Minify HTML
   k. Write to disk
   l. Extract and write fragments
6. Generate sitemap
7. Post-build plugin hooks
8. Phase 3: Rewrite asset references                <-- Phase 3 here
9. DONE
```

**Phase 1** runs inside step 3. Currently `copy_static_assets()` in
`src/build/output.rs` takes only `project_root: &Path` and returns
`Result<()>`. The signature changes to accept `ContentHashConfig` and
return `Result<AssetManifest>`. It copies files as before, then calls
`content_hash::build_manifest()` if hashing is enabled.

**Phase 2** runs inside step 4. Currently `setup_environment()` in
`src/template/environment.rs` takes `(project_root, config, pages,
plugin_registry)`. A new `manifest` parameter is added and passed
through to `register_functions()`.

**Phase 3** runs as a new step 8, after post-build plugin hooks and
before the final log output.

**Why Phase 3 runs after post-build hooks:** Plugins may generate or
modify files in dist during `post_build`. These files might contain
references to static assets that need rewriting. Running Phase 3 last
ensures all references are caught.

**Why Phase 3 does NOT run before minification:** Minification happens
per-page during rendering (step 5j). Phase 3 operates on the final
files on disk after all pages are rendered. This means Phase 3 runs
on already-minified HTML, which is fine -- string replacement works
the same on minified HTML.

### Interaction with Image Optimization

Image optimization (step 5e) already generates content-hashed filenames
for image variants (`hero-480w-a1b2c3d4.webp`). These generated variants
are NOT in the `static/` directory and are NOT covered by Phase 1. They
are generated directly into `dist/` during rendering.

The content hashing feature does not interfere with image optimization:

- Original images in `static/` (e.g., `static/images/hero.jpg`) get
  hashed by Phase 1 to `dist/images/hero.a1b2c3d4e5f67890.jpg`.
- When image optimization processes the rendered HTML, it reads
  `<img src="/images/hero.a1b2c3d4e5f67890.jpg">` and looks for the
  file at `dist/images/hero.a1b2c3d4e5f67890.jpg`. This works because
  Phase 1 already renamed the file.
- Image optimization generates its own variants with their own hashes
  in the filename (e.g., `hero-480w-e5f6a7b8.webp`). These variants
  are written directly to dist and do not need further renaming.

**Important:** The `asset()` function in templates should be used for
the source path (`{{ asset('/images/hero.jpg') }}`), which returns the
Phase 1 hashed path. Image optimization then takes it from there.

### Interaction with Critical CSS

Critical CSS inlining (step 5h) reads stylesheet files from `dist/`
via the `StylesheetCache::get_or_load` method in
`src/build/critical_css/mod.rs`. When content hashing is enabled, the
stylesheet files in dist have hashed filenames.

For templates using `asset()`: the `<link>` href already points to
the hashed filename (e.g., `/css/style.a1b2c3d4e5f67890.css`), and
the file exists at that path after Phase 1. Critical CSS works without
modification.

For hardcoded references that Phase 3 has not yet rewritten (Phase 3
runs after all rendering), the `<link>` will point to
`/css/style.css` but the file on disk is `style.a1b2c3d4e5f67890.css`.
This would cause critical CSS to fail to find the file.

**Resolution:** The `StylesheetCache::get_or_load` method is enhanced
to accept an optional `AssetManifest` reference. When the file at the
original href is not found, it tries resolving through the manifest.
This is a small change to one function. The `load_stylesheet` helper
also receives the manifest for the same fallback behavior.

### Interaction with Resource Hints

Resource hints (step 5i) scan rendered HTML for `<img>` tags and
navigation links. Since templates using `asset()` already have hashed
paths, and the hints module reads paths from the rendered HTML, no
special handling is needed. The preload `<link>` will naturally contain
the hashed URL.

For hardcoded paths, the preload `<link>` will reference the un-hashed
path, which Phase 3 will rewrite in the final output. The preload
still works because the browser uses the URL to match the eventual
request, and Phase 3 rewrites both the preload hint and the actual
asset reference consistently.

### Interaction with Asset Localization

Asset localization (step 5d) downloads remote assets and rewrites URLs
to local paths like `/assets/photo-abc123.jpg`. These downloaded assets
are placed directly in `dist/assets/` and are NOT in `static/`. They
are therefore NOT covered by Phase 1 content hashing.

This is correct behavior: localized assets already have content-based
filenames (the download module uses a hash of the URL). They do not need
additional content hashing.

### Dev Server

Content hashing is **disabled during dev server** regardless of config.
The dev server (`src/dev/rebuild.rs`) uses its own render functions
(`render_static_page_dev`, `render_dynamic_page_dev`) that skip image
optimization, critical CSS, resource hints, and minification. These
functions do not call the production build pipeline's post-processing
steps.

The dev server calls `copy_static_assets` in two places:
- `full_build()` (line 165)
- `rebuild()` with `RebuildScope::StaticOnly` (line 128)

Both must pass a disabled `ContentHashConfig` to avoid renaming files
without updating references.

Implementation: The `asset()` function receives an
`Option<Arc<AssetManifest>>` rather than a bare `AssetManifest`. When
`None` (dev server, or hashing disabled), it returns the path unchanged.
When `Some`, it looks up the hashed path.

The dev server's `setup_environment` call on line 168 does not pass a
manifest (passes `None`), so `asset()` falls through to pass-through
behavior. This means content hashing is fully disabled in dev mode with
no special-casing needed beyond the `Option` type.

## Data Models and Types

### Configuration

Add to `src/config/mod.rs`:

```rust
/// Configuration for content hashing of static assets.
///
/// Located under `[build.content_hash]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentHashConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Glob patterns for files in `static/` to exclude from hashing.
    /// Matched against the path relative to `static/`.
    /// Default: common files that must keep stable names.
    #[serde(default = "default_hash_exclude")]
    pub exclude: Vec<String>,
}

fn default_hash_exclude() -> Vec<String> {
    vec![
        "favicon.ico".into(),
        "robots.txt".into(),
        "CNAME".into(),
        "_headers".into(),
        "_redirects".into(),
        ".well-known/**".into(),
    ]
}

impl Default for ContentHashConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            exclude: default_hash_exclude(),
        }
    }
}
```

Note: `sitemap.xml` is NOT in the default exclude list because the
build generates `sitemap.xml` via `sitemap::generate_sitemap()` (step
6 in the pipeline). It is never copied from `static/`. If a user places
a `sitemap.xml` in `static/`, they likely want it to be hashed. Users
who need a stable-named `sitemap.xml` in `static/` can add it to their
custom exclude list.

Add the field to `BuildConfig` (existing fields shown for context):

```rust
pub struct BuildConfig {
    pub fragments: bool,
    pub fragment_dir: String,
    pub content_block: String,
    pub oob_blocks: Vec<String>,
    pub minify: bool,
    pub critical_css: CriticalCssConfig,
    pub hints: HintsConfig,

    /// Content hashing for static assets.
    #[serde(default)]
    pub content_hash: ContentHashConfig,
}
```

Update the `Default` impl for `BuildConfig`:

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
            hints: HintsConfig::default(),
            content_hash: ContentHashConfig::default(),
        }
    }
}
```

### Asset Manifest

The manifest is the central data structure. It is built once during Phase
1, shared (immutably, via `Arc`) with the template engine for Phase 2,
and consumed by Phase 3 for the post-render rewrite.

```rust
/// Mapping from original asset URL paths to content-hashed URL paths.
///
/// Built during `copy_static_assets` when content hashing is enabled.
/// Shared across the build pipeline via `Arc`.
///
/// All paths are URL paths relative to site root with a leading slash,
/// e.g. `/css/style.css` -> `/css/style.a1b2c3d4e5f67890.css`.
pub struct AssetManifest {
    /// Forward map: original path -> hashed path.
    entries: HashMap<String, String>,
}

impl AssetManifest {
    /// Create a new empty manifest.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Create with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(cap),
        }
    }

    /// Insert a mapping.
    pub fn insert(&mut self, original: String, hashed: String) {
        self.entries.insert(original, hashed);
    }

    /// Look up the hashed path for an original path.
    /// Returns the original path unchanged if not found in the manifest.
    pub fn resolve(&self, original: &str) -> &str {
        self.entries.get(original).map(|s| s.as_str()).unwrap_or(original)
    }

    /// Return all (original, hashed) pairs sorted by original path
    /// length descending.
    ///
    /// Longest-match-first ordering prevents partial replacements during
    /// string rewriting. For example, `/css/style.css` must be replaced
    /// before `/css/style.css.map` would be (though in practice the
    /// paths have different extensions so collisions are unlikely, the
    /// ordering provides a safety guarantee).
    pub fn pairs_longest_first(&self) -> Vec<(&str, &str)> {
        let mut pairs: Vec<(&str, &str)> = self.entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        pairs
    }

    /// Whether the manifest is empty (no assets were hashed).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}
```

### Content Hash Computation

```rust
use std::fmt::Write;

/// Compute the content hash for a file's bytes.
///
/// Returns a 16-character lowercase hex string (SHA-256 truncated to 8
/// bytes). This matches the hash length used by image optimization.
fn content_hash(data: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for b in &result[..8] {
        write!(hex, "{:02x}", b).expect("write to String never fails");
    }
    hex
}
```

Note: This uses `write!` into a pre-allocated `String` instead of
`.map(|b| format!("{:02x}", b)).collect()` to avoid per-byte String
allocations, consistent with the project's style of avoiding unnecessary
allocations (see CLAUDE.md).

Note: This is intentionally a separate function from `images::source_hash`
even though the logic is identical. The image hash function is private
to `images.rs` and its contract is specific to image cache keys. Sharing
it would create a coupling between unrelated modules. If both need to
change (e.g., to a different hash length), they should be able to change
independently.

### Hashed Filename Construction

```rust
/// Construct a hashed filename from the original filename and hash.
///
/// Inserts the hash between the stem and extension:
///   `style.css` + `a1b2c3d4` -> `style.a1b2c3d4.css`
///   `app.min.js` + `a1b2c3d4` -> `app.min.a1b2c3d4.js`
///
/// Files without an extension get the hash appended:
///   `LICENSE` + `a1b2c3d4` -> `LICENSE.a1b2c3d4`
fn hashed_filename(original_filename: &str, hash: &str) -> String {
    match original_filename.rfind('.') {
        Some(dot_pos) => {
            let stem = &original_filename[..dot_pos];
            let ext = &original_filename[dot_pos + 1..];
            format!("{stem}.{hash}.{ext}")
        }
        None => {
            format!("{original_filename}.{hash}")
        }
    }
}
```

## API Surface

### Public Functions

#### `build::content_hash` module

The module `build::content_hash` is a new single-file module at
`src/build/content_hash.rs`. It exposes two public functions and the
`AssetManifest` struct:

```rust
/// Build the asset manifest by hashing and renaming static assets in
/// dist_dir.
///
/// Walks all files in dist_dir that correspond to files from static_dir,
/// computes content hashes, renames files to include the hash, and
/// returns the manifest.
///
/// Files matching the exclude patterns are left at their original names.
///
/// This function is called from `copy_static_assets` when content
/// hashing is enabled.
pub fn build_manifest(
    dist_dir: &Path,
    static_dir: &Path,
    config: &ContentHashConfig,
) -> Result<AssetManifest>
```

```rust
/// Rewrite all references to static assets in rendered HTML, CSS, and
/// JS files in dist_dir.
///
/// Walks all `.html`, `.css`, and `.js` files in dist_dir and replaces
/// occurrences of original asset paths with their hashed equivalents
/// from the manifest.
///
/// This is Phase 3: the post-render safety-net rewrite. It catches
/// references that were not resolved by the `asset()` template
/// function.
///
/// Fragment HTML files in `_fragments/` ARE rewritten (they reference
/// CSS/JS/images and must use the same hashed paths as the parent
/// page).
pub fn rewrite_references(
    dist_dir: &Path,
    manifest: &AssetManifest,
) -> Result<()>
```

#### Changes to `template::functions`

Current signature (from `src/template/functions.rs`):

```rust
pub fn register_functions(env: &mut Environment<'_>, config: &SiteConfig)
```

New signature:

```rust
pub fn register_functions(
    env: &mut Environment<'_>,
    config: &SiteConfig,
    manifest: Option<Arc<AssetManifest>>,
)
```

The `asset()` function changes from:

```rust
// Current implementation (line 54-60 of src/template/functions.rs):
env.add_function("asset", |path: &str| -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
});
```

To:

```rust
let manifest_clone = manifest.clone();
env.add_function("asset", move |path: &str| -> String {
    let normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    match &manifest_clone {
        Some(m) => m.resolve(&normalized).to_string(),
        None => normalized,
    }
});
```

#### Changes to `build::output`

Current signature (from `src/build/output.rs`):

```rust
pub fn copy_static_assets(project_root: &Path) -> Result<()>
```

New signature:

```rust
pub fn copy_static_assets(
    project_root: &Path,
    content_hash_config: &ContentHashConfig,
) -> Result<AssetManifest>
```

The function copies files as before, then calls
`content_hash::build_manifest()` if hashing is enabled. When disabled,
returns an empty `AssetManifest`.

**Dev server call sites** (in `src/dev/rebuild.rs`):

The dev server calls `copy_static_assets` in two places:
- `full_build()` at line 165
- `rebuild()` at line 128 (for `RebuildScope::StaticOnly`)

Both must be updated. The cleanest approach: the dev server passes
`&ContentHashConfig::default()` (which has `enabled: false`) to ensure
hashing is never performed during development. The returned manifest is
discarded (the `_` pattern).

```rust
// In full_build():
let _ = crate::build::output::copy_static_assets(
    project_root,
    &crate::config::ContentHashConfig::default(),
)?;

// In rebuild() for StaticOnly:
let _ = crate::build::output::copy_static_assets(
    &self.project_root,
    &crate::config::ContentHashConfig::default(),
)?;
```

#### Changes to `build::critical_css`

The `StylesheetCache::get_or_load` method gains an optional manifest
parameter for path resolution:

Current signature (from `src/build/critical_css/mod.rs`):

```rust
fn get_or_load(&mut self, href: &str, dist_dir: &Path) -> Option<&str>
```

New signature:

```rust
fn get_or_load(
    &mut self,
    href: &str,
    dist_dir: &Path,
    manifest: Option<&AssetManifest>,
) -> Option<&str>
```

When the file at the original href is not found AND a manifest is
provided, it tries resolving the href through the manifest (in case
the HTML contains the original path but the file on disk has been
renamed). This handles the edge case where a template hardcodes a
stylesheet path without using `asset()`.

The `load_stylesheet` helper function (also in `critical_css/mod.rs`)
receives the same manifest parameter for the same fallback.

All existing call sites inside `inline_critical_css` must be updated
to pass the manifest. This means `inline_critical_css` itself needs
an additional parameter:

```rust
pub fn inline_critical_css(
    html: &str,
    config: &CriticalCssConfig,
    dist_dir: &Path,
    css_cache: &mut StylesheetCache,
    manifest: Option<&AssetManifest>,
) -> String
```

And the call sites in `render_static_page` and `render_dynamic_page`
in `src/build/render.rs` (lines 321-328 and 585-593) must be updated
to pass the manifest reference.

#### Changes to `template::environment`

Current signature (from `src/template/environment.rs`):

```rust
pub fn setup_environment(
    project_root: &Path,
    config: &SiteConfig,
    pages: &[PageDef],
    plugin_registry: Option<&PluginRegistry>,
) -> Result<Environment<'static>>
```

New signature:

```rust
pub fn setup_environment(
    project_root: &Path,
    config: &SiteConfig,
    pages: &[PageDef],
    plugin_registry: Option<&PluginRegistry>,
    manifest: Option<Arc<AssetManifest>>,
) -> Result<Environment<'static>>
```

The manifest is passed through to `register_functions` (line 89).

### Internal Functions

```rust
/// Check whether a file should be excluded from content hashing.
///
/// Uses glob::Pattern matching against the relative path from static/.
/// Follows the same pattern as `images::is_excluded` in
/// `src/assets/images.rs`.
fn is_excluded(relative_path: &str, exclude_patterns: &[String]) -> bool

/// Compute content hash for file bytes (SHA-256, first 8 bytes as hex).
fn content_hash(data: &[u8]) -> String

/// Construct hashed filename: `style.css` -> `style.{hash}.css`.
fn hashed_filename(original_filename: &str, hash: &str) -> String

/// Rewrite asset references in a single file's content.
///
/// Performs string replacement of all original paths with their hashed
/// equivalents, using longest-match-first ordering to prevent partial
/// matches.
///
/// Returns the rewritten content, or None if no replacements were made
/// (to avoid unnecessary file writes).
fn rewrite_file_content(
    content: &str,
    manifest: &AssetManifest,
) -> Option<String>
```

## Error Handling

| Scenario | Behavior |
|---|---|
| File read error during hashing | Log warning, skip file (leave at original name, no manifest entry) |
| File rename error after hashing | Return `Err`, fail the build (filesystem is in inconsistent state) |
| Hash collision (two files produce same hash) | Astronomically unlikely (2^-64). If it happens, the second file overwrites the first. No special handling. |
| `asset()` called with path not in manifest | Return the path unchanged (no error). This handles paths to dynamic assets, external URLs, etc. |
| Rewrite pass fails to read an HTML/CSS/JS file | Log warning, skip that file. Other files are still rewritten. |
| Rewrite pass fails to write an HTML/CSS/JS file | Return `Err`, fail the build (output is corrupted) |
| Empty `static/` directory | Empty manifest, all phases are no-ops |
| `static/` directory does not exist | Empty manifest (existing `copy_static_assets` returns early). All phases are no-ops. |
| Content hashing disabled | All phases are skipped. `asset()` returns unchanged paths. Empty manifest returned from `copy_static_assets`. |

**Principle:** Phase 1 (manifest building / file renaming) errors are
fatal because a partial rename leaves dist in an inconsistent state.
Phase 3 (rewrite) read errors are non-fatal (skip the file), but write
errors are fatal (the output file is corrupted). The `asset()` function
never errors -- unknown paths pass through unchanged.

## Edge Cases and Failure Modes

### Files without extensions

Files like `CNAME`, `LICENSE`, `.htaccess` are handled:
- `CNAME` and similar deployment files are in the default exclude list
  and are not hashed.
- If a file without an extension is NOT excluded, it gets the hash
  appended: `LICENSE` -> `LICENSE.a1b2c3d4e5f67890`.

### Dotfiles

Files starting with `.` (e.g., `.htaccess`, `.well-known/`) are handled:
- `.well-known/**` is in the default exclude list.
- Other dotfiles are hashed normally. `.htaccess` -> `.htaccess.a1b2c3d4e5f67890`
  which would break Apache. Users who have `.htaccess` in their static
  dir should add it to the exclude list.

**Note:** The `hashed_filename` function uses `rfind('.')` to locate the
extension. For a dotfile like `.htaccess`, the dot is at position 0 and
`rfind` returns `Some(0)`. This means `stem` is empty and the result is
`.a1b2c3d4e5f67890.htaccess` (hash inserted between empty stem and the
rest as "extension"). This is technically correct for cache-busting
purposes but looks unusual. Since `.htaccess` should be in the exclude
list anyway, and dotfiles in `static/` are rare, this is acceptable.

### Files with multiple dots

`app.min.js` -> `app.min.a1b2c3d4e5f67890.js`. The hash is inserted
before the LAST extension (using `rfind('.')`). This preserves the `.js`
extension for MIME type detection while keeping the `.min` qualifier.

### CSS `url()` references

CSS files may reference other assets via `url()`:

```css
.icon { background-image: url('/images/icon.png'); }
@font-face { src: url('/fonts/inter.woff2'); }
```

Phase 3's rewrite pass handles these by performing string replacement
on the CSS file content. Since the replacement is `"/images/icon.png"`
-> `"/images/icon.a1b2c3d4e5f67890.png"`, and the original path is unique
(it includes the full directory path), the replacement is safe inside
`url()` contexts.

### CSS `url()` with relative paths

CSS files may use relative paths in `url()`:

```css
/* In /css/style.css */
.icon { background-image: url('../images/icon.png'); }
```

Phase 3's string replacement operates on path strings as they appear
in the file. Relative paths like `../images/icon.png` do NOT match the
manifest key `/images/icon.png` and will NOT be rewritten.

**Mitigation:** The `asset()` template function always returns absolute
paths. For CSS files in `static/`, authors should use absolute paths
(`/images/icon.png`) rather than relative paths. For CSS files that use
relative paths, the referenced assets will still be served correctly
(the original files are renamed in Phase 1, but the relative path
resolution in the browser computes the correct directory), except that
the browser will request the un-hashed filename which no longer exists.

**Recommendation:** Document that CSS files in `static/` should use
absolute paths for asset references when content hashing is enabled.
This is a known limitation shared with most static cache-busting tools.

### CSS files referencing other CSS files via `@import`

`@import` URLs are rewritten by Phase 3 just like `url()`. The imported
file itself has been renamed by Phase 1. The same limitation applies
to relative `@import` paths as described above.

### Sourcemaps

Source maps (`.map` files) are static assets and are hashed like any
other file. References to source maps in CSS/JS (`//# sourceMappingURL=`)
are rewritten by Phase 3. If a CSS file `style.css` has a sourcemap
reference `style.css.map`, both files are hashed independently. Phase 3
rewrites the reference inside the CSS file.

**Note:** `sourceMappingURL` references are often relative (e.g.,
`//# sourceMappingURL=style.css.map`). These will NOT be rewritten by
Phase 3 because the manifest uses absolute paths. Source maps are
primarily a development tool and are not typically needed in production.
Users who need production source maps should either:
- Use absolute paths in their source map references.
- Add `*.map` to the exclude list and handle source maps separately.

### JavaScript `import` and dynamic references

Phase 3 rewrites `.js` files as well as `.html` and `.css`. This catches
static string paths like those in `import '/js/utils.js'`. However,
dynamic references constructed at runtime (e.g.,
`fetch('/api/' + name + '.json')`) cannot be rewritten. This is an
inherent limitation shared by all static-analysis-based cache busting
tools.

**Decision:** We scan `.js` files in Phase 3 in addition to `.html` and
`.css`. The string replacement approach is safe for JS files because
the original asset paths (e.g., `/css/style.css`) are specific enough
to not collide with JS identifiers or string content. The paths always
start with `/` and contain file extensions, making false positives
extremely unlikely.

### String replacement safety

Phase 3 uses simple string replacement (`str::replace`), not
regex-based or AST-aware rewriting. This is safe because:

1. **Longest-match-first ordering** prevents partial replacements.
   If `/css/style.css` and `/css/style.css.map` are both in the
   manifest, the longer path is replaced first.

2. **Manifest paths are absolute and specific.** They always start
   with `/` and include directory components and file extensions.
   The probability of a manifest path appearing as a substring in
   an unrelated context (e.g., inside prose text or a JS variable
   name) is negligible.

3. **Already-hashed paths are not re-matched.** The hashed filename
   (`style.a1b2c3d4e5f67890.css`) is longer than the original
   (`style.css`), so it cannot be a substring match for any other
   manifest entry.

4. **Idempotency.** Running the rewrite twice produces the same
   output: after the first pass, no original paths remain to match.

### Image optimization interaction details

When content hashing is enabled:

1. Phase 1 renames `dist/images/hero.jpg` to
   `dist/images/hero.a1b2c3d4e5f67890.jpg`.
2. Templates using `{{ asset('/images/hero.jpg') }}` render with
   `/images/hero.a1b2c3d4e5f67890.jpg`.
3. Image optimization reads `<img src="/images/hero.a1b2c3d4e5f67890.jpg">`,
   resolves to `dist/images/hero.a1b2c3d4e5f67890.jpg`, and generates
   variants like `hero-480w-e5f6a7b8.webp`.
4. The `<picture>` rewrite uses the new variant paths directly.

For templates NOT using `asset()`:

1. Phase 1 renames the file in dist.
2. Template renders `<img src="/images/hero.jpg">`.
3. Image optimization looks for `dist/images/hero.jpg` -- NOT FOUND.
4. Image optimization skips the image (logs a debug message).
5. Phase 3 rewrites the `<img src>` to use the hashed path.
6. But image optimization has already run and will not re-run.

**This means hardcoded image paths will get hashed filenames but NOT
responsive image variants when content hashing is enabled.** This is
acceptable because:
- The `asset()` function is the recommended way to reference assets.
- The behavior is correct (the image loads, just without optimization).
- This edge case only affects templates that hardcode paths instead of
  using `asset()`.
- The build logs will show the image as "not found" during optimization,
  providing a clear signal to the template author.

### Fragment files

Fragment HTML files in `dist/_fragments/` reference CSS, JS, and images.
Phase 3 rewrites these references too. Fragments are loaded via HTMX
into pages that already have stylesheets loaded, so the rewritten
asset paths in fragments must match the hashed paths used by the
parent page.

### Critical CSS with hardcoded stylesheet paths

When a template hardcodes `<link href="/css/style.css">` without using
`asset()`, critical CSS runs before Phase 3 rewrites the path. The
`StylesheetCache::get_or_load` enhancement (described in the API
section) resolves this by trying the manifest lookup when the file
is not found at the original path.

### `srcset` attribute rewriting

`<source>` elements with `srcset` attributes contain comma-separated
URL+descriptor pairs. Phase 3's string replacement works here because
the replacement is path-based: `/images/photo.jpg` is replaced with
`/images/photo.a1b2c3d4e5f67890.jpg` regardless of where it appears
in the attribute value.

### Paths with query strings or fragments

Asset paths like `/css/style.css?v=1` or `/js/app.js#module` are NOT
matched by Phase 3's string replacement (the manifest key is the clean
path without query/fragment). This is intentional: query-string
versioning is an alternative to content hashing and should not be
combined with it.

### Non-UTF-8 files

Binary files (images, fonts, compiled WASM) are hashed and renamed by
Phase 1 but NOT scanned for references during Phase 3. Phase 3 only
processes text files (`.html`, `.css`, `.js`). Binary files do not
contain rewritable text references.

Phase 3 uses `std::fs::read_to_string` to read files. If a `.js` or
`.css` file contains non-UTF-8 bytes (extremely rare), the read will
fail and the file is skipped with a warning.

### Very large static directories

For sites with thousands of static files, the manifest HashMap lookup
is O(1) per reference. The Phase 3 rewrite scans each output file once
and performs string replacements. The manifest is sorted longest-first
for replacement safety (preventing partial matches). Total memory for
the manifest is approximately 200 bytes per asset (two string
allocations). For 10,000 assets, this is ~2 MB -- negligible.

### Concurrent/parallel builds

The current build processes pages sequentially in a single thread
(the `for page in &pages` loop in `build()`). The manifest is built
before rendering starts and is immutable during rendering (shared via
`Arc`). Phase 3 is a post-build pass that runs after all pages are
written. No concurrency issues.

If the build is parallelized in the future, the manifest is already
`Arc`-wrapped and read-only during rendering. Phase 3 could be
parallelized per-file with no shared mutable state.

### Interaction with inlined critical CSS

When critical CSS inlines CSS rules into `<style>` blocks, those
inlined rules may contain `url()` references to static assets. Phase 3
scans HTML files and performs string replacement on the entire file
content, which includes any `<style>` blocks. Therefore, `url()`
references inside inlined critical CSS are correctly rewritten.

## Configuration Examples

### Minimal (opt-in)

```toml
[build.content_hash]
enabled = true
```

### Custom exclusions

```toml
[build.content_hash]
enabled = true
exclude = [
    "favicon.ico",
    "robots.txt",
    "CNAME",
    "_headers",
    "_redirects",
    ".well-known/**",
    ".htaccess",
    "sw.js",           # service workers must have stable URLs
    "manifest.json",   # PWA manifest must have stable URL
]
```

### Template usage

```html
<!-- Recommended: use asset() for all static asset references -->
<link rel="stylesheet" href="{{ asset('/css/style.css') }}">
<script src="{{ asset('/js/app.js') }}"></script>
<img src="{{ asset('/images/logo.png') }}" alt="Logo">

<!-- Also works: hardcoded paths are rewritten by Phase 3 -->
<link rel="stylesheet" href="/css/style.css">
```

## What Is NOT In Scope

1. **Hashing rendered HTML pages.** HTML pages are not cacheable with
   immutable headers because their content changes on every build.
   Only static assets (CSS, JS, images, fonts) from `static/` are hashed.

2. **Hashing dynamically generated assets.** Image optimization variants,
   localized assets, sitemap.xml (generated), and fragment HTML files
   are not hashed by this feature. Image variants already have
   content-based filenames. Generated files change on every build.

3. **`_headers` or `.htaccess` generation.** This feature renames files
   and rewrites references. Generating the appropriate cache headers
   for the deployment platform is a separate concern (proposal #25 in
   `docs/proposals.md`: Security Headers File Generation).

4. **Content-addressable storage.** Files are not deduplicated based on
   content. If two different source files produce identical content,
   they get the same hash but remain as separate files at different
   paths.

5. **Manifest file output.** We do not write a `manifest.json` or
   `asset-manifest.json` to dist. The manifest is an in-memory data
   structure used during the build. If users need a manifest file (e.g.,
   for server-side rendering), it can be added later as a small
   extension.

6. **Cache header injection.** The feature enables immutable caching but
   does not configure it. Users must set up `Cache-Control` headers in
   their CDN/server configuration. This is documented but not automated.

7. **Hashing assets referenced only in JavaScript runtime.** Dynamic
   `fetch()` calls with constructed URLs cannot be statically rewritten.
   This is a fundamental limitation of static analysis.

8. **Development server support.** Content hashing is a production
   optimization. The dev server does not hash assets. The `asset()`
   function returns unchanged paths in dev mode.

9. **Rollback or cleanup of old hashed files.** Since `dist/` is cleaned
   on every build (`setup_output_dir` removes it entirely), there are
   no stale hashed files to clean up.

10. **Relative path rewriting.** CSS `url()` and `@import` references
    that use relative paths (e.g., `../images/icon.png`) are not
    rewritten. Only absolute paths matching manifest keys are rewritten.
    See the "CSS `url()` with relative paths" edge case section.

## Module Structure

```
src/build/
  content_hash.rs     -- AssetManifest, build_manifest(), rewrite_references(),
                         content_hash(), hashed_filename(), is_excluded(),
                         rewrite_file_content()
```

A single file (not a directory with `mod.rs`) because the feature has
one clear concern (hash assets, rewrite references) and the total code
is estimated at ~200 lines. The `AssetManifest` struct, the manifest
builder, and the reference rewriter are tightly coupled -- the rewriter's
only input is the manifest. There is no benefit to splitting them.

This follows the pattern of `build/minify.rs` (single-file module for a
single-concern build step) rather than `build/critical_css/` (multi-file
directory module for a multi-concern feature with extract/rewrite split).

## Integration Points

### build/render.rs

In `build()`, the current code (lines 67-77) does:

```rust
output::setup_output_dir(project_root, ...)?;
output::copy_static_assets(project_root)?;
tracing::info!("Copying static assets... ✓");
let env = template::setup_environment(project_root, &config, &pages, Some(&plugin_registry))?;
```

This changes to:

```rust
output::setup_output_dir(project_root, ...)?;

// Phase 1: Copy static assets (with content hashing if enabled).
let manifest = output::copy_static_assets(
    project_root,
    &config.build.content_hash,
)?;
let manifest = Arc::new(manifest);

if !manifest.is_empty() {
    tracing::info!(
        "Content hashing: {} assets fingerprinted.",
        manifest.len(),
    );
}
tracing::info!("Copying static assets... ✓");

// Phase 2: Setup template engine (pass manifest for asset() function).
let env = template::setup_environment(
    project_root,
    &config,
    &pages,
    Some(&plugin_registry),
    if config.build.content_hash.enabled {
        Some(manifest.clone())
    } else {
        None
    },
)?;
```

The critical CSS calls in `render_static_page` (lines 321-328) and
`render_dynamic_page` (lines 585-593) also need the manifest:

```rust
let full_html = if config.build.critical_css.enabled {
    critical_css::inline_critical_css(
        &full_html,
        &config.build.critical_css,
        dist_dir,
        css_cache,
        if manifest.is_empty() { None } else { Some(&manifest) },
    )
} else {
    full_html
};
```

After the post-build hooks (line 172), add Phase 3:

```rust
// Run post-build hooks
plugin_registry.post_build(&dist_dir, project_root)?;

// Phase 3: Rewrite remaining asset references.
if config.build.content_hash.enabled && !manifest.is_empty() {
    content_hash::rewrite_references(&dist_dir, &manifest)?;
    tracing::info!("Asset references rewritten.");
}
```

### build/mod.rs

Add module declaration (current file at `src/build/mod.rs`):

```rust
pub mod content_hash;   // <-- NEW
pub mod context;
pub mod critical_css;
pub mod fragments;
pub mod hints;
pub mod minify;
pub mod output;
pub mod render;
pub mod sitemap;
```

### config/mod.rs

Add `ContentHashConfig` struct and `content_hash` field to `BuildConfig`.
Follow the existing pattern used by `CriticalCssConfig` and `HintsConfig`.

### template/functions.rs

Update `register_functions` signature to accept
`Option<Arc<AssetManifest>>`. Update `asset()` closure to use manifest
lookup. The existing test helper `test_config()` and tests must be
updated to pass `None` for the manifest.

### template/environment.rs

Update `setup_environment` signature to accept and pass through the
manifest. The `register_functions` call at line 89 becomes:

```rust
functions::register_functions(&mut env, config, manifest);
```

The dev server's call to `setup_environment` (in `rebuild.rs` line 168)
passes `None` for the manifest.

### build/output.rs

Update `copy_static_assets` to accept `&ContentHashConfig`, call
`content_hash::build_manifest()` when enabled, and return
`Result<AssetManifest>`.

### build/critical_css/mod.rs

Update `get_or_load`, `load_stylesheet`, and `inline_critical_css` to
accept optional manifest for path resolution fallback.

### dev/rebuild.rs

Update calls to `copy_static_assets` (lines 128, 165) to pass
`&ContentHashConfig::default()`.

Update calls to `setup_environment` (line 168) to pass `None` for the
manifest parameter.

### Cargo.toml

No new dependencies. `sha2`, `walkdir`, and `glob` are already present.

## Test Plan

Tests follow the project convention: write tests immediately after the
feature, avoid "ceremony" tests that just test the library, do not use
`unwrap` or `expect` unless it is an invariant.

### Unit tests in `content_hash.rs`

| Test | What it verifies |
|---|---|
| `test_content_hash_deterministic` | Same bytes produce same hash |
| `test_content_hash_different_data` | Different bytes produce different hashes |
| `test_content_hash_length` | Hash is exactly 16 hex characters |
| `test_hashed_filename_with_ext` | `style.css` + hash -> `style.{hash}.css` |
| `test_hashed_filename_multi_dot` | `app.min.js` + hash -> `app.min.{hash}.js` |
| `test_hashed_filename_no_ext` | `LICENSE` + hash -> `LICENSE.{hash}` |
| `test_hashed_filename_dotfile` | `.htaccess` + hash -> `.{hash}.htaccess` |
| `test_is_excluded_default` | `favicon.ico`, `robots.txt`, `CNAME` are excluded |
| `test_is_excluded_glob` | `**.well-known/**` pattern excludes deep paths |
| `test_is_excluded_no_match` | Non-excluded files pass through |
| `test_manifest_resolve_found` | Known path returns hashed path |
| `test_manifest_resolve_not_found` | Unknown path returns original path |
| `test_manifest_pairs_longest_first` | Pairs are sorted by path length descending |
| `test_build_manifest_basic` | Creates manifest from static dir, renames files |
| `test_build_manifest_excludes` | Excluded files are not renamed, no manifest entry |
| `test_build_manifest_empty_dir` | Empty static dir produces empty manifest |
| `test_build_manifest_nested_dirs` | Deep directory structure is handled |
| `test_rewrite_references_html` | HTML file `<link>`, `<script>`, `<img>` references are rewritten |
| `test_rewrite_references_css` | CSS `url()` references are rewritten |
| `test_rewrite_references_js` | JS string paths are rewritten |
| `test_rewrite_references_no_match` | Files without asset references are unchanged (no write) |
| `test_rewrite_references_multiple` | Multiple references in one file are all rewritten |
| `test_rewrite_references_fragments` | Fragment files in `_fragments/` are also rewritten |
| `test_rewrite_file_content_returns_none` | No changes means None returned (avoids write) |
| `test_rewrite_idempotent` | Running rewrite twice produces same output |

### Unit tests in `template/functions.rs`

| Test | What it verifies |
|---|---|
| `test_asset_with_manifest` | `asset('/css/style.css')` returns hashed path |
| `test_asset_without_manifest` | `asset('/css/style.css')` returns original path (None manifest) |
| `test_asset_unknown_path_with_manifest` | Unknown path passes through unchanged |
| `test_asset_normalizes_path` | `asset('css/style.css')` (no leading /) still resolves |

### Unit tests in `critical_css/mod.rs`

| Test | What it verifies |
|---|---|
| `test_get_or_load_with_manifest_fallback` | When file not found at original path, manifest resolves to hashed path |
| `test_get_or_load_without_manifest` | Existing behavior unchanged when manifest is None |

### Integration tests

| Test | What it verifies |
|---|---|
| `test_build_with_content_hash` | Full build with hashing enabled: files renamed, HTML references rewritten, `asset()` returns hashed paths |
| `test_build_content_hash_disabled` | Default config does not hash assets |
| `test_build_content_hash_with_critical_css` | Critical CSS finds hashed stylesheet files (both via asset() and hardcoded) |
| `test_build_content_hash_with_image_optimization` | Image optimization works with hashed source images via `asset()` |

### What we do NOT test

- SHA-256 correctness (that is the `sha2` library's job).
- `walkdir` traversal correctness (that is the library's job).
- Glob pattern matching correctness (that is the `glob` library's job).
- Filesystem operations (copy, rename) correctness (OS-level).
