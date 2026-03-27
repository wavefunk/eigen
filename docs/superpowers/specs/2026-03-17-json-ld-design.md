# JSON-LD Structured Data -- Design Spec

Date: 2026-03-17

## Motivation and Goals

Search engines (Google, Bing) use structured data to understand page content
and generate rich results (rich snippets, knowledge panels, breadcrumb trails,
sitelinks search boxes). The primary format for structured data is JSON-LD
(JavaScript Object Notation for Linked Data) -- a `<script type="application/ld+json">`
block embedded in the page's `<head>` or `<body>`.

Eigen already auto-injects Open Graph and Twitter Card meta tags via
`build::seo` for social sharing previews. JSON-LD structured data is the
complementary piece for search engine enrichment. The OG/Twitter meta spec
explicitly listed JSON-LD as out of scope:

> "JSON-LD structured data. Schema.org markup is a separate concern with
> different rules and much higher complexity (nested objects, type-specific
> schemas). It deserves its own feature."

This feature delivers that separate concern.

**Goals:**

- Support three schema.org types out of the box: `Article`,
  `BreadcrumbList`, and `WebSite`.
- Define a `schema` field in frontmatter that maps to schema.org types and
  allows per-page schema configuration.
- Auto-populate JSON-LD fields from existing page context (URL, title,
  dates, images) and site configuration, minimizing what the user must
  specify manually.
- Inject the JSON-LD `<script>` block into `<head>` during the build
  pipeline, requiring zero template changes from the user.
- Support dynamic pages: schema fields can reference collection item data
  via minijinja expressions (e.g. `{{ post.title }}`).
- Skip injection when a `<script type="application/ld+json">` block is
  already present in the HTML (respect author-provided structured data).
- Produce valid JSON-LD that passes Google's Rich Results Test.

**Non-goals (see "Out of Scope"):**

- Schema.org types beyond Article, BreadcrumbList, and WebSite.
- Schema.org validation or linting of generated output.
- Nested schema composition (e.g. embedding a Review inside a Product).
- Microdata or RDFa formats (JSON-LD only).

## Architecture

### Approach: Build-Pipeline Injection with Frontmatter + Config Defaults

This follows the same approach as `build::seo` -- build-pipeline injection
using `lol_html`. The rationale is identical:

- Mirrors the established pattern in `build::seo` and `build::hints`.
- Requires zero template changes.
- Auto-populates fields from page context and site config.
- For dynamic pages, expressions are resolved via `minijinja::render_str`.
- Detects existing JSON-LD blocks and skips injection to avoid duplicates.

**Why not a template function?** Same reasoning as OG/Twitter tags. A
template function like `{{ json_ld() }}` requires user action. If the user
forgets to include it, pages silently lack structured data. Build-pipeline
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
  -> SEO meta tag injection       (build::seo)
  -> minify HTML                  (build::minify)
  -> write to disk
```

JSON-LD injection slots in **after SEO meta tags and before minify**:

```
  -> SEO meta tag injection       (build::seo)
  -> JSON-LD injection            (build::json_ld)  <-- NEW
  -> minify HTML                  (build::minify)
  -> write to disk
