# RSS/Atom Feed Generation -- Design Spec

Date: 2026-03-17

## Motivation and Goals

Static sites with dynamic collections (blog posts, podcasts, changelogs)
need syndication feeds so that readers can subscribe via RSS readers and
aggregators. Without feed generation, site authors must either manually
maintain an XML file or use a third-party service.

Eigen already has the machinery to discover dynamic collections (via
frontmatter `collection` queries), fetch and sort their items, and
render per-collection pages. Generating an Atom feed is a natural
extension: at build time, after rendering all pages, emit `feed.xml`
files for configured collections.

**Goals:**

- Generate valid Atom 1.0 XML (RFC 4287) for configured collections.
- Support per-collection feeds at configurable output paths.
- Integrate into the existing build pipeline with zero disruption to
  non-feed sites.
- Require minimal configuration: a `[feed]` table in `site.toml`.
- Reuse existing data-fetching infrastructure (`DataFetcher`, source
  configs, transforms).

**Non-goals:**

- RSS 2.0 output. Atom is the modern standard, is a strict superset in
  capability, and is universally supported. Supporting both formats adds
  complexity without value.
- Full-content feed entries. Entries include title, link, updated date,
  and an optional summary. Embedding full rendered HTML in `<content>`
  would require rendering each collection item's template a second time
  (once for the page, once for the feed) and is out of scope for the
  initial implementation.
- Feed pagination (RFC 5005). Static site feeds are typically small
  enough (< 100 entries) that pagination is unnecessary.
- Autodiscovery `<link>` injection into HTML `<head>`. This would
  require a new lol_html rewriting pass. Authors can add the tag
  manually in their base template. Can be added as a follow-up.

## Atom Feed Format

The generated feed conforms to Atom 1.0 (RFC 4287). Example output:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>My Blog</title>
  <link href="https://example.com/"/>
  <link href="https://example.com/blog/feed.xml" rel="self"/>
  <id>https://example.com/blog/feed.xml</id>
  <updated>2026-03-17T12:00:00Z</updated>
  <author>
    <name>Jane Doe</name>
  </author>
  <entry>
    <title>Hello World</title>
    <link href="https://example.com/blog/hello-world.html"/>
    <id>https://example.com/blog/hello-world.html</id>
    <updated>2026-03-17T12:00:00Z</updated>
    <summary>My first blog post about...</summary>
  </entry>
</feed>
```

### Required Elements

Per RFC 4287, a valid feed must contain:

- `<title>`: from feed config or site name.
- `<link>`: the HTML alternate link (site base URL or collection URL).
- `<link rel="self">`: the feed's own URL.
- `<id>`: a permanent, unique identifier (we use the self link URL).
- `<updated>`: the most recent update timestamp.

Per entry:

- `<title>`: from a configurable field on the collection item.
- `<link>`: the full URL to the rendered page.
- `<id>`: same as `<link>` (URL serves as permanent ID).
- `<updated>`: from a configurable field on the collection item, or
  falls back to the build timestamp.

### Optional Elements

- `<author><name>`: from feed config or `site.schema.author`.
- `<summary>`: from a configurable field on the collection item.

## Configuration

### site.toml Schema

A new top-level `[feed]` table with a map of named feed definitions.
Each feed specifies its data source inline (same fields as a frontmatter
`DataQuery`) plus feed-specific metadata and field mappings.

**Remote source example:**

```toml
[feed.blog]
source = "blog_api"            # Required (or `file`): source from [sources.*]
query_path = "/posts"          # Optional: URL path appended to source base URL
root = "data.posts"            # Optional: dot-path into response JSON
sort = "-publishedAt"          # Optional: sort spec
title = "My Blog Feed"         # Optional: feed title, defaults to site.name
path = "blog/feed.xml"         # Optional: output path in dist/, defaults to "feed.xml"
author = "Jane Doe"            # Optional: defaults to site.schema.author
limit = 20                     # Optional: max entries, defaults to 50
title_field = "title"          # Optional: field on each item for entry title, defaults to "title"
date_field = "publishedAt"     # Optional: field for entry updated date, defaults to "date"
summary_field = "excerpt"      # Optional: field for entry summary, omitted if unset
link_prefix = "blog"           # Optional: URL prefix for entry links
slug_field = "slug"            # Optional: field for URL slug, defaults to "slug"
```

**File-based data example:**

```toml
[feed.blog]
file = "posts.json"            # Required (or `source`): file in _data/
sort = "-date"
limit = 20
path = "blog/feed.xml"
link_prefix = "blog"
slug_field = "slug"
```

**Multiple feeds:**

```toml
[feed.blog]
file = "posts.json"
path = "blog/feed.xml"
link_prefix = "blog"

