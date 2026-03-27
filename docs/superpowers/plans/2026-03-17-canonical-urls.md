# Canonical URL Tags Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-insert `<link rel="canonical">` using `base_url` plus page path, preventing duplicate content issues, with frontmatter override support.

**Status:** ALREADY IMPLEMENTED. All milestone requirements are met by the existing `build::seo` module. This plan documents the verification performed and an optional refactoring task.

**Architecture:** Canonical URL generation and injection is handled by `src/build/seo.rs` as part of the OG/Twitter Card meta tag injection pipeline step. The canonical URL is auto-derived from `config.site.base_url + current_url` with `index.html` stripping, can be overridden via `seo.canonical_url` in frontmatter (including minijinja template expressions for dynamic pages), and is skipped if the HTML already contains a `<link rel="canonical">` tag.

**Tech Stack:** Rust, lol_html (HTML scanning and injection), minijinja (template expression resolution)

---

## Verification Checklist

All items below have been verified as complete in the existing codebase.

### Auto-generation

- [x] `resolve_seo` in `src/build/seo.rs` (line 130) computes canonical URL from `base_url + current_url`
- [x] `index.html` suffix stripped: `/index.html` -> `/`, `/blog/index.html` -> `/blog/` (line 136)
- [x] Trailing slash on `base_url` normalized via `make_absolute_url` (line 66)
- [x] `generate_meta_html` emits `<link rel="canonical" href="...">` (line 280)
- [x] `inject_into_head` injects into `<head>` via lol_html (line 299)

### Frontmatter Override

- [x] `SeoMeta.canonical_url: Option<String>` in `src/frontmatter/mod.rs` (line 107)
- [x] `RawFrontmatter.seo` field with `#[serde(default)]` in `src/frontmatter/mod.rs` (line 188)
- [x] `parse_frontmatter` passes through `seo` field in `src/frontmatter/mod.rs` (line 265)
- [x] `resolve_seo` uses frontmatter `canonical_url` when present (line 131)

### Template Expression Support

- [x] `resolve_seo_expressions` resolves `canonical_url` field (line 416 in `src/build/seo.rs`)
- [x] Called in `render_static_page` before `inject_seo_tags` (in `src/build/render.rs`)
- [x] Called in `render_dynamic_page` per-item before `inject_seo_tags` (in `src/build/render.rs`)

### Duplicate Prevention

- [x] `has_canonical_link` scans for existing `<link rel="canonical">` (line 198 in `src/build/seo.rs`)
- [x] `generate_meta_html` skips canonical link when `has_canonical` is true (line 281)
- [x] `inject_seo_tags` orchestrates detection + generation (line 342)

### Pipeline Integration

- [x] `inject_seo_tags` called in `render_static_page` at line 433 in `src/build/render.rs`
- [x] `inject_seo_tags` called in `render_dynamic_page` at line 731 in `src/build/render.rs`
- [x] Position: after hints, before minify (correct pipeline ordering)
- [x] Fragments excluded (no `<head>` element, so lol_html selector does not match)

### Test Coverage

- [x] `test_resolve_seo_all_defaults` -- canonical URL with defaults
- [x] `test_resolve_seo_canonical_url_auto` -- auto-generation
- [x] `test_resolve_seo_canonical_url_override` -- frontmatter override
- [x] `test_resolve_seo_canonical_strips_index` -- index.html stripping
- [x] `test_resolve_seo_base_url_slash_normalization` -- slash normalization
- [x] `test_has_canonical_link_present` -- detection of existing tag
- [x] `test_has_canonical_link_absent` -- absence detection
- [x] `test_generate_meta_html_full` -- tag generation
- [x] `test_generate_meta_html_skips_canonical` -- skip when present
- [x] `test_inject_seo_tags_basic` -- end-to-end injection
- [x] `test_inject_seo_tags_all_existing` -- no duplicates
- [x] `test_inject_seo_tags_partial_existing` -- partial injection
- [x] `test_parse_seo_frontmatter_full` -- YAML parsing
- [x] `test_parse_seo_frontmatter_partial` -- partial YAML parsing
- [x] `test_parse_seo_frontmatter_absent` -- default when absent

---

## Optional: Deduplicate URL Helpers (Low Priority)

The following refactoring task is not required for this milestone but
is documented for future cleanup.

### Task: Extract shared URL utilities

- [ ] Create `src/build/url.rs` with:
  - `pub fn make_absolute_url(url: &str, base_url: &str) -> String`
  - `pub fn canonical_page_url(current_url: &str, base_url: &str) -> String`
- [ ] Add `pub mod url;` to `src/build/mod.rs`
- [ ] Update `src/build/seo.rs` to use `url::make_absolute_url` instead of its private copy
- [ ] Update `src/build/json_ld.rs` to use `url::make_absolute_url` and `url::canonical_page_url` instead of its private copies
- [ ] Remove the private `make_absolute_url` from both `seo.rs` and `json_ld.rs`
- [ ] Remove the private `canonical_url` from `json_ld.rs`
- [ ] Move relevant tests to `url.rs`, keep integration tests in their original modules
- [ ] Run `cargo test` to confirm no regressions

**Affected files:**

| File | Change |
|---|---|
| `src/build/url.rs` | NEW -- shared URL utility functions |
| `src/build/mod.rs` | Add `pub mod url;` |
| `src/build/seo.rs` | Replace private `make_absolute_url` with `url::make_absolute_url` |
| `src/build/json_ld.rs` | Replace private `make_absolute_url` and `canonical_url` with shared versions |

**Estimated effort:** ~30 minutes. Pure refactor, no behavior change.

---

## Summary

**Required implementation tasks: 0**
**Optional refactoring tasks: 1** (7 sub-steps, low priority)

The canonical URL feature is complete. All three milestone requirements
(auto-insertion, duplicate prevention, frontmatter override) are
implemented, tested, and integrated into the build pipeline.
