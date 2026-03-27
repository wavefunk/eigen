# Content Hashing for Static Assets

## Overview

Content hashing fingerprints static assets (CSS, JS, images, fonts) by
embedding a SHA-256 hash in the filename. This enables browsers and CDNs
to cache assets indefinitely with `Cache-Control: immutable`.

Example: `style.css` becomes `style.a1b2c3d4e5f67890.css`

## Configuration

Add to `site.toml`:

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
    "sw.js",           # service workers need stable URLs
    "manifest.json",   # PWA manifests need stable URLs
]
```

## How It Works

### Three Phases

1. **Phase 1 (Build Manifest):** During `copy_static_assets`, files in
   `dist/` are hashed and renamed. A manifest maps original paths to
   hashed paths.

2. **Phase 2 (Template Resolution):** The `asset()` template function
   uses the manifest to return hashed paths at render time.

3. **Phase 3 (Post-Render Rewrite):** After all rendering, a final pass
   rewrites any remaining hardcoded asset references in `.html`, `.css`,
   and `.js` files.

### Template Usage

Always use the `asset()` function for static asset references:

```html
<link rel="stylesheet" href="{{ asset('/css/style.css') }}">
<script src="{{ asset('/js/app.js') }}"></script>
<img src="{{ asset('/images/logo.png') }}" alt="Logo">
```

Hardcoded paths also work (Phase 3 rewrites them), but `asset()` is
preferred because it gives templates the correct URL at render time.

## Default Exclusions

These files are excluded from hashing by default (they must keep stable
names for deployment platforms):

- `favicon.ico`
- `robots.txt`
- `CNAME`
- `_headers`
- `_redirects`
- `.well-known/**`

## Dev Server

Content hashing is always disabled during dev server operation. The
`asset()` function returns paths unchanged in dev mode.

## Interactions

- **Image optimization:** Works correctly. Use `asset()` for source
  image paths; image optimization reads the hashed filename from dist.
- **Critical CSS:** Enhanced to resolve hashed paths via manifest
  fallback when the original file path is not found.
- **Resource hints:** Preload/prefetch URLs naturally use hashed paths
  when templates use `asset()`.
- **Asset localization:** Downloaded assets are not hashed (they already
  have content-based filenames).

## Limitations

- Relative CSS `url()` paths (e.g., `../images/icon.png`) are not
  rewritten. Use absolute paths in CSS files.
- Dynamic JavaScript references constructed at runtime cannot be
  statically rewritten.
- Source map relative references may not be rewritten. Use absolute
  paths or exclude `*.map` files.

## Module Location

`src/build/content_hash.rs` -- single-file module containing
`AssetManifest`, `build_manifest()`, and `rewrite_references()`.