```

**Rationale:**

- It must run after SEO meta tag injection because both inject into
  `<head>`. Ordering them separately makes debugging easier (each step
  adds one concern).
- It must run after image optimization because JSON-LD `image` fields
  should reference the final asset paths.
- It must run after plugins because plugins may inject content that
  affects page structure.
- It must run before minification so that the injected JSON-LD block
  benefits from `minify-html`.
- It does not need to run after hints because there is no dependency
  between preload hints and structured data.
- Fragments do NOT get JSON-LD. Fragments are partial HTML snippets
  loaded via HTMX -- they have no `<head>` and are not crawled
  independently.

**Note on content hashing:** The content hash rewrite step runs in
Phase 3, after all pages are written to disk. JSON-LD blocks containing
image URLs will be rewritten by the content hash step automatically (it
rewrites all URLs in HTML files). The JSON-LD injection does not need to
be content-hash-aware.

### Schema Type Selection

The user specifies which schema type(s) to generate via the `schema`
field in frontmatter. This is a YAML value that can take three forms:

1. **String (single type):** `schema: Article`
2. **List (multiple types):** `schema: [Article, BreadcrumbList]`
3. **Map (type with overrides):** `schema: { type: Article, author: "Jane" }`

When `schema` is absent from frontmatter, the behavior depends on
site-level configuration:

- If `[site.schema]` is configured in `site.toml`, site-level defaults
  determine whether any schemas are generated.
- If no schema configuration exists at any level, no JSON-LD is injected
  (unlike OG/Twitter tags, structured data is opt-in because not all
  pages benefit from it).

**Why opt-in?** OG/Twitter meta tags are universally useful (every shared
link benefits). Structured data is type-specific: an Article schema on a
navigation page or a 404 page would be incorrect and could harm search
rankings. The user must consciously opt in to structured data on pages
where it is appropriate.

### Schema Field Resolution

Schema field values come from three layers, with later layers overriding
earlier ones:

1. **Auto-derived fields.** Populated from page context and site config:
   - `url`: `base_url + current_url`
   - `name` / `headline`: `site.name` or page title (from `seo.title`)
   - `datePublished` / `dateModified`: from collection item fields if
     available
   - `image`: from `seo.image` if available
   - `description`: from `seo.description` if available
   - `publisher.name`: `site.name`

2. **Site-level defaults** from `[site.schema]` in `site.toml`. These
   provide fallback values (e.g. a default author, organization info,
   default schemas to apply to all pages).

3. **Page-level overrides** from `schema` in frontmatter. These override
   site defaults for specific pages.

For dynamic pages, page-level schema fields may contain minijinja
template expressions. These are resolved per-item using the same
mechanism as `seo::resolve_seo_expressions`.

### Supported Schema Types

#### Article

Maps to `schema.org/Article`. Used for blog posts, news articles, and
similar content pages.

Generated fields:
```json
{
  "@context": "https://schema.org",
  "@type": "Article",
  "headline": "<title>",
  "description": "<description>",
  "image": "<image_url>",
  "url": "<canonical_url>",
  "datePublished": "<date>",
  "dateModified": "<date>",
  "author": {
    "@type": "Person",
    "name": "<author_name>"
  },
  "publisher": {
    "@type": "Organization",
    "name": "<site_name>"
  }
}
```

Auto-population:
- `headline`: `seo.title` > `site.seo.title` > `site.name`
- `description`: `seo.description` > `site.seo.description`
- `image`: `seo.image` (absolute URL) > `site.seo.image`
- `url`: canonical URL (same logic as `build::seo`)
- `datePublished`: from `schema.date_published` in frontmatter, or from
  collection item data
- `dateModified`: from `schema.date_modified` in frontmatter, or falls
  back to `datePublished`
- `author`: from `schema.author` in frontmatter or `site.schema.author`
  in config
- `publisher.name`: always `site.name`

#### BreadcrumbList

Maps to `schema.org/BreadcrumbList`. Generated from the page URL path.

For a page at `/blog/posts/my-article.html`, the breadcrumb list is:

```json
{
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  "itemListElement": [
    {
      "@type": "ListItem",
      "position": 1,
      "name": "Home",
      "item": "https://example.com/"
    },
    {
      "@type": "ListItem",
      "position": 2,
      "name": "blog",
      "item": "https://example.com/blog/"
    },
    {
      "@type": "ListItem",
      "position": 3,
      "name": "posts",
      "item": "https://example.com/blog/posts/"
    },
    {
      "@type": "ListItem",
      "position": 4,
      "name": "my-article",
      "item": "https://example.com/blog/posts/my-article.html"
    }
  ]
}
```

Auto-population:
- Path segments are derived from `current_url`.
- The first item is always "Home" pointing to `base_url + /`.
- Segment names are the raw path component (e.g. "blog", "posts").
  The user can override names via `schema.breadcrumb_names` in
  frontmatter (a map of segment index to display name).

#### WebSite

Maps to `schema.org/WebSite`. Typically used on the homepage only.
Enables the sitelinks search box in Google results.

```json
{
  "@context": "https://schema.org",
  "@type": "WebSite",
  "name": "<site_name>",
  "url": "<base_url>",
  "description": "<site_description>"
}
```

Auto-population:
- `name`: `site.name`
- `url`: `site.base_url`
- `description`: `site.seo.description`

### Existing JSON-LD Detection

If the rendered HTML already contains a `<script type="application/ld+json">`
block, the injection step detects it and skips injection entirely.

**Why skip entirely instead of per-type?** Unlike OG/Twitter tags (which
are individual `<meta>` elements that can be detected per-tag), JSON-LD
blocks are opaque JSON blobs. Parsing the existing block to determine
which schema types are already present would add complexity (JSON parsing,
`@type` extraction, handling `@graph` arrays). The simpler approach is:
if any JSON-LD exists, respect the author's full control and inject
nothing.

This is consistent with the principle: if the author has opted to manage
structured data manually, the build pipeline should not interfere.

### Dependencies

No new dependencies. The feature uses:

- `lol_html` (already in Cargo.toml) for HTML parsing and injection.
- `minijinja` (already in Cargo.toml) for resolving template expressions.
- `serde_json` (already in Cargo.toml) for building and serializing the
  JSON-LD objects.

## Data Models and Types

### Configuration

Add to `src/config/mod.rs`:

```rust
/// Site-level structured data (JSON-LD) defaults.
///
/// Located under `[site.schema]` in site.toml.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SiteSchemaConfig {
    /// Default author name for Article schemas.
    /// Used when a page does not specify an author in frontmatter.
    pub author: Option<String>,

    /// Default schema types to apply to all pages.
    /// Example: ["BreadcrumbList"]
    /// Pages can override this with their own `schema` frontmatter field.
    #[serde(default)]
    pub default_types: Vec<String>,
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
    /// Site-level structured data (JSON-LD) defaults.
    #[serde(default)]
    pub schema: SiteSchemaConfig,
}
```

**Migration note:** Adding `schema: SiteSchemaConfig` to `SiteMeta`
requires updating all existing code that constructs `SiteMeta` directly
(test helpers in `context.rs`, `sitemap.rs`, `discovery/mod.rs`,
`template/environment.rs`, `template/functions.rs`, `template/filters.rs`,
and `build/seo.rs`). Since `SiteSchemaConfig` derives `Default` and the
field uses `#[serde(default)]`, existing `site.toml` files and
TOML-based construction are unaffected.

