# Open Graph / Twitter Card Meta Tags -- Design Spec

Date: 2026-03-17

## Motivation and Goals

When a page is shared on social media (Twitter/X, Facebook, LinkedIn,
Slack, Discord, iMessage, etc.), the platform's crawler reads Open Graph
and Twitter Card meta tags from the page's `<head>` to generate a rich
preview card. Without these tags, shared links appear as plain text URLs
-- no title, no description, no image. This directly harms click-through
rates and makes the site look unfinished.

Eigen already injects `<link>` tags into `<head>` for preload/prefetch
hints (via `build::hints`). SEO meta tags follow the same pattern:
structured data derived from page metadata, injected into `<head>`
during the build pipeline.

**Goals:**

- Define an `[seo]` section in frontmatter for per-page SEO fields
  (title, description, image, type).
- Provide site-level defaults in `site.toml` (under `[site.seo]`) so
  pages without explicit SEO fields still get reasonable meta tags.
- Auto-inject `og:*` and `twitter:*` meta tags into `<head>` during the
  build pipeline, requiring zero template changes from the user.
- Support dynamic pages: SEO fields can reference collection item data
  via minijinja expressions (e.g. `{{ post.title }}`).
- Generate a canonical URL automatically from `base_url` + page path.
- Skip injection when meta tags are already present in the HTML
  (respect author-provided tags).

**Non-goals (see "Out of Scope"):**

- JSON-LD structured data (schema.org).
- Automated image generation for social cards.
- `<meta name="robots">` or other crawler directives.
- Sitemap XML generation (already handled by `build::sitemap`).

## Architecture

### Approach: Build-Pipeline Injection with Frontmatter + Config Defaults

Three approaches were considered:

1. **Template-only approach.** Provide an `{% include "_partials/seo.html" %}`
   partial that users manually add to their base template. SEO fields are
   exposed as template context variables. The user is responsible for
   including it.

2. **Template function approach.** Provide a `{{ seo_tags() }}` function
   that returns the meta tag HTML string. The user calls it somewhere in
   `<head>`.

3. **Build-pipeline injection.** Auto-inject meta tags during the build
   pipeline using lol_html, the same way resource hints are injected.
   Zero template changes required.

**Chosen: Option 3 (build-pipeline injection).**

Rationale:

- Mirrors the established pattern in `build::hints` -- both inject
  `<link>`/`<meta>` tags into `<head>` using lol_html.
- Requires zero template changes. Every page gets correct SEO tags
  automatically, even if the user forgets to include a partial.
- Handles canonical URL generation automatically from
  `config.site.base_url` and the local `url_path` variable, which are
  already available in the render functions but would require explicit
  passing in a template function.
- For dynamic pages, SEO field values containing template expressions
  are resolved using `minijinja::Environment::render_str` with the same
  context used for the page template, so `{{ post.title }}` works
  naturally.
- The injection step can detect existing OG/Twitter meta tags in the
  rendered HTML and skip injection to avoid duplicates (respecting
  author intent).

**Why not the template approach?** Template approaches require user
action. A new eigen project or a template without the include/function
call would silently produce pages with no SEO tags. Build-pipeline
injection makes correct behavior the default.

### Pipeline Position

The current per-page build pipeline (from `render_static_page` and
`render_dynamic_page` in `src/build/render.rs`):

```
render template
  -> strip fragment markers
  -> localize assets
  -> optimize images (img -> picture)
  -> rewrite CSS background images
  -> plugin post_render_html
  -> critical CSS inlining        (build::critical_css)
  -> preload/prefetch hints       (build::hints)
  -> minify HTML                  (build::minify)
  -> write to disk
```

After all pages are rendered, a separate post-render phase runs:
```
  -> sitemap generation           (build::sitemap)
  -> plugin post_build hooks
  -> CSS/JS bundling              (build::bundling)
  -> content hash rewriting       (build::content_hash)
```

SEO meta tag injection slots in **after preload/prefetch hints and
before minify**:

```
  -> preload/prefetch hints
  -> SEO meta tag injection  <-- NEW  (build::seo)
  -> minify HTML
  -> write to disk
```

