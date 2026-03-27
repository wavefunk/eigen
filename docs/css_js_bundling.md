# CSS/JS Bundling and Tree-Shaking

## Overview

CSS/JS bundling merges multiple CSS and JS files into single bundled files,
reduces HTTP requests, and removes unused CSS selectors via tree-shaking.

## Configuration

Add to `site.toml`:

```toml
[build.bundling]
enabled = true
```

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `enabled` | `false` | Master switch (opt-in) |
| `css` | `true` | Bundle CSS files |
| `tree_shake_css` | `true` | Remove unused CSS selectors |
| `js` | `true` | Bundle JS files |
| `css_output` | `"css/bundle.css"` | Output path for CSS bundle |
| `js_output` | `"js/bundle.js"` | Output path for JS bundle |
| `exclude` | `[]` | Glob patterns to exclude from bundling |

### Examples

CSS only (no JS bundling):
```toml
[build.bundling]
enabled = true
js = false
```

Exclude vendor files:
```toml
[build.bundling]
enabled = true
exclude = ["**/vendor/**", "**/print.css"]
```

## Pipeline Position

Runs as Phase 2.5 in the build pipeline:
1. Phase 1: Copy static assets (+ content hash manifest)
2. Phase 2: Render all pages (per-page pipeline)
3. Phase 2 (cont): Sitemap, post-build plugins
4. **Phase 2.5: CSS/JS bundling and tree-shaking** <-- this feature
5. Phase 3: Content hash rewrite

## How It Works

### CSS Bundling
1. Scans all HTML files in `dist/` for `<link rel="stylesheet">` tags
2. Deduplicates hrefs in first-encounter order (sorted by file path)
3. Reads and concatenates CSS files, resolving `@import` chains
4. Tree-shakes: parses all HTML DOMs, keeps selectors matching any page
5. Writes bundled CSS to `dist/{css_output}`

### JS Bundling
1. Scans all HTML files for `<script src="...">` tags
2. Skips: `defer`, `async`, `type="module"`, inline scripts
3. Wraps each file in IIFE: `;(function(){ ... })();`
4. Writes bundled JS to `dist/{js_output}`

### HTML Rewriting
- First `<link>`/`<script>` referencing a bundled file: rewritten to bundle
- Subsequent references: removed
- Critical CSS preload pattern: both preload `<link>` and `<noscript>` fallback rewritten

## Interactions

- **Critical CSS**: Runs before bundling (Phase 2). Bundling rewrites the preload links.
- **Content Hashing**: Runs after bundling (Phase 3). Hashes the bundle files.
- **Dev Server**: Bundling is disabled during dev.

## What Is Skipped
- External URLs (http/https)
- `<link>` with `media` attribute
- `<script>` with `defer`, `async`, or `type="module"`
- Inline `<style>` and `<script>` blocks
- Paths matching `exclude` patterns

## Module Structure
```
src/build/bundling/
  mod.rs      -- public API (bundle_assets), orchestration
  collect.rs  -- HTML scanning, reference collection
  css.rs      -- CSS merging, tree-shaking
  js.rs       -- JS concatenation with IIFE wrapping
  rewrite.rs  -- HTML rewriting
```
