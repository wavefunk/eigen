# JSON-LD Structured Data

## Overview

Eigen auto-injects JSON-LD structured data (`<script type="application/ld+json">`)
into the `<head>` of rendered pages during the build pipeline. This enables search
engines (Google, Bing) to understand page content and generate rich results.

JSON-LD is complementary to the OG/Twitter meta tags injected by `build::seo`.
While OG/Twitter tags target social media crawlers, JSON-LD targets search engine
crawlers.

## Supported Schema Types

Three schema.org types are supported out of the box:

### Article

For blog posts, news articles, and similar content pages.

Fields (auto-populated from SEO config and frontmatter):
- `headline`: from `seo.title` > `site.seo.title` > `site.name`
- `url`: canonical URL (same logic as `build::seo`)
- `description`: from `seo.description` > `site.seo.description` (optional)
- `image`: from `seo.image` > `site.seo.image`, resolved to absolute URL (optional)
- `datePublished`: from `schema.date_published` in frontmatter (optional)
- `dateModified`: from `schema.date_modified`, falls back to `datePublished` (optional)
- `author`: from `schema.author` or `site.schema.author` (optional)
- `publisher`: always `{ "@type": "Organization", "name": site.name }`

### BreadcrumbList

Generated from the page URL path. For `/blog/posts/my-article.html`, produces
breadcrumb items: Home > blog > posts > my-article.

Skipped for root pages (`/` or `/index.html`) since single-item breadcrumbs
are not useful.

Segment names can be overridden via `breadcrumb_names` in frontmatter.

### WebSite

For the homepage. Enables sitelinks in search results.

Fields:
- `name`: `site.name`
- `url`: `site.base_url`
- `description`: `site.seo.description` (optional)

## Configuration

### Site-Level Defaults (site.toml)

```toml
[site.schema]
author = "Jane Doe"              # Default author for Article schemas
default_types = ["BreadcrumbList"] # Schema types applied to all pages
```

When `default_types` is set, those schemas are generated for every page unless
the page's frontmatter overrides them. Pages can still add additional types.

### Frontmatter

The `schema` field in frontmatter supports three formats:

**String (single type):**
```yaml
schema: Article
```

**List (multiple types):**
```yaml
schema:
  - Article
  - BreadcrumbList
```

**Full config (type with overrides):**
```yaml
schema:
  type: Article
  author: "Jane Doe"
  date_published: "2026-03-15"
  date_modified: "2026-03-17"
  breadcrumb_names:
    blog: "Blog Posts"
```

### Dynamic Pages

For dynamic pages, schema fields can contain minijinja template expressions:

```yaml
schema:
  type: Article
  author: "{{ post.author_name }}"
  date_published: "{{ post.published_at }}"
  date_modified: "{{ post.updated_at }}"
```

Expressions are resolved per-item via `resolve_schema_expressions`, which runs
alongside `seo::resolve_seo_expressions` in the render flow.

## Opt-In Behavior

JSON-LD is **opt-in** -- no structured data is generated unless configured.
This differs from OG/Twitter tags (which are universally useful). Structured
data is type-specific: an Article schema on a 404 page would be incorrect.

The three ways to enable JSON-LD:
1. Set `default_types` in `[site.schema]` for site-wide defaults.
2. Add `schema` to individual page frontmatter.
3. Both (page frontmatter overrides site defaults).

## Existing JSON-LD Detection

If the rendered HTML already contains a `<script type="application/ld+json">`
block (e.g. from the template itself), the injection step skips entirely.
This respects author-provided structured data.

## Pipeline Position

```
  -> SEO meta tag injection       (build::seo)
  -> JSON-LD injection            (build::json_ld)  <-- this feature
  -> minify HTML                  (build::minify)
  -> write to disk
```

JSON-LD runs after SEO tags (both inject into `<head>`) and before minification
(so the injected script benefits from `minify-html`).

Fragments do NOT get JSON-LD -- they are partial HTML snippets loaded via HTMX
and have no `<head>`.

## Error Handling

JSON-LD is an enhancement. Failures never break the build:
- Unrecognized schema type: warning logged, type skipped.
- Template expression failure: field treated as absent, warning logged.
- lol_html injection error: original HTML returned unchanged, warning logged.
- No `<head>` element: no injection (lol_html selector does not match).

## Module Structure

Single file: `src/build/json_ld.rs`

Public API:
- `inject_json_ld()`: main pipeline step
- `resolve_schema_expressions()`: template expression evaluation

## Examples

### Minimal (no JSON-LD)

No `[site.schema]` in site.toml and no `schema` in frontmatter. No JSON-LD
is generated on any page.

### Blog post with Article schema

```yaml
---
seo:
  title: "My First Post"
  description: "An introduction to my blog"
  image: "/assets/hero.jpg"
schema:
  type: Article
  author: "Jane Doe"
  date_published: "2026-03-15"
---
```

### Homepage with WebSite schema

```yaml
---
schema: WebSite
---
```

### BreadcrumbList everywhere via site defaults

```toml
[site.schema]
default_types = ["BreadcrumbList"]
```