[feed.changelog]
source = "cms"
query_path = "/releases"
path = "changelog/feed.xml"
title_field = "version"
link_prefix = "changelog"
```

### Data Source for Feed Entries

The feed config specifies its data source using the same fields as a
frontmatter `DataQuery`: `file`, `source`, `query_path` (mapped to
`DataQuery.path`), `root`, and `sort`. The feed generator constructs
a `DataQuery` from these fields and fetches via the existing
`DataFetcher`, reusing source configs, caching, and transform
infrastructure.

The `query_path` field is named differently from `DataQuery.path` to
avoid collision with the feed's output `path` field.

### Design Decision: Inline Data Query vs. Referencing Dynamic Pages

**Option A: Reference a dynamic page template.**
The feed would point to a dynamic template like `posts/[post].html` and
inherit its collection query. This is elegant but creates a tight
coupling between feed generation and template discovery. It also fails
for feeds that do not correspond to a dynamic page (e.g., a feed of
all pages, or a feed sourced from a different API endpoint than any
template uses).

**Option B: Inline data query in feed config.** (Chosen)
The feed config specifies its own data query fields (`source`,
`query_path`, `file`, `root`, `sort`, `limit`). This reuses the
`DataQuery` struct and `DataFetcher` infrastructure but keeps feed
generation independent of template discovery. It is more explicit and
flexible.

### Config Struct

```rust
/// Top-level feed configuration from `[feed]` in site.toml.
/// Maps feed name -> feed definition.
pub type FeedConfigs = HashMap<String, FeedConfig>;

/// Configuration for a single Atom feed.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedConfig {
    // -- Data source (inline DataQuery fields) --

    /// Local file in `_data/`, e.g. `"posts.json"`.
    pub file: Option<String>,
    /// Source name from `[sources.*]`.
    pub source: Option<String>,
    /// URL path appended to the source's base URL.
    pub query_path: Option<String>,
    /// Dot-path into the response to extract data.
    pub root: Option<String>,
    /// Sort spec: `"field"` or `"-field"`.
    pub sort: Option<String>,

    // -- Feed metadata --

    /// Feed title. Defaults to `site.name`.
    pub title: Option<String>,
    /// Output path relative to `dist/`. Defaults to `"feed.xml"`.
    #[serde(default = "default_feed_path")]
    pub path: String,
    /// Feed author name. Defaults to `site.schema.author`.
    pub author: Option<String>,
    /// Maximum number of entries. Defaults to 50.
    #[serde(default = "default_feed_limit")]
    pub limit: usize,

    // -- Entry field mapping --

    /// Field name on each item for the entry title. Defaults to `"title"`.
    #[serde(default = "default_title_field")]
    pub title_field: String,
    /// Field name on each item for the entry date. Defaults to `"date"`.
    #[serde(default = "default_date_field")]
    pub date_field: String,
    /// Field name on each item for an entry summary. No default (omitted if absent).
    pub summary_field: Option<String>,
    /// Field name for the URL slug. Defaults to `"slug"`.
    #[serde(default = "default_slug_field")]
    pub slug_field: String,
    /// URL path prefix for entry links. E.g., `"blog"` produces
    /// links like `{base_url}/blog/{slug}.html`.
    pub link_prefix: Option<String>,
}
```

### Validation

A new `validate_feed_configs` function is called from the existing
`validate_config`:

1. Each feed must have at least one of `file` or `source`. If neither
   is set, bail with a clear error naming the feed.
2. If `source` is set, it must exist in `config.sources`.
3. `path` must not be empty. (`.xml` extension is recommended but not
   enforced -- some sites use `atom.xml` or `index.xml`.)
4. `limit` must be > 0.
5. `slug_field` must not be empty.

## Architecture

### New Module: `src/build/feed.rs`

A single new module, following the pattern of `src/build/sitemap.rs`.

**Public API:**

```rust
/// Generate Atom feeds for all configured feed definitions.
pub fn generate_feeds(
    dist_dir: &Path,
    config: &SiteConfig,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
    build_time: &str,
) -> Result<usize>
```

Returns the number of feeds generated (for logging).

**Internal functions:**

```rust
/// Generate a single Atom feed.
fn generate_feed(
    dist_dir: &Path,
    feed_name: &str,
    feed_config: &FeedConfig,
    site: &SiteMeta,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
    build_time: &str,
) -> Result<()>