### Frontmatter

Add to `src/frontmatter/mod.rs`:

```rust
/// Per-page structured data (JSON-LD) configuration.
///
/// The inner value is `Option<SchemaConfigValue>` to cleanly handle
/// the "absent" case via serde's `#[serde(default)]`. When the `schema`
/// key is absent from YAML, serde gives `None`. When present, serde
/// deserializes the value as one of the `SchemaConfigValue` variants.
pub type SchemaConfig = Option<SchemaConfigValue>;

/// The actual schema configuration, deserialized via `#[serde(untagged)]`.
///
/// Supported formats:
/// - `schema: Article`  (single type, string)
/// - `schema: [Article, BreadcrumbList]`  (multiple types, list)
/// - `schema: { type: Article, author: "Jane" }`  (type with overrides)
///
/// For dynamic pages, override values may contain minijinja template
/// expressions.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SchemaConfigValue {
    /// Single schema type name (e.g. "Article").
    TypeName(String),
    /// List of schema type names.
    TypeList(Vec<String>),
    /// Full configuration with type and field overrides.
    Full(SchemaFullConfig),
}
```

```rust
/// Full schema configuration with type and field overrides.
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaFullConfig {
    /// The schema.org type(s) to generate.
    /// Can be a single string or a list.
    /// Uses `#[serde(alias)]` so YAML can use either `type` or `types`.
    #[serde(alias = "type")]
    pub types: SchemaTypes,

    /// Author name override for Article schema.
    pub author: Option<String>,

    /// Date published override (ISO 8601 string).
    pub date_published: Option<String>,

    /// Date modified override (ISO 8601 string).
    pub date_modified: Option<String>,

    /// Breadcrumb display name overrides.
    /// Maps path segment (by name) to a display label.
    /// Example: { "blog": "Blog Posts", "posts": "Archive" }
    pub breadcrumb_names: Option<HashMap<String, String>>,
}
```

```rust
/// Schema type specification -- single string or list.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SchemaTypes {
    Single(String),
    Multiple(Vec<String>),
}

