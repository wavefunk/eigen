# Open Graph / Twitter Card Meta Tags

## Overview

Eigen auto-injects Open Graph (`og:*`) and Twitter Card (`twitter:*`)
meta tags into every page's `<head>` during the build pipeline. This
ensures shared links on social media (Twitter/X, Facebook, LinkedIn,
Slack, Discord, etc.) display rich preview cards with title,
description, and image.

No template changes are required. The feature is always-on.

## How It Works

1. SEO values are resolved from a three-layer cascade:
   - Auto-derived defaults (title = site.name, og:type = "website", canonical URL = base_url + page path)
   - Site-level defaults from `[site.seo]` in `site.toml`
   - Per-page overrides from `[seo]` in template frontmatter

2. The rendered HTML is scanned for existing OG/Twitter meta tags.
3. Only missing tags are generated and injected into `<head>`.

## Configuration

### Site-level defaults (`site.toml`)

```toml
[site]
name = "My Blog"
base_url = "https://blog.example.com"

[site.seo]
title = "My Blog"                          # optional, falls back to site.name
description = "A blog about interesting things"  # optional
image = "/assets/default-share.jpg"        # optional, site-relative or absolute URL
og_type = "website"                        # default: "website"
twitter_site = "@myblog"                   # optional, Twitter @handle
twitter_card = "summary_large_image"       # default: "summary_large_image"
```

All fields are optional. If `[site.seo]` is omitted entirely, sensible
defaults are used.

### Per-page frontmatter

```yaml
---
seo:
  title: About Us
  description: Learn about our team and mission
  image: /assets/about-hero.jpg
  og_type: website
  twitter_card: summary_large_image
  canonical_url: https://example.com/about
---
```

All fields are optional. When absent, site-level defaults are used.

### Dynamic pages with template expressions

For dynamic pages, SEO field values can reference collection item data
using minijinja template expressions:

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

Expressions are resolved per-item using the same template context
used for the page template.

## Generated Tags

For a fully configured page, these tags are generated:

```html
<!-- Open Graph -->
<meta property="og:title" content="Page Title">
<meta property="og:description" content="Page description">
<meta property="og:image" content="https://example.com/assets/share.jpg">
<meta property="og:url" content="https://example.com/about.html">
<meta property="og:type" content="website">
<meta property="og:site_name" content="My Site">

<!-- Twitter Card -->
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="Page Title">
<meta name="twitter:description" content="Page description">
<meta name="twitter:image" content="https://example.com/assets/share.jpg">
<meta name="twitter:site" content="@mysite">

<!-- Canonical URL -->
<link rel="canonical" href="https://example.com/about.html">
```

Tags are omitted when their value is not available:
- No description at any level: `og:description` and `twitter:description` omitted
- No image at any level: `og:image`, `twitter:image` omitted, `twitter:card` forced to `"summary"`
- No `twitter_site` configured: `twitter:site` omitted

## Duplicate Detection

If a template already contains OG/Twitter meta tags or a canonical
link, the build pipeline detects them and does not inject duplicates.
This allows template authors to manually control specific tags when
needed.

## URL Resolution

- Image paths starting with `/` are resolved to absolute URLs using `site.base_url`
- Already-absolute URLs (`http://` or `https://`) are used as-is
- Canonical URLs are auto-generated from `site.base_url + page_path`
- `/index.html` paths are normalized to `/` in canonical URLs

## Pipeline Position

SEO injection runs after preload/prefetch hints and before HTML
minification. It does not run during dev server rendering.

## Module

`src/build/seo.rs` -- single file containing all resolution, detection,
generation, and injection logic.
