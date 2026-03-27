# Draft / Scheduled Pages -- Design Spec

Date: 2026-03-18

## Motivation and Goals

Content authors need to work on pages without publishing them. Two
common scenarios:

1. **Work in progress:** A page is being written and isn't ready for
   production. It should be visible in `eigen dev` but excluded from
   `eigen build`.
2. **Scheduled publication:** A page is finished but embargoed until a
   specific date. It should appear in dev mode but be excluded from
   production builds until the publish date arrives.

**Goals:**

- Add `draft` and `publish_date` frontmatter fields for static pages.
- In production builds (`eigen build`), skip pages that are drafts or
  have a future publish date. They are not rendered, not in the
  sitemap, not in feeds, not audited.
- In dev mode (`eigen dev`), render all pages including drafts and
  scheduled pages, with a visual banner indicating their status.
- Zero impact on existing sites (both fields are optional with safe
  defaults).

**Non-goals:**

- Draft/scheduled filtering for individual collection items in dynamic
  pages. Filtering items is the data source's responsibility (already
  possible via the existing `filter` transform in data queries).
- Hour-level scheduling precision. `publish_date` is date-only.
- Draft-aware incremental builds (future feature).
- Per-environment config overrides.

## Frontmatter Fields

Two new optional fields on `Frontmatter` and `RawFrontmatter`:

```yaml
---
draft: true
publish_date: 2026-04-01
---
```

### `draft`

- Type: `bool`
- Default: `false`
- When `true`, the page is excluded from production builds regardless
  of `publish_date`.
- Semantics: "this page is not ready for anyone to see."

### `publish_date`

- Type: `Option<chrono::NaiveDate>`
- Default: `None` (no scheduling, page is published immediately)
- When set and the date is in the future (compared to the build date),
  the page is excluded from production builds.
- When set and the date is today or in the past, the page is published
  normally.
- Uses `chrono::NaiveDate` — chrono is already a dependency (used by
  `current_year()` in `template/functions.rs`).
- Format: `YYYY-MM-DD` (ISO 8601 date). Deserialized as
  `Option<String>` in `RawFrontmatter`, then parsed manually with
  `NaiveDate::parse_from_str(s, "%Y-%m-%d")` in `parse_frontmatter()`.
  This matches the existing pattern in `build/feed.rs` and
  `template/filters.rs` which parse dates from strings rather than
  relying on chrono's serde feature (which is not enabled).
- Semantics: "this page is finished but should not go live until this
  date."

### Publication Logic

A static page is **unpublished** if ANY of these conditions are true:

1. `draft == true`
2. `publish_date` is set and is after today's date

Both conditions are independent. A page can be:

- `draft: true` with no `publish_date` → permanent WIP
- `draft: false` with future `publish_date` → finished but embargoed
- `draft: true` with future `publish_date` → WIP with a target date
- `draft: false` with no `publish_date` → published (default)
- `draft: false` with past `publish_date` → published

### Dynamic Pages

Dynamic pages (collection templates like `[slug].html`) are never
filtered at the template level. The `draft` and `publish_date` fields
are parsed but ignored during rendering for dynamic pages. Filtering
individual collection items by draft status is the data source's
responsibility.

## Architecture

### Filtering in the Build Pipeline

Filtering happens in `build()` in `src/build/render.rs`, after page
discovery (line 66) and before the rendering loop (line 157). The
build function gains a `dev` parameter to distinguish dev from
production:

```rust
pub fn build(project_root: &Path, dev: bool) -> Result<()>
```

After discovery and before the rendering loop, filter the pages and
log any skipped drafts:

```rust
let total_discovered = pages.len();
let pages: Vec<PageDef> = if dev {
    pages
} else {
    let today = chrono::Utc::now().date_naive();
    pages.into_iter()
        .filter(|p| is_published(&p.frontmatter, today))
        .collect()
};
if pages.len() < total_discovered {
    tracing::info!(
        "Skipped {} draft/scheduled page(s).",
        total_discovered - pages.len()
    );
}
```

