# Draft and Scheduled Pages

Mark pages as drafts or schedule them for future publication. Draft and
scheduled pages are excluded from production builds but visible in dev
mode.

## Draft Pages

Add `draft: true` to a page's frontmatter:

```yaml
---
draft: true
---
```

The page will be:
- **Excluded** from `eigen build` output (not rendered, not in sitemap)
- **Visible** in `eigen dev` with a red "DRAFT" banner at the bottom

## Scheduled Pages

Set a `publish_date` to schedule publication:

```yaml
---
publish_date: 2026-04-01
---
```

The page will be excluded from production builds until the date arrives.
On the publish date and after, it builds normally.

## Combining Both

You can use both fields together:

```yaml
---
draft: true
publish_date: 2026-04-01
---
```

This means "work in progress, targeting April 1st." The page is excluded
from production builds due to `draft: true` regardless of the date. Remove
`draft: true` when the content is ready, and the page will auto-publish
on the scheduled date.

## Dev Mode

In `eigen dev`, all pages are rendered including drafts and scheduled
pages. A fixed banner at the bottom of the viewport shows the page status:

- **DRAFT** — for pages with `draft: true`
- **SCHEDULED: 2026-04-01** — for pages with a future publish date
- **DRAFT | SCHEDULED: 2026-04-01** — for both

The banner has `id="eigen-draft-banner"` if you need to style or hide it.

## Scope

Draft/scheduled filtering applies to **static pages only**. Dynamic
collection pages (e.g., `[slug].html`) are never skipped — filter
individual collection items via data query transforms instead.

The `publish_date` format is `YYYY-MM-DD` (date only, no time component).