impl SchemaTypes {
    pub fn to_vec(&self) -> Vec<&str> {
        match self {
            SchemaTypes::Single(s) => vec![s.as_str()],
            SchemaTypes::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}
```

Add to `Frontmatter` and `RawFrontmatter`:

```rust
pub struct Frontmatter {
    // ... existing fields ...

    /// Structured data (JSON-LD) configuration.
    pub schema: SchemaConfig,
}
```

```rust
struct RawFrontmatter {
    // ... existing fields ...
    #[serde(default)]
    schema: SchemaConfig,
}
```

The `SchemaConfig` type alias (`Option<SchemaConfigValue>`) handles the
three frontmatter formats cleanly:
- YAML string `schema: Article` -> `Some(SchemaConfigValue::TypeName("Article"))`
- YAML list `schema: [Article, BreadcrumbList]` -> `Some(SchemaConfigValue::TypeList(...))`
- YAML map `schema: { type: Article, author: "Jane" }` -> `Some(SchemaConfigValue::Full(...))`
- Absent `schema` key -> `None` (via `#[serde(default)]` on the field)

**Why `Option<SchemaConfigValue>` instead of a `None` variant?** Serde's
`#[serde(untagged)]` enums do not support the `#[default]` attribute.
Using `Option<T>` with `#[serde(default)]` on the field is the idiomatic
serde approach for optional untagged enums. When the YAML key is absent,
serde applies `Default::default()` for `Option<T>`, which is `None`. When
the key is present but has a YAML null value (`schema:`), serde treats it
as `None` because `Option<T>` handles null natively.

### Resolved Schema Data

The builder functions (`build_article_schema`, `build_breadcrumb_schema`,
`build_website_schema`) construct `serde_json::Value` objects directly
rather than going through intermediate Rust structs. This avoids
unnecessary allocations -- the JSON-LD objects are built once, serialized
to a string, and discarded. The conceptual fields for each schema type
are documented in the "Supported Schema Types" section above.

## API Surface

### Public Functions

The module `build::json_ld` exposes two public functions:
`inject_json_ld` (the pipeline step) and `resolve_schema_expressions`
(template expression evaluation, called earlier in the render flow).

#### `inject_json_ld`

```rust
/// Inject JSON-LD structured data script block into HTML.
///
/// This is the main entry point called from the build pipeline.
/// It is infallible by design: any internal error causes a fallback
/// to returning the original HTML unchanged, with a warning logged.
///
/// Steps:
/// 1. Determine which schema types to generate (from frontmatter + site config).
/// 2. If no types, return HTML unchanged (no-op).
/// 3. Check if JSON-LD already exists in the HTML (skip if so).
/// 4. Build JSON-LD objects and serialize to JSON.
/// 5. Inject the `<script type="application/ld+json">` block(s) into `<head>`.
///
/// Returns the (possibly rewritten) HTML string.
pub fn inject_json_ld(
    html: &str,
    schema: &SchemaConfig,
    seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> String
```

**Return type rationale:** Same as `inject_seo_tags` -- returns `String`,
not `Result<String>`. JSON-LD is an enhancement. If anything goes wrong,
return the original HTML unchanged.

**Parameter rationale:**

- `html`: The rendered HTML string from the pipeline.
- `schema`: The page's schema configuration from frontmatter
  (`Option<SchemaConfigValue>`). For dynamic pages, template expressions
  have already been resolved by the caller.
- `seo`: The page's resolved SEO metadata (already resolved by
  `seo::resolve_seo_expressions`). Used to auto-populate title,
  description, and image fields.
- `site_config`: The `[site]` config section, providing `name`,
  `base_url`, `seo`, and `schema` defaults.
- `current_url`: The page's URL path (e.g. `/about.html`).

#### `resolve_schema_expressions`

```rust
/// Resolve template expressions in schema frontmatter fields.
///
/// Uses `minijinja::Environment::render_str` to evaluate expressions
/// like `{{ post.title }}` in schema override fields.
///
/// If rendering fails for any field, the field is left as `None`
/// (falls back to auto-derived value) and a warning is logged.
///
/// For fields that contain no template expressions, the value is
/// returned as-is without calling render_str (fast path).
pub fn resolve_schema_expressions(
    schema: &SchemaConfig,
    env: &minijinja::Environment<'_>,
    ctx: &minijinja::Value,
) -> SchemaConfig
```

### Internal Functions

```rust
/// Check if the HTML already contains a `<script type="application/ld+json">`
/// block.
fn has_existing_json_ld(html: &str) -> bool

/// Determine which schema types to generate from page-level and site-level
/// config.
///
/// Returns an empty vec if no schemas should be generated.
fn resolve_schema_types(
    schema: &SchemaConfig,
    site_schema: &SiteSchemaConfig,
) -> Vec<String>

/// Build the Article JSON-LD object.
fn build_article_schema(
    schema: &SchemaConfig,
    seo: &SeoMeta,
    site_config: &SiteMeta,
    current_url: &str,
) -> serde_json::Value

/// Build the BreadcrumbList JSON-LD object from the page URL path.
/// Returns `None` for root pages (single-item breadcrumbs are not useful).
fn build_breadcrumb_schema(
    schema: &SchemaConfig,
    site_config: &SiteMeta,
    current_url: &str,
) -> Option<serde_json::Value>

/// Build the WebSite JSON-LD object.
fn build_website_schema(
    site_config: &SiteMeta,
) -> serde_json::Value

/// Inject a JSON-LD script block into the `<head>` element.
///
/// Uses lol_html to append content inside `<head>`.
fn inject_into_head(html: &str, json_ld_html: &str) -> Result<String, String>
```

### Generated JSON-LD

For a fully configured Article page:

```html
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "Article",
  "headline": "My Blog Post",
  "description": "An introduction to my blog post",
  "image": "https://example.com/assets/hero.jpg",
  "url": "https://example.com/blog/my-post.html",
  "datePublished": "2026-03-15",
  "dateModified": "2026-03-17",
  "author": {
    "@type": "Person",
    "name": "Jane Doe"
  },
  "publisher": {
    "@type": "Organization",
    "name": "My Blog"
  }
}
</script>
```

For multiple schema types on the same page, each is a separate
`<script type="application/ld+json">` block. Google recommends this
over a single `@graph` array for simplicity and clarity.

Fields are only included when their value is available:
- If no `description` is set, the field is omitted.
- If no `image` is set, the field is omitted.
- If no `datePublished` is set, date fields are omitted.
- If no `author` is set, the author field is omitted.
- `headline`, `url`, `publisher.name` always have values from defaults.

### URL Resolution

Image URLs in JSON-LD must be absolute. The same `make_absolute_url`
function from `build::seo` is reused (or a local equivalent with the
same logic): if the URL starts with `http://` or `https://`, use as-is;
if it starts with `/`, prepend `base_url`.

## Error Handling

| Scenario | Behavior |
|---|---|
| No `schema` in frontmatter and no `[site.schema]` in config | No JSON-LD injected. Silent no-op. |
| Schema type not recognized (e.g. "Product") | Log warning, skip that schema type. Generate others if any. |
| Template expression in schema field fails to render | Log warning, treat field as `None` (fall back to auto-derived value) |
| Template expression renders to empty string | Treat as `None` (fall back to auto-derived value) |
| lol_html injection error | Log warning, return original HTML unchanged |
| No `<head>` element in HTML | lol_html selector does not match; return HTML unchanged |
| JSON-LD already exists in HTML | Skip injection entirely, return HTML unchanged |
| `serde_json` serialization error | Log warning, return original HTML unchanged (should not happen with well-formed data) |
| `seo` fields not available (no SEO config) | Auto-derived fields use site.name and base_url only. Description, image, dates omitted. |
| `SchemaConfig` deserialization from frontmatter fails for `schema` field | If the `schema` key is present but none of the untagged variants match, `serde_yaml` returns an error. The `parse_frontmatter` function propagates this. Users should either omit the key or provide a valid value. |

**Principle:** JSON-LD is an enhancement. Failures must never break the
build or degrade the output. Every error path falls back to returning
the original HTML unchanged.

## Edge Cases and Failure Modes

### Content hashing interaction

Same as SEO meta tags: JSON-LD image URLs reference pre-hashed paths.
The content hash rewrite step (Phase 3) rewrites URLs in HTML files on
disk, including those inside `<script>` blocks. No special handling
needed.

### Bundling interaction

CSS/JS bundling does not affect JSON-LD because structured data contains
only page/content URLs, not CSS/JS references. No conflict.

### Image optimization interaction

Same as SEO meta tags: JSON-LD image URLs come from frontmatter (not
rendered HTML), so they reference original image paths. Image
optimization only rewrites `<img>` tags. No conflict.

### Dynamic page JSON-LD with collection item data

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
schema:
  type: Article
  author: "{{ post.author_name }}"
  date_published: "{{ post.published_at }}"
  date_modified: "{{ post.updated_at }}"
---
```

The schema field overrides (`author`, `date_published`, `date_modified`)
contain template expressions resolved per-item via
`resolve_schema_expressions`.

### Special characters in JSON-LD values

JSON-LD values are serialized via `serde_json`, which handles JSON
string escaping (double quotes, backslashes, control characters)
automatically. No manual escaping is needed.

### Multiple schema types on one page

A page can request multiple schema types:

```yaml
schema: [Article, BreadcrumbList]
```

Each type generates a separate `<script type="application/ld+json">`
block. This is the Google-recommended approach for multiple schemas.

### BreadcrumbList for root page

For `index.html` (the homepage), the breadcrumb list contains only one
item (Home). A single-item breadcrumb is still valid but not useful, so
BreadcrumbList is skipped for root pages (`/` or `/index.html`).

### BreadcrumbList segment names

By default, breadcrumb segment names are derived from path components:
`/blog/posts/my-article.html` produces segments "blog", "posts",
"my-article". These are raw directory/file names. The user can override
them:

```yaml
schema:
  type: BreadcrumbList
  breadcrumb_names:
    blog: "Blog"
    posts: "All Posts"
```

### WebSite schema on non-homepage

The `WebSite` schema is typically only appropriate on the homepage.
However, we do not enforce this restriction -- the user explicitly opts
in per-page. If they add `schema: WebSite` to a non-homepage, it is
generated. This is valid (if unusual) and not our concern to police.

### Interaction with SEO meta tags

JSON-LD and OG/Twitter meta tags are complementary, not conflicting.
They address different consumers:
- OG/Twitter tags -> social media platform crawlers
- JSON-LD -> search engine crawlers (Google, Bing)

Both can coexist on the same page. The `inject_json_ld` function
receives the already-resolved `SeoMeta` from the pipeline (the same
values used for OG/Twitter tags). This ensures consistency: the title
in `og:title` matches the `headline` in the Article schema.

### Dev server

JSON-LD injection is **disabled during dev server** regardless of config.
The dev server render functions do not call post-render optimization
steps. JSON-LD is only useful in production where pages are crawled by
search engines.

### Existing `<title>` tag

JSON-LD does not modify the `<title>` element. It only injects
`<script>` blocks.

## Configuration Examples

### Minimal (zero config -- no JSON-LD)

With no `[site.schema]` in site.toml and no `schema` in frontmatter:

```toml
[site]
name = "My Site"
base_url = "https://example.com"
```

No JSON-LD is generated on any page. Structured data is opt-in.

### Site-level defaults with BreadcrumbList everywhere

```toml
[site]
name = "My Blog"
base_url = "https://blog.example.com"

[site.schema]
author = "Jane Doe"
default_types = ["BreadcrumbList"]
```

Every page automatically gets a BreadcrumbList schema (except the root
page). Article pages additionally opt in:

```yaml
---
schema: [Article, BreadcrumbList]
seo:
  title: My First Post
  description: An introduction to my blog
---
```

### Dynamic blog post with Article schema

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
schema:
  type: Article
  author: "{{ post.author_name }}"
  date_published: "{{ post.published_at }}"
---
```

### Homepage with WebSite schema

```yaml
---
schema: WebSite
---
```

Generates:
```json
{
  "@context": "https://schema.org",
  "@type": "WebSite",
  "name": "My Blog",
  "url": "https://blog.example.com",
  "description": "A blog about interesting things"
}
```

### Multiple schemas with full overrides

```yaml
---
schema:
  type: [Article, BreadcrumbList]
  author: "John Smith"
  date_published: "2026-03-15"
  breadcrumb_names:
    blog: "Blog"
---
```

## What Is NOT In Scope

1. **Schema.org types beyond Article, BreadcrumbList, WebSite.** Types
   like Product, Recipe, Event, FAQ, HowTo have complex field
   requirements that vary significantly. Each would need its own builder
   function and validation logic. These can be added in future iterations
   by extending the `ResolvedSchema` enum and adding new builder functions.

2. **Schema.org validation.** We do not validate that generated JSON-LD
   conforms to Google's requirements (e.g. that Article has a required
   `image` field for rich results). The user can validate using Google's
   Rich Results Test tool.