Uses `Utc::now()` for consistency with the existing build timestamp
(line 150 of render.rs) and predictability in CI environments.

The `is_published` helper:

```rust
fn is_published(fm: &Frontmatter, today: NaiveDate) -> bool {
    if fm.draft {
        return false;
    }
    match fm.publish_date {
        Some(date) if date > today => false,
        _ => true,
    }
}
```

In dev mode (`dev == true`), all pages pass the filter. In production
(`dev == false`), only published pages are rendered.

**Cascading exclusion:** Since filtered pages are never rendered, they
automatically do not appear in:

- **Sitemap** — `generate_sitemap` receives `&[RenderedPage]` (only
  rendered pages)
- **Feeds** — feeds fetch data independently, but static page feeds
  are not a pattern in eigen
- **Audits** — `run_audit` receives `&[RenderedPage]` (only rendered
  pages)

No changes needed to sitemap, feed, or audit modules.

### Dev Mode Differentiation

Currently `build()` takes only `project_root`. Adding `dev: bool`:

- `src/main.rs` line 32: `build::build(&project, false)` for
  `eigen build`
- `src/dev/rebuild.rs` line 155: The dev rebuild has its own
  `full_build()` method that constructs the build pipeline manually.
  It needs the same filtering logic but with `dev == true` (so no
  pages are filtered). Since dev always shows everything, the filter
  is a no-op in dev mode. The `full_build` method does not call
  `build::build()` — it has its own rendering loop. No change needed
  to the dev rebuild loop since it renders everything.

Wait — actually, looking at the code more carefully: `DevBuildState::full_build()`
has its own rendering loop (lines 192-231) that is separate from
`build::build()`. So there are two integration points:

1. **`build::build()`** — add `dev: bool` parameter, filter pages
   when `dev == false`.
2. **`DevBuildState::full_build()`** — does NOT need filtering (dev
   shows everything), but DOES need the draft banner injection.

### Dev Mode Banner Injection

For draft/scheduled pages rendered in dev mode, inject a visual
banner before `</body>`. This follows the same pattern as the
existing `inject_reload_script` in `src/dev/inject.rs`.

Add `inject_status_banner(html: &str, label: &str) -> String` to
`src/dev/inject.rs`. The function takes a pre-computed label string
rather than a `Frontmatter` reference, keeping `inject.rs` as a pure
string-manipulation module with no domain dependencies. The caller in
`rebuild.rs` computes the label from the frontmatter fields:

1. If `draft == true`, inject a "DRAFT" banner.
2. If `publish_date` is set and in the future, inject a
   "SCHEDULED: YYYY-MM-DD" banner.
3. If both, show "DRAFT | SCHEDULED: YYYY-MM-DD".
4. If neither, return HTML unchanged.

The banner is a fixed-position div at the bottom of the viewport:

```html
<div id="eigen-draft-banner" style="position:fixed;bottom:0;left:0;
right:0;background:#b91c1c;color:#fff;text-align:center;
padding:6px 12px;font:14px/1.4 system-ui;z-index:99999;">
  DRAFT
</div>
```

Inline-styled (no CSS dependency), red background, unmistakable.
Injected before `</body>` using the same `rfind("</body>")` pattern
as `inject_reload_script`.

Called in `render_static_page_dev` (line ~349), right before or after
`inject_reload_script`. The caller computes the label string from the
frontmatter fields. Dynamic pages do NOT get the banner — since
dynamic pages are never filtered, showing a "DRAFT" banner on every
generated page from a collection template would be confusing.

The banner has `id="eigen-draft-banner"` for easy CSS targeting and
potential dismissal via user JS.

## Impact on Existing Code

### Files Modified

| File | Change |
|---|---|
| `src/frontmatter/mod.rs` | Add `draft: bool` and `publish_date: Option<NaiveDate>` to `Frontmatter` and `RawFrontmatter`, parse `publish_date` from string in `parse_frontmatter`, update `Default` |
| `src/build/render.rs` | Add `dev: bool` to `build()`, add `is_published()` helper, filter pages before rendering loop |
| `src/dev/inject.rs` | Add `inject_status_banner()` function |
| `src/dev/rebuild.rs` | Compute banner label from frontmatter, call `inject_status_banner()` in `render_static_page_dev` |
| `src/main.rs` | Pass `false` to `build::build()` for both `Build` and `Audit` commands |

