# Atom Feed Generation

Eigen can auto-generate Atom 1.0 (RFC 4287) XML feeds for configured
data collections. Feeds are written to `dist/` at configurable paths
during the build process.

## Configuration

Add `[feed.<name>]` tables to `site.toml`. Each feed defines its data
source (same fields as a frontmatter `DataQuery`) plus metadata and
field mappings.

### File-based example

```toml
[feed.blog]
file = "posts.json"        # file in _data/
sort = "-date"              # sort descending by date
limit = 20                  # max 20 entries
path = "blog/feed.xml"      # output path in dist/
link_prefix = "blog"        # entry URLs: {base_url}/blog/{slug}.html
```

### Remote source example

```toml
[sources.cms]
url = "https://cms.example.com/api"

[feed.blog]
source = "cms"              # source from [sources.*]
query_path = "/posts"       # appended to source URL
root = "data.posts"         # dot-path into response JSON
sort = "-publishedAt"
title = "My Blog Feed"
author = "Jane Doe"
title_field = "title"
date_field = "publishedAt"
summary_field = "excerpt"
slug_field = "slug"
link_prefix = "blog"
```

### Multiple feeds

```toml
[feed.blog]
file = "posts.json"
path = "blog/feed.xml"
link_prefix = "blog"

[feed.changelog]
file = "releases.json"
path = "changelog/feed.xml"
title_field = "version"
link_prefix = "changelog"
```

## Configuration Reference

| Field | Type | Default | Description |
|---|---|---|---|
| `file` | string | - | Local file in `_data/`. Required if `source` not set. |
| `source` | string | - | Source name from `[sources.*]`. Required if `file` not set. |
| `query_path` | string | - | URL path appended to source's base URL. |
| `root` | string | - | Dot-path into response JSON for array extraction. |
| `sort` | string | - | Sort spec: `"field"` ascending, `"-field"` descending. |
| `title` | string | `site.name` | Feed title. |
| `path` | string | `"feed.xml"` | Output path relative to `dist/`. |
| `author` | string | `site.schema.author` | Feed author name. |
| `limit` | usize | `50` | Maximum number of entries. |
| `title_field` | string | `"title"` | Field on each item for entry title. |
| `date_field` | string | `"date"` | Field on each item for entry date. |
| `summary_field` | string | - | Field on each item for entry summary. Omitted if unset. |
| `slug_field` | string | `"slug"` | Field on each item for URL slug. |
| `link_prefix` | string | - | URL prefix for entry links. |

## Validation

- Each feed must have either `file` or `source`.
- If `source` is set, it must exist in `[sources.*]`.
- `path` must not be empty.
- `limit` must be > 0.
- `slug_field` must not be empty.

## Entry Link Construction

Entry URLs are built as: `{base_url}/{link_prefix}/{slug}.html`

If `link_prefix` is not set: `{base_url}/{slug}.html`

## Date Handling

Supported input formats for the date field:
- RFC 3339: `2026-03-17T12:00:00Z`
- Date only: `2026-03-17` (midnight UTC assumed)
- With timezone: `2026-03-17T12:00:00+05:30`

Dates are output as RFC 3339 in the Atom `<updated>` element.
If a date cannot be parsed, the build timestamp is used as fallback.

## Error Handling

- Missing required fields (title, slug) on an entry: entry is skipped
  with a warning, not a build failure.
- Invalid feed config: build error via validation.
- Empty collections: valid feed with zero entries.

## Architecture

- Module: `src/build/feed.rs`
- Pipeline position: after sitemap generation, before post-build hooks
- Reuses `DataFetcher` (cache-warm from page rendering) and
  `escape_xml` from `src/build/sitemap.rs`
- No external XML library; string-based XML construction
- No new crate dependencies (uses existing chrono, serde, eyre)

## Autodiscovery

Autodiscovery `<link>` tags are not injected automatically. Add this
to your base template's `<head>` manually:

```html
<link rel="alternate" type="application/atom+xml"
      title="My Blog Feed" href="/blog/feed.xml" />
```