3. **Nested schema composition.** E.g. embedding a `Review` inside a
   `Product`. This requires arbitrary nesting and is out of scope for
   the initial implementation.

4. **Microdata or RDFa formats.** Only JSON-LD is supported. JSON-LD is
   the format recommended by Google and the most widely supported.

5. **`@graph` arrays.** Multiple schemas are injected as separate
   `<script>` blocks, not as a single `@graph` array. Both approaches
   are valid; separate blocks are simpler to generate and debug.

6. **SearchAction for WebSite.** The `potentialAction` field (for
   sitelinks search box) requires a search URL template, which is
   site-specific and not derivable from config. Users who need this
   can provide their own JSON-LD block.

7. **Organization schema.** While useful, it requires fields like
   `logo`, `contactPoint`, `sameAs` (social profiles) that go beyond
   what we can auto-derive. Can be added later.

8. **Per-field opt-out.** No mechanism to say "generate Article but
   omit the author field." Fields are included when values are available,
   omitted when they are not.

## Module Structure

```
src/build/
  json_ld.rs        -- public API (inject_json_ld, resolve_schema_expressions),
                       schema builders, JSON-LD detection, HTML injection
```

A single file because the feature is focused:

- `has_existing_json_ld`: lol_html scan (~15 lines)
- `resolve_schema_types`: type list merging (~20 lines)
- `build_article_schema`: JSON object construction (~40 lines)
- `build_breadcrumb_schema`: path segment parsing + JSON (~40 lines)
- `build_website_schema`: JSON object construction (~15 lines)
- `inject_into_head`: lol_html injection (~20 lines)
- `inject_json_ld`: orchestration (~30 lines)
- `resolve_schema_expressions`: minijinja rendering (~40 lines)
- Tests (~300 lines)