### Files NOT Modified

- `src/build/sitemap.rs` — no changes, draft pages are never rendered
- `src/build/feed.rs` — no changes, feeds use data queries
- `src/build/audit/` — no changes, audits only see rendered pages
- `src/config/mod.rs` — no config changes, this is per-page frontmatter
- `src/discovery/mod.rs` — no changes, all pages are still discovered
  (filtering happens later in the pipeline)
- `src/dev/server.rs` — no changes, server infrastructure unchanged
- `Cargo.toml` — no new dependencies (chrono already present)

## Error Handling

| Scenario | Behavior |
|---|---|
| `draft` field missing | Defaults to `false` (page is published) |
| `publish_date` field missing | Defaults to `None` (no scheduling) |
| Invalid `publish_date` format | Parse error in `parse_frontmatter()` with a clear message naming the bad value |
| `draft: true` on dynamic page | Parsed but ignored — dynamic pages are never filtered |
| `publish_date` exactly today | Page IS published (`date > today` is false) |

## Test Plan

### Frontmatter Tests (in `src/frontmatter/mod.rs`)

| Test | What it verifies |
|---|---|
| `test_parse_draft_true` | `draft: true` parsed correctly |
| `test_parse_draft_false` | `draft: false` parsed correctly |
| `test_parse_draft_default` | Missing `draft` defaults to `false` |
| `test_parse_publish_date` | `publish_date: 2026-04-01` parsed as `NaiveDate` |
| `test_parse_publish_date_absent` | Missing `publish_date` defaults to `None` |

### Filter Logic Tests (in `src/build/render.rs`)

| Test | What it verifies |
|---|---|
| `test_is_published_default` | Default frontmatter is published |
| `test_is_published_draft` | `draft: true` is not published |
| `test_is_published_future_date` | Future `publish_date` is not published |
| `test_is_published_past_date` | Past `publish_date` is published |
| `test_is_published_today` | Today's `publish_date` is published |
| `test_is_published_draft_and_future` | `draft: true` + future date is not published |

### Banner Injection Tests (in `src/dev/inject.rs`)

| Test | What it verifies |
|---|---|
| `test_inject_draft_banner` | Draft page gets "DRAFT" banner |
| `test_inject_scheduled_banner` | Future publish_date gets "SCHEDULED: YYYY-MM-DD" banner |
| `test_inject_draft_and_scheduled_banner` | Both draft + scheduled shows combined banner |
| `test_inject_no_banner` | Non-draft, no-schedule page gets no banner |
| `test_inject_banner_no_body` | HTML without `</body>` still works |

## What Is NOT In Scope

1. **Dynamic page item filtering.** Individual collection items with
   draft/published status should be filtered via data query transforms,
   not template-level frontmatter.

2. **Hour-level scheduling.** `publish_date` is date-only (NaiveDate).
   Static sites are published when `eigen build` runs, so sub-day
   precision is meaningless.

3. **Draft-aware incremental builds.** A future incremental build
   system would need to re-check publish dates on each build. Out of
   scope for this feature.

4. **Visual customization of the dev banner.** The banner style is
   hardcoded. Users who want different styling can target
   `#eigen-draft-banner` in their own CSS.

5. **Config-level draft override.** No `[build] include_drafts = true`
   setting. The `eigen dev` command is the way to see drafts.

## Performance Considerations

- The filter runs once per build over the `Vec<PageDef>`. This is
  O(n) where n is the number of pages — negligible.
- Draft pages are skipped entirely: no template rendering, no data
  fetching, no file I/O. This actually speeds up production builds
  for sites with many drafts.
- The banner injection in dev mode adds ~200 bytes per page. Same
  pattern as the existing reload script injection.
- No new dependencies.
