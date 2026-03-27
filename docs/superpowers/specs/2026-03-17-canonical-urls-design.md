# Canonical URL Tags -- Design Spec

Date: 2026-03-17

## Status: ALREADY IMPLEMENTED

The canonical URL feature described in this milestone is **fully
implemented** as part of the OG/Twitter Card meta tag feature in
`build::seo`. This design spec documents the existing implementation,
confirms completeness against the milestone requirements, and
identifies one minor improvement opportunity.

## Milestone Requirements

The milestone specifies:

> Auto-insert link rel=canonical using base_url plus page path.
> Prevents duplicate content issues. Allow frontmatter override with
> a canonical field.

Three requirements:

1. **Auto-insert `<link rel="canonical">`** using `base_url` + page path.
2. **Prevent duplicate content issues** (the purpose).
3. **Allow frontmatter override** via a `canonical` field.

## Existing Implementation

All three requirements are satisfied by the existing code in
`src/build/seo.rs`, integrated into the build pipeline via
`src/build/render.rs`.

### Requirement 1: Auto-insert canonical URL

**File:** `src/build/seo.rs`, `resolve_seo` function (lines 130-143)

The canonical URL is auto-generated from `site.base_url` +
`current_url` (the page's URL path). The `index.html` suffix is
stripped for cleaner URLs:

- `/about.html` becomes `https://example.com/about.html`
- `/index.html` becomes `https://example.com/`
- `/blog/index.html` becomes `https://example.com/blog/`

The `generate_meta_html` function (lines 280-288) emits:

```html
<link rel="canonical" href="https://example.com/about.html">
```

This is injected into `<head>` by `inject_into_head` using lol_html.

### Requirement 2: Prevent duplicates

**File:** `src/build/seo.rs`, `has_canonical_link` function (lines
198-217)

Before injecting, the pipeline scans the rendered HTML for an existing
`<link rel="canonical">` tag using lol_html. If one is already present
(author-provided in the template), the auto-generated canonical tag
is skipped. This prevents duplicate `<link rel="canonical">` tags.

### Requirement 3: Frontmatter override

**File:** `src/frontmatter/mod.rs`, `SeoMeta` struct (lines 105-107)

The `canonical_url` field on `SeoMeta` allows per-page override:

```yaml
---
seo:
  canonical_url: https://original-source.com/article
---
```

When set, this value is used verbatim instead of the auto-generated
URL. This supports syndicated content scenarios where the canonical
source is a different domain.

For dynamic pages, the `canonical_url` field supports minijinja
template expressions:

```yaml
---
seo:
  canonical_url: "{{ post.original_url }}"
---
```

These are resolved per-item by `resolve_seo_expressions` (line 416
in `seo.rs`).

## Architecture

### Pipeline Position

The canonical URL tag is part of the SEO meta tag injection step,
which slots into the per-page build pipeline after preload/prefetch
hints and before HTML minification:

```
render template
  -> strip fragment markers
  -> localize assets
  -> optimize images
  -> rewrite CSS background images
  -> plugin post_render_html
  -> critical CSS inlining        (build::critical_css)
  -> preload/prefetch hints       (build::hints)
  -> SEO meta tag injection       (build::seo)  <-- canonical URL here
  -> JSON-LD structured data      (build::json_ld)
  -> minify HTML                  (build::minify)
  -> write to disk
```

### Resolution Logic

The canonical URL value comes from a two-layer cascade:

1. **Frontmatter override:** `seo.canonical_url` in frontmatter, if
   set, is used verbatim (after template expression resolution for
   dynamic pages).
2. **Auto-derived:** `base_url + current_url`, with `index.html`
   stripped from the path suffix.

### Integration Points

- **`src/build/render.rs`:** `inject_seo_tags` is called in both
  `render_static_page` (line 433) and `render_dynamic_page` (line
  731), passing `&url_path` as the `current_url` parameter.
- **`src/config/mod.rs`:** `SiteMeta.base_url` provides the base URL.
  `validate_config` rejects empty `base_url` at load time.
- **`src/frontmatter/mod.rs`:** `SeoMeta.canonical_url` provides the
  per-page override.

### URL Normalization

- Trailing slash on `base_url` is stripped before concatenation to
  avoid double slashes (`https://example.com//about.html`).
- `index.html` at the end of the URL path is stripped so that
  `/blog/index.html` produces canonical URL `https://example.com/blog/`
  rather than `https://example.com/blog/index.html`.
- Already-absolute URLs (starting with `http://` or `https://`) in
  frontmatter overrides are used as-is.

## Test Coverage

All canonical URL behaviors are tested in `src/build/seo.rs`:

| Test | What it verifies |
|---|---|
| `test_resolve_seo_all_defaults` | Canonical URL is `base_url + current_url` with default config |
| `test_resolve_seo_canonical_url_auto` | Auto-generated canonical URL for `/blog/post.html` |
| `test_resolve_seo_canonical_url_override` | Frontmatter `canonical_url` overrides auto-generation |
| `test_resolve_seo_canonical_strips_index` | `/index.html` stripped to `/`, `/blog/index.html` to `/blog/` |
| `test_resolve_seo_base_url_slash_normalization` | No double slash when `base_url` ends with `/` |
| `test_has_canonical_link_present` | Detects existing `<link rel="canonical">` in HTML |
| `test_has_canonical_link_absent` | Correctly returns false when no canonical link exists |
| `test_generate_meta_html_full` | Canonical link tag is generated with correct href |
| `test_generate_meta_html_skips_canonical` | Canonical link NOT generated when `has_canonical` is true |
| `test_inject_seo_tags_basic` | End-to-end: canonical link appears in output HTML |
| `test_inject_seo_tags_all_existing` | No duplicate canonical link when one already exists |
| `test_inject_seo_tags_partial_existing` | Canonical link injected alongside existing partial OG tags |

Additionally, frontmatter parsing tests:

| Test | What it verifies |
|---|---|
| `test_parse_seo_frontmatter_full` | `canonical_url` parsed from YAML |
| `test_parse_seo_frontmatter_partial` | `canonical_url` is None when not specified |
| `test_parse_seo_frontmatter_absent` | Default `SeoMeta` has `canonical_url: None` |

## Improvement Opportunity: Deduplicate Canonical URL Logic

The only actionable finding from this review is a **minor code
duplication** between `src/build/seo.rs` and `src/build/json_ld.rs`.

Both files contain:

1. A private `make_absolute_url` function (identical logic).
2. A private `canonical_url` function in `json_ld.rs` that
   reimplements the same `index.html`-stripping + `make_absolute_url`
   logic that lives in `resolve_seo` in `seo.rs`.

This duplication is harmless (both implementations are correct and
tested), but could be consolidated by extracting a shared helper into
a common location (e.g. a `build::url` module or a utility function
on `SiteMeta`).

**Severity:** Low. The logic is simple (< 15 lines each), both copies
are tested, and the two modules have no other shared surface. This is
a "nice to have" refactor, not a correctness issue.

## What Is NOT In Scope

1. **Trailing slash policy configuration.** Some sites prefer
   `/about/` while others prefer `/about.html`. The current behavior
   (preserve the original URL path, strip only `index.html`) is the
   most common convention. A configurable trailing slash policy would
   add complexity for an edge case.

2. **Sitemap interaction.** The sitemap generator (`build::sitemap`)
   produces its own URLs independently. Ensuring sitemap URLs match
   canonical URLs is desirable but is a separate concern.

3. **`<meta name="robots">` tags.** Canonical URLs and robots
   directives serve different purposes. Robots directives are out of
   scope for this milestone.

4. **Per-page disable.** There is no mechanism to suppress canonical
   URL generation for specific pages. This is intentional -- every
   page should have a canonical URL.

## Conclusion

**No implementation work is required.** The canonical URL feature was
implemented as part of the OG/Twitter Card meta tag feature
(`build::seo`). All three milestone requirements are met:

- Auto-insertion using `base_url` + page path
- Duplicate prevention via existing-tag detection
- Frontmatter override via `seo.canonical_url`

The optional improvement (deduplicating `make_absolute_url` /
canonical URL logic between `seo.rs` and `json_ld.rs`) is documented
but does not block this milestone.