Total: ~520 lines. Reasonable for a single file.

This follows the pattern of `build/seo.rs` (single-file module for a
focused build step).

## Integration Points

### config/mod.rs

Add `SiteSchemaConfig` struct and the `schema` field to `SiteMeta`.

Test helpers that construct `SiteMeta` directly must be updated to
include the new `schema` field. Since `SiteSchemaConfig` derives
`Default`, these helpers can use `schema: SiteSchemaConfig::default()`.

### frontmatter/mod.rs

Add `SchemaConfigValue`, `SchemaFullConfig`, `SchemaTypes`, and the
`SchemaConfig` type alias. Add the `schema` field to both `Frontmatter`
and `RawFrontmatter`. Update `parse_frontmatter` to pass through the
parsed schema data. Update `Frontmatter::default()` to include
`schema: None` (since `SchemaConfig` is `Option<SchemaConfigValue>`).

### build/mod.rs

Add `pub mod json_ld;` to the module declarations.

### build/render.rs

In `render_static_page` and `render_dynamic_page`, add the JSON-LD step
after SEO meta tag injection and before minification. Two integration
points per function:

1. **Early:** Call `resolve_schema_expressions` after
   `seo::resolve_seo_expressions` (both resolve template expressions in
   the same context).

2. **Late:** Call `inject_json_ld` in the pipeline after
   `seo::inject_seo_tags` and before `minify::minify_html`.