**Rationale:**

- It must run after plugins because plugins may inject content that
  affects SEO (e.g. a CMS plugin that adds structured data).
- It must run after image optimization because the SEO image path
  should reference the final optimized path.
- It runs after hints because both inject into `<head>` and there is
  no dependency between them, but hints are performance-critical
  (earlier in `<head>` is better for preload) while SEO tags are not
  position-sensitive within `<head>`.
- It must run before minification so that the injected meta tags
  benefit from `minify-html`.
- Fragments do NOT get SEO meta tags. Fragments are partial HTML
  snippets loaded via HTMX -- they have no `<head>` and are not
  crawled independently.

**Note on content hashing:** The content hash rewrite step runs in
Phase 3, after all pages are written to disk. It rewrites asset URLs
in HTML files on disk. SEO meta tags containing image URLs will be
rewritten automatically by this step, so the SEO injection does not
need to be content-hash-aware. See "Content hashing interaction"
below.

### SEO Field Resolution

SEO meta tag values come from three layers, with later layers
overriding earlier ones:

1. **Auto-derived defaults.** Canonical URL from `base_url + current_url`.
   `og:type` defaults to `"website"`. Title falls back to `site.name`.
2. **Site-level defaults** from `[site.seo]` in `site.toml`. These
   provide fallback values for all pages (e.g. a default description,
   a default share image, a Twitter handle).
3. **Page-level overrides** from `[seo]` in frontmatter. These override
   site defaults for specific pages.

For dynamic pages, page-level SEO fields may contain minijinja template
expressions (e.g. `title: "{{ post.title }} | Blog"`). These are
resolved per-item using `minijinja::Environment::render_str` with the
same context used for the page template. This reuses minijinja's
existing template engine rather than implementing a custom interpolation
mechanism.

The resolution happens inside the render function, after the template
context is built (via `context::build_page_context`) but before the
meta tag injection step. The resolved SEO values are passed to the
injection function as a struct.

**Why `[site.seo]` and not `[build.seo]`?** Other pipeline features
(critical_css, hints, content_hash, bundling) live under `[build.*]`
because they are build-process configuration (enabled flags, tuning
knobs). SEO defaults are site-level metadata -- they describe what the
site *is* (its title, description, image, Twitter handle), not how the
build process should behave. Placing them under `[site.seo]` keeps
site identity separate from build mechanics. There is no `enabled`
flag because SEO tags are always beneficial and cost nothing.

### Existing Meta Tag Detection

If the rendered HTML already contains `<meta property="og:title">`
(or any other OG/Twitter tag), the injection step skips that specific
tag. This allows template authors to manually specify meta tags in
their templates when they need full control, without the build pipeline
duplicating them.

Detection uses lol_html to scan for `<meta>` elements with
`property="og:*"` or `name="twitter:*"` attributes. The set of
already-present tag names is collected first, then only missing tags
are injected.

### Dependencies

No new dependencies. The feature uses:

- `lol_html` (already in Cargo.toml) for HTML parsing and injection.
- `minijinja` (already in Cargo.toml) for resolving template
  expressions in SEO fields.

## Data Models and Types

### Configuration

Add to `src/config/mod.rs`:

```rust
/// Site-level SEO defaults.
///
/// Located under `[site.seo]` in site.toml.
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// Note: when no image is available (neither from frontmatter nor
    /// from this config), `twitter:card` is forced to `"summary"`
    /// regardless of this setting, because `summary_large_image`
    /// requires an image. This override happens in `resolve_seo`.
    #[serde(default = "default_twitter_card")]
    pub twitter_card: String,
}

fn default_og_type() -> String {
    "website".to_string()
}

fn default_twitter_card() -> String {
    "summary_large_image".to_string()
}
```

Add the field to `SiteMeta`:

```rust
pub struct SiteMeta {
    pub name: String,
    pub base_url: String,
    /// Site-level SEO defaults for Open Graph and Twitter Card tags.
    #[serde(default)]
    pub seo: SiteSeoConfig,
}
```

