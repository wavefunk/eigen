# Critical CSS Inlining

## What it does

Critical CSS inlining extracts the subset of CSS rules actually used on each page, inlines them in a `<style>` block in `<head>`, and defers the full stylesheet load. This eliminates render-blocking CSS, improving First Contentful Paint (FCP) and Largest Contentful Paint (LCP).

## How it works

1. After rendering, eigen finds all `<link rel="stylesheet">` tags pointing to local CSS files.
2. It parses each stylesheet with lightningcss and the rendered HTML with scraper.
3. For each CSS rule, it tests whether its selector matches any element in the HTML.
4. Matching rules (plus transitively referenced @font-face, @keyframes, and custom properties) are serialized into a `<style>` block.
5. The original `<link>` tags are rewritten to load asynchronously using the preload pattern.

## Pipeline position

```
render template -> strip fragments -> localize assets -> optimize images
  -> rewrite CSS backgrounds -> plugin post_render_html
  -> critical CSS inlining  <-- HERE
  -> minify HTML -> write to disk
```

Critical CSS runs after plugins (which may generate CSS files) and before minification (so the inlined CSS benefits from minify-html's CSS minifier).

## Configuration

In `site.toml`:

```toml
[build.critical_css]
enabled = true              # default: false (opt-in)
max_inline_size = 50000     # default: 50KB, skip inlining if exceeded
preload_full = true         # default: true, async-load full stylesheet
exclude = ["**/vendor/**"]  # glob patterns to skip
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Master switch |
| `max_inline_size` | usize | `50000` | Max bytes for inlined CSS before falling back |
| `preload_full` | bool | `true` | Keep async `<link>` for full stylesheet |
| `exclude` | Vec\<String\> | `[]` | Glob patterns for hrefs to skip |

## Deferred loading pattern

When `preload_full = true`, each processed `<link>` becomes:

```html
<style>/* critical CSS */</style>
<link rel="preload" href="/css/style.css" as="style"
      onload="this.onload=null;this.rel='stylesheet'">
<noscript><link rel="stylesheet" href="/css/style.css"></noscript>
```

When `preload_full = false`, the `<link>` is removed entirely (tree-shaking mode).

## What is skipped

- External stylesheets (http/https URLs)
- `<link>` tags with a `media` attribute (already non-blocking)
- Stylesheets matching `exclude` glob patterns
- Existing inline `<style>` blocks (already inline)
- Fragments (partial HTML loaded via HTMX)

## Error handling

Critical CSS is a pure optimization. All failures fall back to returning the original HTML unchanged:

- Missing CSS file: warning logged, stylesheet skipped
- CSS parse error: warning logged, stylesheet skipped
- Size limit exceeded: info logged, no inlining for that page
- HTML parse error: warning logged, original HTML returned
- Selector parse error: rule included conservatively

## Caching

A `StylesheetCache` is created per build to avoid re-reading the same CSS file for every page. The raw CSS text (after reading from disk and resolving @import) is cached. The CSS is re-parsed per page by lightningcss since parsing is fast and the AST contains page-specific matching state.

## Dev server

Critical CSS inlining is disabled during `eigen dev` regardless of configuration. The dev server render functions do not invoke it.

## Module structure

```
src/build/critical_css/
  mod.rs       -- public API, StylesheetCache, orchestration, @import resolution
  extract.rs   -- CSS parsing, rule walking, selector matching, global deps
  rewrite.rs   -- lol_html HTML mutation (inject <style>, rewrite <link>)
```