## Test Plan

Tests follow the project convention: test behavior, not library
internals. Do not use `unwrap`/`expect` unless it is an invariant.

### Unit tests in `json_ld.rs`

| Test | What it verifies |
|---|---|
| `test_has_existing_json_ld_present` | Detects `<script type="application/ld+json">` in HTML |
| `test_has_existing_json_ld_absent` | Returns false for HTML without JSON-LD |
| `test_has_existing_json_ld_other_script` | Does not false-positive on regular `<script>` tags |
| `test_resolve_schema_types_from_frontmatter_string` | Single string type name |
| `test_resolve_schema_types_from_frontmatter_list` | List of type names |
| `test_resolve_schema_types_from_frontmatter_full` | Full config with types |
| `test_resolve_schema_types_site_defaults` | Site default_types used when no frontmatter |
| `test_resolve_schema_types_none` | Empty result when no config at any level |
| `test_build_article_schema_full` | All fields populated correctly |
| `test_build_article_schema_minimal` | Only headline, url, publisher (no description/image/dates) |
| `test_build_article_schema_auto_populates_from_seo` | Title/description/image from SeoMeta |
| `test_build_breadcrumb_schema` | Correct items for multi-segment path |
| `test_build_breadcrumb_schema_root_page` | Returns None/empty for root page |
| `test_build_breadcrumb_schema_custom_names` | Segment name overrides applied |
| `test_build_website_schema` | All fields populated |
| `test_build_website_schema_no_description` | Description omitted when absent |
| `test_inject_json_ld_basic` | JSON-LD script block appears in `<head>` |
| `test_inject_json_ld_no_head` | HTML without `<head>` returns unchanged |
| `test_inject_json_ld_existing_json_ld` | Existing JSON-LD, returns HTML unchanged |
| `test_inject_json_ld_multiple_types` | Multiple script blocks generated |
| `test_inject_json_ld_no_schema` | No schema config, no injection |
| `test_inject_json_ld_unrecognized_type` | Warning logged, skipped |
| `test_resolve_schema_expressions_basic` | Template expression resolved in author field |
| `test_resolve_schema_expressions_no_expressions` | Literal values pass through unchanged |
| `test_resolve_schema_expressions_missing_var` | Missing variable, field becomes None |

### Configuration tests in `config/mod.rs`

| Test | What it verifies |
|---|---|
| `test_site_schema_config_defaults` | Empty config gives empty default_types and None author |
| `test_site_schema_config_custom` | All fields parse correctly from TOML |

### Frontmatter tests in `frontmatter/mod.rs`

| Test | What it verifies |
|---|---|
| `test_parse_schema_frontmatter_string` | `schema: Article` parsed as Some(TypeName) |
| `test_parse_schema_frontmatter_list` | `schema: [Article, BreadcrumbList]` parsed as Some(TypeList) |
| `test_parse_schema_frontmatter_full` | Full config with type and overrides |
| `test_parse_schema_frontmatter_absent` | No `schema:` section gives None |
| `test_parse_schema_with_template_expressions` | Expressions stored as literal strings |

### What we do NOT test

- `serde_json`'s serialization correctness.
- `lol_html`'s attribute/element parsing.
- `minijinja`'s `render_str` correctness.
- That Google/Bing correctly parse the generated JSON-LD.
- That generated JSON-LD passes Google's Rich Results Test.