**Migration note:** Adding `seo: SiteSeoConfig` to `SiteMeta` requires
updating all existing code that constructs `SiteMeta` directly (e.g.
test helpers in `context.rs` and `functions.rs`). Since `SiteSeoConfig`
derives `Default` and the field uses `#[serde(default)]`, existing
`site.toml` files and TOML-based construction are unaffected.

### Frontmatter

Add to `src/frontmatter/mod.rs`:

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

Add to `Frontmatter` (note: `Frontmatter` is not serde-deserialized
directly -- it is constructed from `RawFrontmatter` in
`parse_frontmatter`, so no `#[serde]` attribute is needed):

```rust
pub struct Frontmatter {
    // ... existing fields (collection, slug_field, item_as,
    //     data, fragment_blocks, hero_image) ...

    /// SEO metadata for Open Graph and Twitter Card tags.
    pub seo: SeoMeta,
}
```

And to `RawFrontmatter` (which IS serde-deserialized from YAML):

```rust
struct RawFrontmatter {
    // ... existing fields (collection, slug_field, item_as,
    //     data, fragment_blocks, hero_image) ...
    #[serde(default)]
    seo: SeoMeta,
}
```

### Resolved SEO Values

An internal struct used during the build pipeline, after site defaults
and frontmatter values have been merged and template expressions
resolved:

```rust
/// Fully resolved SEO metadata ready for injection into HTML.
///
/// All template expressions have been evaluated. All fallbacks have
/// been applied. URLs are absolute.
///
/// This is NOT a public type -- it lives in `build::seo` and is only
/// used within the injection pipeline.
struct ResolvedSeo {
    /// Page title. Never empty -- falls back chain:
    /// frontmatter.seo.title > site.seo.title > site.name.
    title: String,
    /// Meta description. None if no description is available at any
    /// level (frontmatter.seo.description > site.seo.description).
    description: Option<String>,
    /// Absolute URL to the share image. None if no image is available
    /// (frontmatter.seo.image > site.seo.image).
    image: Option<String>,
    /// Open Graph type (e.g. "website", "article").
    /// Fallback chain: frontmatter.seo.og_type > site.seo.og_type > "website".
    og_type: String,
    /// Twitter card type. Forced to "summary" when image is None.
    /// Otherwise: frontmatter.seo.twitter_card > site.seo.twitter_card
    /// > "summary_large_image".
    twitter_card: String,
    /// Twitter @handle for the site. None if not configured.
    /// Source: site.seo.twitter_site (no per-page override).
    twitter_site: Option<String>,
    /// Canonical URL (absolute). Always present.
    /// Fallback: frontmatter.seo.canonical_url > base_url + current_url.
    canonical_url: String,
    /// The site name for `og:site_name`. Always `site.name`.
    site_name: String,
}
```

## API Surface

### Public Functions

The module `build::seo` exposes two public functions:
`inject_seo_tags` (the pipeline step) and `resolve_seo_expressions`
(template expression evaluation, called earlier in the render flow).

