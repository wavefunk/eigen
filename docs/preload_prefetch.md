# Preload / Prefetch Resource Hints

## Overview

Eigen automatically injects `<link>` resource hints into `<head>` to improve page load performance:

- **`<link rel="preload">`** for hero images -- tells the browser to start fetching the LCP image immediately, before the HTML parser reaches the `<img>` tag. Reduces Largest Contentful Paint.
- **`<link rel="prefetch">`** for navigation links -- tells the browser to fetch HTMX fragment files at idle time, making partial-page transitions feel instant.

Resource hints are enabled by default. They are purely additive (they add `<link>` tags but never modify or remove existing content) and cannot break output.

## Configuration

### site.toml

```toml
[build.hints]
# Master switch. Default: true.
enabled = true

# Auto-detect hero image from first qualifying <img>. Default: true.
auto_detect_hero = true

# Generate prefetch hints for navigation links. Default: true.
prefetch_links = true

# Maximum prefetch hints per page. Default: 5.
max_prefetch = 5

# Fallback imagesizes for hero preload. Default: "100vw".
hero_image_sizes = "100vw"

# Glob patterns to exclude from prefetching.
exclude_prefetch = ["**/archive/**"]
```

### Frontmatter

Designate a specific hero image in template frontmatter:

```yaml
---
hero_image: /assets/hero-banner.jpg
data:
  nav:
    file: "nav.yaml"
---
```

When `hero_image` is set, it takes priority over auto-detection.

## Hero Image Preload

### Resolution order

1. Frontmatter `hero_image` field (highest priority).
2. Auto-detected first `<img>` without `loading="lazy"` (if `auto_detect_hero` is enabled).
3. No preload hint generated.

### Responsive variants

When image optimization is active and the hero image has a `<picture>` element with `<source srcset>`, the preload uses `imagesrcset` and `imagesizes` to let the browser preload the correct size:

```html
<link rel="preload" as="image" href="/assets/hero.jpg"
      imagesrcset="/assets/hero-480w-abc.avif 480w, /assets/hero-768w-abc.avif 768w"
      imagesizes="100vw"
      type="image/avif">
```

The preload targets the best format (AVIF if available). Browsers that don't support AVIF ignore the hint safely.

### CSS background images

If the hero is a CSS `background-image`, auto-detection won't find it. Use the frontmatter `hero_image` field. The module will scan `dist/` for variant files as a fallback.

## Navigation Prefetch

### How it works

- When fragments are enabled: scans `hx-get` attributes to prefetch fragment files.
- When fragments are disabled: scans `<a href>` attributes to prefetch full pages (adds `as="document"`).

### Filtering

- External URLs (http/https) are excluded.
- Self-referencing links (current page) are excluded.
- Anchor-only links (#...) are excluded.
- Links with `target="_blank"` are excluded (href mode only).
- URLs matching `exclude_prefetch` glob patterns are excluded.
- Duplicate URLs are deduplicated (first occurrence wins).
- At most `max_prefetch` URLs are included (document order).

## Pipeline position

```
render template
  -> strip fragment markers
  -> localize assets
  -> optimize images (img -> picture)
  -> rewrite CSS background images
  -> plugin post_render_html
  -> critical CSS inlining
  -> preload/prefetch hints  <-- THIS FEATURE
  -> minify HTML
  -> write to disk
```

Fragments do NOT get resource hints (they lack a `<head>` element).
Dev server builds also skip resource hints.

## Error handling

All errors are non-fatal. If anything goes wrong (missing `<head>`, IO errors, parse failures), the original HTML is returned unchanged with a warning logged.