/// Build a DataQuery from the feed config's inline fields.
fn build_feed_query(config: &FeedConfig) -> DataQuery

/// Render a single Atom entry element.
fn render_entry(
    item: &serde_json::Value,
    feed_config: &FeedConfig,
    base_url: &str,
    build_time: &str,
) -> Option<String>

/// Format a date value into RFC 3339 for Atom `<updated>`.
fn format_atom_date(value: &serde_json::Value, build_time: &str) -> String
```

### Pipeline Position

Feed generation runs after all pages are rendered and after sitemap
generation, but before content-hash rewriting (feeds are not
content-hashed -- they must have stable URLs for subscribers):

```
render all pages
  -> generate sitemap        (build::sitemap)
  -> generate feeds          (build::feed)     <-- NEW
  -> post-build plugin hooks
  -> CSS/JS bundling
  -> content hash rewrite
```

This position is chosen because:

1. Feed generation does not depend on rendered HTML output. It fetches
   collection data independently and constructs XML from raw data
   fields.
2. Running after sitemap keeps the "generate auxiliary files" steps
   grouped together.
3. Running before content-hash rewriting ensures feed XML files are not
   accidentally rewritten (they reference page URLs, not asset paths).
4. The `DataFetcher` cache is warm from page rendering, so collection
   data fetches for feeds will be cache hits (assuming the same sources
   are used).

### Integration into `build()` in `src/build/render.rs`

After the sitemap generation call (line 198-199), before the
post-build plugin hooks (line 201), add:

```rust
// Generate Atom feeds.
if !config.feed.is_empty() {
    let feed_count = feed::generate_feeds(
        &dist_dir, &config, &mut fetcher,
        Some(&plugin_registry), &build_time,
    )?;
    tracing::info!("Generating {} feed(s)... done", feed_count);
}
```

### XML Generation

Following the pattern in `sitemap.rs`, XML is generated via string
building (no XML library dependency). The output is small and
well-structured enough that manual string construction is preferable to
adding a dependency.

The `escape_xml` function from `sitemap.rs` is reused. Since it is
currently private to `sitemap.rs`, it will be made `pub(crate)` or
extracted to a shared utility.

### Entry Link Construction

Each entry's `<link>` URL is built from:

```
{base_url}/{link_prefix}/{slug}.html
```

Where:
- `base_url` comes from `site.base_url` (trailing slash stripped).
- `link_prefix` comes from `feed_config.link_prefix` (optional).
- `slug` comes from `item[slug_field]`.

If `link_prefix` is not set, the entry URL is `{base_url}/{slug}.html`.

### Date Handling

The `date_field` on each item should contain an ISO 8601 date string.
The feed generator attempts to parse and re-format it as RFC 3339 (the
Atom standard). If the value is not a valid date, or the field is
missing, the build timestamp is used as a fallback.

Supported input formats:
- Full RFC 3339: `2026-03-17T12:00:00Z`
- Date only: `2026-03-17` (midnight UTC is assumed)
- Date with timezone: `2026-03-17T12:00:00+05:30`

The `chrono` crate (already a dependency) handles parsing.

### Error Handling

- Missing required fields on items (title, slug) cause the entry to be
  skipped with a `tracing::warn!`, not a build failure. This matches
  how `render_dynamic_page` handles missing slugs.
- Invalid feed config (e.g., missing source) causes a build error via
  `eyre::bail!`.
- Empty collections produce a valid feed with zero entries.

## Impact on Existing Code

### Files Modified

| File | Change |
|---|---|
| `src/config/mod.rs` | Add `FeedConfig`, `FeedConfigs`, field on `SiteConfig`, defaults, validation |
| `src/build/mod.rs` | Add `pub mod feed;` |
| `src/build/render.rs` | Call `feed::generate_feeds` after sitemap |
| `src/build/sitemap.rs` | Make `escape_xml` `pub(crate)` (line 63) |

### Files Created

| File | Description |
|---|---|
| `src/build/feed.rs` | Atom feed generation module |

### Test Helper Updates

Adding a `feed` field to `SiteConfig` requires updating every
`test_config()` helper that constructs `SiteConfig` manually. These
helpers exist in:

- `src/build/sitemap.rs` (line 79)
- `src/build/context.rs` (line 90)
- `src/discovery/mod.rs` (line 210)
- `src/build/render.rs` (multiple test helpers, if any)

Each needs `feed: HashMap::new()` added. This is mechanical.

### Files NOT Modified

- `src/frontmatter/mod.rs` -- no changes needed. Feeds are configured
  in `site.toml`, not in template frontmatter.
- `src/discovery/mod.rs` -- feed generation does not depend on template
  discovery. (Only its test helper needs updating for the new
  `SiteConfig` field.)
- `src/data/` -- reused as-is via `DataFetcher`.
- `src/template/` -- feeds are XML, not rendered via minijinja.

## Test Plan

### Unit Tests (in `src/build/feed.rs`)

| Test | What it verifies |
|---|---|
| `test_generate_feed_basic` | Valid Atom XML with title, links, entries |
| `test_generate_feed_empty_collection` | Valid feed with zero entries |
| `test_generate_feed_entry_fields` | Entry title, link, id, updated, summary |
| `test_generate_feed_missing_title` | Entry skipped when title field missing |
| `test_generate_feed_missing_slug` | Entry skipped when slug field missing |
| `test_generate_feed_date_formats` | ISO 8601 date parsing variants |
| `test_generate_feed_date_fallback` | Build time used when date missing |
| `test_generate_feed_limit` | Only N entries included |
| `test_generate_feed_xml_escaping` | Special chars in title/summary escaped |
| `test_generate_feed_custom_fields` | Custom title_field, date_field, etc. |
| `test_generate_feed_link_prefix` | Entry URLs include prefix |
| `test_generate_feed_no_prefix` | Entry URLs without prefix |
| `test_generate_feed_author` | Author element from config |
| `test_generate_feed_author_fallback` | Author from site.schema.author |
| `test_build_feed_query` | FeedConfig -> DataQuery conversion |

### Config Tests (in `src/config/mod.rs`)

| Test | What it verifies |
|---|---|
| `test_feed_config_defaults` | Default values when `[feed]` is absent |
| `test_feed_config_parsing` | Full feed config parsed from TOML |
| `test_feed_config_multiple` | Multiple feeds parsed |
| `test_feed_config_validation_no_source` | Error when neither file nor source set |
| `test_feed_config_validation_bad_source` | Error when source not in `[sources]` |

## What Is NOT In Scope

1. **RSS 2.0 output.** Atom is universally supported and strictly
   more capable. Adding RSS 2.0 would double the XML generation code
   for no practical benefit.

2. **Autodiscovery `<link>` tag injection.** Injecting
   `<link rel="alternate" type="application/atom+xml" ...>` into
   `<head>` would require a lol_html rewriting pass. Authors can add
   this manually in their base template. This can be added as a
   follow-up feature.

3. **Full HTML content in entries.** Including rendered HTML in
   `<content type="html">` would require re-rendering each collection
   item's template or reading the rendered HTML from disk. This is
   complex and can be added later.

4. **Feed validation.** The generated XML is structurally correct by
   construction. External validation (e.g., W3C Feed Validator) is
   left to the site author.

5. **Content hash exclusion.** Feed files must have stable URLs for
   subscribers. The content-hash module already only processes files
   in `static/` and bundled files, so feed XML in `dist/` is
   naturally excluded. No special handling needed.

## Performance Considerations

- Feed data fetches should be cache hits in `DataFetcher` if the same
  source/path was already fetched during page rendering.
- XML string building is O(n) in the number of entries, with minimal
  allocations (pre-sized `String` buffer).
- The `limit` config caps entry count, preventing unbounded feed sizes.
- No new external dependencies required.