#### `inject_seo_tags`

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
) -> String
```

**Return type rationale:** Same as `inline_critical_css` and
`inject_resource_hints` -- the function returns `String`, not
`Result<String>`. SEO meta tags are an enhancement. If anything goes
wrong, the correct behavior is to return the original HTML unchanged.

**Parameter rationale:**

- `html`: The rendered HTML string from the pipeline.
- `frontmatter_seo`: The page's `[seo]` frontmatter section. For
  dynamic pages, template expressions have already been resolved by
  the caller before passing to this function.
- `site_config`: The `[site]` config section, providing `name`,
  `base_url`, and `seo` defaults.
- `current_url`: The page's URL path (e.g. `/about.html`), used to
  compute the canonical URL.

### Internal Functions

```rust
/// Merge frontmatter SEO fields with site-level defaults to produce
/// fully resolved values.
///
/// Priority: frontmatter > site.seo > auto-derived defaults.
fn resolve_seo(
    frontmatter_seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> ResolvedSeo

/// Scan rendered HTML for existing `<meta property="og:*">` and
/// `<meta name="twitter:*">` tags.
///
/// Returns the set of tag names already present (e.g. "og:title",
/// "twitter:card").
fn detect_existing_tags(html: &str) -> HashSet<String>

/// Also detect existing `<link rel="canonical">` in the HTML.
fn has_canonical_link(html: &str) -> bool

/// Generate meta tag HTML string for all SEO tags that are not
/// already present in the document.
///
/// Returns an empty string if all tags are already present.
fn generate_meta_html(
    seo: &ResolvedSeo,
    existing: &HashSet<String>,
    has_canonical: bool,
) -> String

/// Inject meta tag HTML into the `<head>` element.
///
/// Uses lol_html to append content inside `<head>` (via
/// `el.append(..., ContentType::Html)` on the `head` element).
/// Unlike resource hints (which use `el.prepend` to place hints
/// early in `<head>` for performance), SEO tags are appended late
/// in `<head>` because their position does not affect performance
/// and this avoids interfering with preload hint ordering.
fn inject_into_head(html: &str, meta_html: &str) -> Result<String, String>
```

### Template Expression Resolution

For dynamic pages, SEO field values may contain minijinja expressions.
These are resolved in the render functions (`render_static_page` and
`render_dynamic_page`) before calling `inject_seo_tags`.

This is a second public function on the `build::seo` module (alongside
`inject_seo_tags`), because it is called from `render.rs`:

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
) -> SeoMeta
```

This function is called from `render_static_page` and
`render_dynamic_page` after building the template context but before
the post-render pipeline steps. It returns a new `SeoMeta` with all
expressions evaluated. For static pages, fields without expressions
pass through unchanged. For dynamic pages, expressions like
`{{ post.title }}` are resolved against the current item's context.

### Generated Meta Tags

For a fully configured page, the following tags are generated:

```html
<!-- Open Graph -->
<meta property="og:title" content="Page Title">
<meta property="og:description" content="Page description text">
<meta property="og:image" content="https://example.com/assets/share.jpg">
<meta property="og:url" content="https://example.com/about.html">
<meta property="og:type" content="website">
<meta property="og:site_name" content="My Site">

<!-- Twitter Card -->
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="Page Title">
<meta name="twitter:description" content="Page description text">
<meta name="twitter:image" content="https://example.com/assets/share.jpg">
<meta name="twitter:site" content="@mysite">

<!-- Canonical URL -->
<link rel="canonical" href="https://example.com/about.html">
```

Tags are only generated when their value is available. For example:

- If no `description` is set at any level, `og:description` and
  `twitter:description` are omitted entirely.
- If no `image` is set, `og:image` and `twitter:image` are omitted,
  and `twitter:card` falls back to `"summary"` (no large image).
- If no `twitter_site` is configured, `twitter:site` is omitted.
- `og:title`, `og:url`, `og:type`, `og:site_name`, `twitter:card`,
  and `twitter:title` are always generated (they always have values
  from defaults).

### URL Resolution

Image paths and canonical URLs must be absolute for social media
crawlers. The resolution rules:

- If the value starts with `http://` or `https://`, it is already
  absolute -- use as-is.
- If the value starts with `/`, it is site-relative -- prepend
  `base_url` (e.g. `/assets/hero.jpg` becomes
  `https://example.com/assets/hero.jpg`).
- The canonical URL is always `base_url + current_url` unless
  explicitly overridden in frontmatter.

The `base_url` trailing slash is normalized: if `base_url` ends with
`/` and the path starts with `/`, one slash is removed to avoid
`https://example.com//about.html`.

## Error Handling

| Scenario | Behavior |
|---|---|
| No `[seo]` in frontmatter and no `[site.seo]` in config | Generate tags with `site.name` as title, `og:type` "website", canonical URL. No description/image tags. |
| Template expression in SEO field fails to render | Log warning, treat field as `None` (fall back to site default) |
| Template expression renders to empty string | Treat as `None` (fall back to site default) |
| lol_html injection error | Log warning, return original HTML unchanged |
| No `<head>` element in HTML | lol_html selector does not match; return HTML unchanged (no crash) |
| All OG/Twitter tags already exist in HTML | No injection, return HTML unchanged (no-op) |
| `image` path does not exist on disk | Not validated. The path is used as-is. (Validation is out of scope.) |
| SEO field value contains HTML entities or special characters | Escaped via `escape_attr` when building the meta tag HTML string (see "Special characters" below) |

Note: `base_url` being empty is not a runtime concern because
`validate_config` in `config/mod.rs` already rejects empty `base_url`
at config load time, before the build starts.

**Principle:** SEO meta tags are an enhancement. Failures must never
break the build or degrade the output. Every error path falls back to
returning the original HTML unchanged.

## Edge Cases and Failure Modes

### Content hashing interaction

If content hashing is enabled, the `seo.image` path should reference
the original (pre-hashed) path. The content hash rewrite step (Phase 3
in `build()`) runs after all pages are rendered and rewrites URLs in
the HTML files on disk. Since our meta tags are already in the HTML at
that point, content hash rewriting will update the image URL in the
meta tag automatically.

This means the SEO injection step does not need to know about content
hashing. It uses the original path, and the hash rewrite step handles
the rest. This is the same behavior as `<img>` tags and CSS
`url()` references.

### Bundling interaction

CSS/JS bundling (`build::bundling`) runs in a post-render phase after
all pages are written to disk. It rewrites `<link>` and `<script>` tags
in HTML files. This does not affect SEO meta tags because meta tags
reference image URLs and page URLs, not CSS/JS files. No conflict.

### Image optimization interaction

If the SEO image is also referenced in an `<img>` tag on the page, it
will go through the img-to-picture optimization step. The `og:image`
meta tag should reference the original image format (JPEG/PNG), not
the optimized format (WebP/AVIF), because social media crawlers have
inconsistent support for modern formats.

Since the SEO image path comes from frontmatter (not from the rendered
HTML), it naturally references the original path. The image optimization
step only rewrites `<img>` tags, not `<meta>` tags. No conflict.

### Dynamic page SEO with collection item data

For a dynamic blog post template:

```yaml
---
collection:
  source: blog_api
  path: /posts
item_as: post
seo:
  title: "{{ post.title }} | My Blog"
  description: "{{ post.excerpt }}"
  image: "{{ post.featured_image }}"
  og_type: article
---
```

The `item_as: post` field (from `Frontmatter`) determines the variable
name under which each collection item is exposed in the template context.
The expressions `{{ post.title }}`, `{{ post.excerpt }}`, and
`{{ post.featured_image }}` are resolved per-item using
`resolve_seo_expressions` with the same template context that contains
the `post` variable (built by `context::build_page_context` with the
`item` parameter). If `post.excerpt` is missing or empty, the
description falls back to the site default.

### Special characters in SEO field values

Titles and descriptions may contain characters that need HTML
attribute escaping: `"`, `&`, `<`, `>`. The `generate_meta_html`
function must escape these in the `content` attribute value.

We use a simple escaping function (not a full HTML library) that
replaces `&` with `&amp;`, `"` with `&quot;`, `<` with `&lt;`, and
`>` with `&gt;`. This is sufficient for meta tag content attributes.

### Trailing slash normalization

`base_url` may or may not end with `/`. Page URLs always start with
`/`. The canonical URL computation must handle this:

```
"https://example.com"  + "/about.html" -> "https://example.com/about.html"
"https://example.com/" + "/about.html" -> "https://example.com/about.html"
```

### Pages without `<head>`

Fragment files and non-HTML pages have no `<head>` element. The
lol_html `head` selector simply does not match, and the function
returns the HTML unchanged. No error, no warning.

### `index.html` canonical URL

For `index.html` pages, the canonical URL should be the directory
path, not `/index.html`. For example:

- `/index.html` -> `https://example.com/`
- `/blog/index.html` -> `https://example.com/blog/`

This is a common SEO best practice. The `resolve_seo` function strips
`index.html` from the canonical URL path.

### Dev server

SEO meta tag injection is **disabled during dev server** regardless of
config. The dev server render functions (`render_static_page_dev`,
`render_dynamic_page_dev` in `src/dev/rebuild.rs`) do not call the
post-render optimization steps. SEO tags are only useful in production
where pages are crawled by social media bots.

### Existing `<title>` tag

The `<title>` element is separate from `og:title`. This feature does
NOT modify the `<title>` element. It only injects `<meta>` tags.
Template authors remain responsible for setting `<title>` in their
templates.

### Multiple OG images

The Open Graph protocol supports multiple `og:image` tags. This design
supports only a single image per page. This covers the vast majority
of use cases. Supporting multiple images would add complexity
(array-typed frontmatter fields) for negligible benefit.

## Configuration Examples

### Minimal (zero config)

With no `[site.seo]` in site.toml and no `[seo]` in frontmatter, only
the required `[site]` fields:

```toml
[site]
name = "My Site"
base_url = "https://example.com"
```

Every page gets (e.g. for `/about.html`):
```html
<meta property="og:title" content="My Site">
<meta property="og:url" content="https://example.com/about.html">
<meta property="og:type" content="website">
<meta property="og:site_name" content="My Site">
<meta name="twitter:card" content="summary">
<meta name="twitter:title" content="My Site">
<link rel="canonical" href="https://example.com/about.html">
```

Note: `twitter:card` defaults to `"summary"` (not `"summary_large_image"`)
when no image is available at any level, because `summary_large_image`
without an image produces a broken card on most platforms.

### Site-level defaults

```toml
[site]
name = "My Blog"
base_url = "https://blog.example.com"

[site.seo]
description = "A blog about interesting things"
image = "/assets/default-share.jpg"
twitter_site = "@myblog"
```

Pages without frontmatter `[seo]` inherit these defaults.

### Static page with full SEO

```yaml
---
seo:
  title: About Us
  description: Learn about our team and mission
  image: /assets/about-hero.jpg
  og_type: website
---
```

### Dynamic page with item data

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

### Override canonical URL

```yaml
---
seo:
  canonical_url: https://original-source.com/article
---
```

Useful when syndicating content from another source.

## What Is NOT In Scope

1. **JSON-LD structured data.** Schema.org markup is a separate concern
   with different rules and much higher complexity (nested objects,
   type-specific schemas). It deserves its own feature.

2. **Automated social card image generation.** Generating images with
   the page title overlaid on a template is a complex feature requiring
   an image rendering library or headless browser.

3. **`<meta name="robots">` directives.** Crawler directives
   (`noindex`, `nofollow`, etc.) are an SEO concern but unrelated to
   social sharing meta tags.

4. **Modifying `<title>`.** The `<title>` element is the template
   author's responsibility. OG/Twitter meta tags are separate.

5. **Multiple `og:image` tags.** Only one image per page is supported.

6. **`og:locale` tag.** Internationalization and locale detection are
   separate features.

7. **Image dimension tags.** `og:image:width` and `og:image:height`
   require reading image metadata from disk. This adds I/O and
   complexity for marginal benefit (most platforms fetch and inspect
   the image anyway).

8. **Validation of SEO field values.** We do not validate that image
   paths exist on disk, that descriptions are within recommended
   length limits, or that titles follow best practices.

9. **Per-field configuration for enabling/disabling individual tags.**
   Either all SEO tags are injected (based on available data) or none
   are. There is no `seo.skip_twitter = true` option.

## Module Structure

```
src/build/
  seo.rs          -- public API (inject_seo_tags), resolution logic,
                     meta tag generation, HTML injection, existing
                     tag detection
```

A single file because the feature is focused and the code is
straightforward:

- `resolve_seo`: merging config layers (~30 lines)
- `detect_existing_tags` + `has_canonical_link`: lol_html scan (~30 lines)
- `generate_meta_html`: string building (~50 lines)
- `inject_into_head`: lol_html injection (~20 lines)
- `inject_seo_tags`: orchestration (~20 lines)
- `resolve_seo_expressions`: minijinja rendering (~30 lines)
- `escape_attr`: HTML attribute escaping (~10 lines)
- Tests (~200 lines)

Total: ~390 lines. Well within the threshold for a single file.

This follows the pattern of `build/minify.rs` (single-file module for
a focused build step) rather than `build/critical_css/` (multi-file
module for a complex feature).

## Integration Points

### config/mod.rs

Add `SiteSeoConfig` struct and the `seo` field to `SiteMeta`.

The `site` global variable in `template/functions.rs` currently
exposes only `name` and `base_url` (constructed manually from
`config.site.name` and `config.site.base_url`). The `seo` sub-object
does not need to be exposed to templates -- SEO values are resolved
in the build pipeline, not in templates. No changes to `functions.rs`
are needed.

Test helpers that construct `SiteMeta` directly (in `context.rs` and
`functions.rs` tests) must be updated to include the new `seo` field.
Since `SiteSeoConfig` derives `Default`, these helpers can use
`seo: SiteSeoConfig::default()` or the struct update syntax.

### frontmatter/mod.rs

Add `SeoMeta` struct and the `seo` field to both `Frontmatter` and
`RawFrontmatter`.

Update `parse_frontmatter` to pass through the parsed SEO data:

```rust
Ok(Frontmatter {
    collection: raw.collection,
    slug_field: raw.slug_field.unwrap_or_else(|| "slug".into()),
    item_as: raw.item_as.unwrap_or_else(|| "item".into()),
    data: raw.data,
    fragment_blocks: raw.fragment_blocks,
    hero_image: raw.hero_image,
    seo: raw.seo,  // <-- NEW
})
```

Update `Frontmatter::default()` to include `seo: SeoMeta::default()`.

### build/mod.rs

Add `pub mod seo;` to the module declarations. The current module list
(from `src/build/mod.rs`) is:

```rust
pub mod bundling;
pub mod content_hash;
pub mod context;
pub mod critical_css;
pub mod fragments;
pub mod hints;
pub mod minify;
pub mod output;
pub mod render;
pub mod seo;         // <-- NEW
pub mod sitemap;
```

### build/render.rs

In `render_static_page` and `render_dynamic_page`, add the SEO step
after hints and before minification. Two integration points per
function:

1. **Early:** Call `resolve_seo_expressions` after
   `context::build_page_context` builds the template context (`ctx`)
   but before rendering. This is where template expressions like
   `{{ post.title }}` are evaluated.

2. **Late:** Call `inject_seo_tags` in the pipeline after
   `hints::inject_resource_hints` and before `minify::minify_html`.

**In `render_static_page`:**

```rust
// After: let ctx = context::build_page_context(...);
// Before: let rendered = tmpl.render(&ctx)?;

// Resolve SEO template expressions (static pages rarely use these,
// but support them for consistency).
let resolved_seo = seo::resolve_seo_expressions(
    &page.frontmatter.seo,
    env,
    &ctx,
);

// ... (existing pipeline steps: render, strip markers, localize,
//      optimize images, plugins, critical CSS, hints) ...

// SEO meta tag injection (after hints, before minify).
// Step 4f in the pipeline.
let full_html = seo::inject_seo_tags(
    &full_html,
    &resolved_seo,
    &config.site,
    &url_path,
);
```

**In `render_dynamic_page` (inside the per-item loop):**

```rust
// After: let ctx = context::build_page_context(..., Some((item_as, item)));
// Before: let rendered = tmpl.render(&ctx)?;

// Resolve SEO template expressions for this item.
let resolved_seo = seo::resolve_seo_expressions(
    &page.frontmatter.seo,
    env,
    &ctx,
);

// ... (existing pipeline steps) ...

// SEO meta tag injection.
let full_html = seo::inject_seo_tags(
    &full_html,
    &resolved_seo,
    &config.site,
    &url_path,
);
```

Note: `resolve_seo_expressions` is called after `build_page_context`
(which creates the template context as a `minijinja::Value`) and
before the pipeline steps. The resolved SEO is passed to
`inject_seo_tags` later in the pipeline. No `?` operator needed --
both functions are infallible. No new parameters are needed on the
render function signatures -- `env` and `config.site` are already
available.

## Test Plan

Tests follow the project convention: test behavior, not library
internals. Do not use `unwrap`/`expect` unless it is an invariant.

### Unit tests in `seo.rs`

| Test | What it verifies |
|---|---|
| `test_resolve_seo_all_defaults` | With empty `SeoMeta` and minimal `SiteMeta`, title is site.name, og_type is "website", canonical URL is base_url + path |
| `test_resolve_seo_frontmatter_overrides` | Frontmatter title/description/image override site defaults |
| `test_resolve_seo_site_defaults_fill_gaps` | `site.seo.description` used when frontmatter has no description |
| `test_resolve_seo_canonical_url_auto` | Canonical URL is `base_url + current_url` |
| `test_resolve_seo_canonical_url_override` | Explicit `canonical_url` in frontmatter is used verbatim |
| `test_resolve_seo_canonical_strips_index` | `/index.html` -> `base_url + /`, `/blog/index.html` -> `base_url + /blog/` |
| `test_resolve_seo_image_absolute_url` | Image starting with `https://` is used as-is |
| `test_resolve_seo_image_relative_path` | Image starting with `/` gets `base_url` prepended |
| `test_resolve_seo_base_url_slash_normalization` | No double slashes when base_url ends with `/` and path starts with `/` |
| `test_resolve_seo_twitter_card_no_image` | `twitter:card` is `"summary"` when no image is available |
| `test_resolve_seo_twitter_card_with_image` | `twitter:card` is `"summary_large_image"` when image is available |
| `test_detect_existing_og_tags` | Detects `<meta property="og:title">` in HTML |
| `test_detect_existing_twitter_tags` | Detects `<meta name="twitter:card">` in HTML |
| `test_detect_no_existing_tags` | Returns empty set for HTML without OG/Twitter tags |
| `test_has_canonical_link` | Detects `<link rel="canonical">` |
| `test_generate_meta_html_full` | All tags generated with correct attributes |
| `test_generate_meta_html_no_description` | `og:description` and `twitter:description` omitted when None |
| `test_generate_meta_html_no_image` | `og:image` and `twitter:image` omitted when None |
| `test_generate_meta_html_skips_existing` | Tags in the existing set are not duplicated |
| `test_generate_meta_html_skips_canonical` | Canonical link not generated when already present |
| `test_escape_attr_special_chars` | `"`, `&`, `<`, `>` are escaped in content attributes |
| `test_inject_seo_tags_basic` | Full pipeline: meta tags appear in `<head>` |
| `test_inject_seo_tags_no_head` | HTML without `<head>` returns unchanged |
| `test_inject_seo_tags_all_existing` | All tags already present, returns HTML unchanged |
| `test_inject_seo_tags_partial_existing` | Only missing tags are injected |
| `test_resolve_seo_expressions_basic` | `{{ post.title }}` is resolved from context |
| `test_resolve_seo_expressions_missing_var` | Missing variable logs warning, field becomes None |
| `test_resolve_seo_expressions_empty_result` | Expression rendering to empty string treated as None |
| `test_resolve_seo_expressions_no_expressions` | Literal values pass through unchanged |

### Configuration tests in `config/mod.rs`

| Test | What it verifies |
|---|---|
| `test_site_seo_config_defaults` | Empty config gives default `og_type` and `twitter_card` |
| `test_site_seo_config_custom` | All fields parse correctly from TOML |
| `test_site_seo_config_partial` | Some fields set, others use defaults |

### Frontmatter tests in `frontmatter/mod.rs`

| Test | What it verifies |
|---|---|
| `test_parse_seo_frontmatter_full` | All SEO fields parsed from YAML |
| `test_parse_seo_frontmatter_partial` | Some fields set, others None |
| `test_parse_seo_frontmatter_absent` | No `seo:` section gives `SeoMeta::default()` |
| `test_parse_seo_with_template_expressions` | Expressions stored as literal strings (not evaluated at parse time) |

### What we do NOT test

- lol_html's attribute parsing (that is the library's job).
- minijinja's `render_str` correctness (that is the library's job).
- That social media crawlers correctly parse the generated tags (that
  is the crawler's job).
- That generated URLs are valid (we trust `base_url` from config;
  `validate_config` rejects empty `base_url` at load time).
