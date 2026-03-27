# Draft / Scheduled Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `draft` and `publish_date` frontmatter fields so static pages can be excluded from production builds while remaining visible in dev mode with a status banner.

**Architecture:** New fields on `Frontmatter`/`RawFrontmatter`, `publish_date` parsed manually from string to `NaiveDate`. Filtering in `build()` before the rendering loop via an `is_published()` helper. Dev mode banner injected via `inject_status_banner()` in `dev/inject.rs` following the existing `inject_reload_script` pattern.

**Tech Stack:** Rust, chrono::NaiveDate (already a dependency), serde_yaml

**Spec:** `docs/superpowers/specs/2026-03-18-draft-pages-design.md`

---

### Task 1: Add `draft` and `publish_date` to frontmatter

**Files:**
- Modify: `src/frontmatter/mod.rs:7-46` (Frontmatter struct and Default)
- Modify: `src/frontmatter/mod.rs:178-191` (RawFrontmatter struct)
- Modify: `src/frontmatter/mod.rs:254-268` (parse_frontmatter function)

- [ ] **Step 1: Write the failing tests**

Add at the end of the `#[cfg(test)]` block in `src/frontmatter/mod.rs`:

```rust
#[test]
fn test_parse_draft_true() {
    let yaml = "draft: true\n";
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert!(fm.draft);
}

#[test]
fn test_parse_draft_false() {
    let yaml = "draft: false\n";
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert!(!fm.draft);
}

#[test]
fn test_parse_draft_default() {
    let yaml = "";
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert!(!fm.draft);
}

#[test]
fn test_parse_publish_date() {
    let yaml = "publish_date: \"2026-04-01\"\n";
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert_eq!(
        fm.publish_date,
        Some(chrono::NaiveDate::from_ymd_opt(2026, 4, 1).unwrap())
    );
}

#[test]
fn test_parse_publish_date_absent() {
    let yaml = "";
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert!(fm.publish_date.is_none());
}

#[test]
fn test_parse_publish_date_invalid() {
    let yaml = "publish_date: \"not-a-date\"\n";
    let result = parse_frontmatter(yaml, "test.html");
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test frontmatter::tests::test_parse_draft -- --nocapture`
Expected: compilation error — `draft` field does not exist on `Frontmatter`

- [ ] **Step 3: Add fields to Frontmatter and RawFrontmatter, update parsing**

In `Frontmatter` struct (after `schema` field at line 31):

```rust
    /// Whether this page is a draft (excluded from production builds).
    pub draft: bool,
    /// Scheduled publication date. Pages with a future date are excluded
    /// from production builds.
    pub publish_date: Option<chrono::NaiveDate>,
```

In `Frontmatter::default()` (after `schema: None` at line 44):

```rust
            draft: false,
            publish_date: None,
```

In `RawFrontmatter` (after `schema` field at line 190):

```rust
    #[serde(default)]
    draft: bool,
    publish_date: Option<String>,
```

In `parse_frontmatter` (after `schema: raw.schema` at line 266), add `publish_date` parsing and the new fields:

```rust
    let publish_date = match raw.publish_date {
        Some(ref s) => {
            let date = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .wrap_err_with(|| {
                    format!(
                        "Invalid publish_date '{}' in {file_path} (expected YYYY-MM-DD)",
                        s
                    )
                })?;
            Some(date)
        }
        None => None,
    };

    Ok(Frontmatter {
        collection: raw.collection,
        slug_field: raw.slug_field.unwrap_or_else(|| "slug".into()),
        item_as: raw.item_as.unwrap_or_else(|| "item".into()),
        data: raw.data,
        fragment_blocks: raw.fragment_blocks,
        hero_image: raw.hero_image,
        seo: raw.seo,
        schema: raw.schema,
        draft: raw.draft,
        publish_date,
    })
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test frontmatter::tests::test_parse_draft -- --nocapture`
Run: `cargo test frontmatter::tests::test_parse_publish_date -- --nocapture`
Expected: all 6 new tests PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 6: Commit**

```
feat(frontmatter): add draft and publish_date fields
```

---

### Task 2: Add `is_published` filter and wire into `build()`

**Files:**
- Modify: `src/build/render.rs:51` (build function signature)
- Modify: `src/build/render.rs:67-73` (after discovery, before rendering loop)
- Modify: `src/main.rs:32` (Build command call)
- Modify: `src/main.rs:62` (Audit command call)

- [ ] **Step 1: Write the failing tests**

Add a `#[cfg(test)]` block at the bottom of `src/build/render.rs` (or near the existing helpers):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::Frontmatter;
    use chrono::NaiveDate;

    #[test]
    fn test_is_published_default() {
        let fm = Frontmatter::default();
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert!(is_published(&fm, today));
    }

    #[test]
    fn test_is_published_draft() {
        let fm = Frontmatter { draft: true, ..Default::default() };
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert!(!is_published(&fm, today));
    }

    #[test]
    fn test_is_published_future_date() {
        let fm = Frontmatter {
            publish_date: Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
            ..Default::default()
        };
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert!(!is_published(&fm, today));
    }

    #[test]
    fn test_is_published_past_date() {
        let fm = Frontmatter {
            publish_date: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            ..Default::default()
        };
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert!(is_published(&fm, today));
    }

    #[test]
    fn test_is_published_today() {
        let fm = Frontmatter {
            publish_date: Some(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
            ..Default::default()
        };
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert!(is_published(&fm, today));
    }

    #[test]
    fn test_is_published_draft_and_future() {
        let fm = Frontmatter {
            draft: true,
            publish_date: Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
            ..Default::default()
        };
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        assert!(!is_published(&fm, today));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test build::render::tests::test_is_published -- --nocapture`
Expected: compilation error — `is_published` does not exist

- [ ] **Step 3: Add `is_published` helper**

Add near the other helpers in `render.rs` (e.g., near `count_page_types`):

```rust
/// Check whether a page should be included in production builds.
///
/// A page is unpublished if `draft == true` or if `publish_date` is
/// set and is after `today`.
fn is_published(fm: &crate::frontmatter::Frontmatter, today: chrono::NaiveDate) -> bool {
    if fm.draft {
        return false;
    }
    match fm.publish_date {
        Some(date) if date > today => false,
        _ => true,
    }
}
```

- [ ] **Step 4: Run `is_published` tests**

Run: `cargo test build::render::tests::test_is_published -- --nocapture`
Expected: all 6 tests PASS

- [ ] **Step 5: Change `build()` signature and add filtering**

Change `build()` signature from:

```rust
pub fn build(project_root: &Path) -> Result<()> {
```

to:

```rust
pub fn build(project_root: &Path, dev: bool) -> Result<()> {
```

After the page discovery and logging (after line 73), add filtering:

```rust
    // Filter out draft and future-scheduled pages in production builds.
    let total_discovered = pages.len();
    let pages: Vec<PageDef> = if dev {
        pages
    } else {
        let today = chrono::Utc::now().date_naive();
        pages.into_iter()
            .filter(|p| p.page_type.is_dynamic() || is_published(&p.frontmatter, today))
            .collect()
    };
    let skipped = total_discovered - pages.len();
    if skipped > 0 {
        tracing::info!("Skipped {} draft/scheduled page(s).", skipped);
    }
```

Note: dynamic pages are never filtered (per spec). Check if `PageType` has an `is_dynamic()` method. If not, use `matches!(p.page_type, PageType::Dynamic { .. })`.

- [ ] **Step 6: Update call sites in `main.rs`**

Line 32 (Build command):
```rust
            build::build(&project, false)?;
```

Line 62 (Audit command):
```rust
                build::build(&project, false)?;
```

- [ ] **Step 7: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 8: Commit**

```
feat(build): filter draft and scheduled pages in production builds
```

---

### Task 3: Add dev mode status banner injection

**Files:**
- Modify: `src/dev/inject.rs` (add `inject_status_banner` function)
- Modify: `src/dev/rebuild.rs:349` (call banner in `render_static_page_dev`)

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` block in `src/dev/inject.rs`:

```rust
#[test]
fn test_inject_status_banner_draft() {
    let html = "<html><body><h1>Hi</h1></body></html>";
    let result = inject_status_banner(html, "DRAFT");
    assert!(result.contains("DRAFT"));
    assert!(result.contains("eigen-draft-banner"));
    let banner_pos = result.find("eigen-draft-banner").unwrap();
    let body_close_pos = result.find("</body>").unwrap();
    assert!(banner_pos < body_close_pos);
}

#[test]
fn test_inject_status_banner_scheduled() {
    let html = "<html><body><h1>Hi</h1></body></html>";
    let result = inject_status_banner(html, "SCHEDULED: 2026-04-01");
    assert!(result.contains("SCHEDULED: 2026-04-01"));
    assert!(result.contains("eigen-draft-banner"));
}

#[test]
fn test_inject_status_banner_empty_label() {
    let html = "<html><body><h1>Hi</h1></body></html>";
    let result = inject_status_banner(html, "");
    assert_eq!(result, html, "Empty label should not inject a banner");
}

#[test]
fn test_inject_status_banner_no_body() {
    let html = "<h1>Fragment</h1>";
    let result = inject_status_banner(html, "DRAFT");
    assert!(result.contains("DRAFT"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test inject::tests::test_inject_status_banner -- --nocapture`
Expected: compilation error — `inject_status_banner` does not exist

- [ ] **Step 3: Implement `inject_status_banner`**

Add to `src/dev/inject.rs` (after the `inject_reload_script` function):

```rust
/// Inject a status banner (e.g. "DRAFT" or "SCHEDULED: 2026-04-01")
/// into rendered HTML during dev mode.
///
/// The banner is a fixed-position div at the bottom of the viewport.
/// If `label` is empty, returns the HTML unchanged.
pub fn inject_status_banner(html: &str, label: &str) -> String {
    if label.is_empty() {
        return html.to_string();
    }

    let banner = format!(
        r#"<div id="eigen-draft-banner" style="position:fixed;bottom:0;left:0;right:0;background:#b91c1c;color:#fff;text-align:center;padding:6px 12px;font:14px/1.4 system-ui;z-index:99999;">{}</div>"#,
        label
    );

    let lower = html.to_lowercase();
    if let Some(pos) = lower.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + banner.len() + 1);
        result.push_str(&html[..pos]);
        result.push('\n');
        result.push_str(&banner);
        result.push('\n');
        result.push_str(&html[pos..]);
        result
    } else {
        format!("{}\n{}", html, banner)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test inject::tests::test_inject_status_banner -- --nocapture`
Expected: all 4 tests PASS

- [ ] **Step 5: Wire into `render_static_page_dev`**

In `src/dev/rebuild.rs`, in `render_static_page_dev` (around line 349 where `inject_reload_script` is called), add banner injection right after the reload script:

```rust
    let full_html = inject::inject_reload_script(&full_html);

    // Inject draft/scheduled status banner.
    let draft_label = build_draft_label(&page.frontmatter);
    let full_html = inject::inject_status_banner(&full_html, &draft_label);
```

Add the `build_draft_label` helper near the top of `rebuild.rs` (or as a private function):

```rust
/// Compute the status banner label for a page's frontmatter.
///
/// Returns an empty string if the page is neither draft nor scheduled.
fn build_draft_label(fm: &crate::frontmatter::Frontmatter) -> String {
    let today = chrono::Utc::now().date_naive();
    let is_draft = fm.draft;
    let is_scheduled = fm.publish_date.map_or(false, |d| d > today);

    match (is_draft, is_scheduled) {
        (true, true) => format!("DRAFT | SCHEDULED: {}", fm.publish_date.unwrap()),
        (true, false) => "DRAFT".to_string(),
        (false, true) => format!("SCHEDULED: {}", fm.publish_date.unwrap()),
        (false, false) => String::new(),
    }
}
```

- [ ] **Step 6: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 7: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no new warnings in our files

- [ ] **Step 8: Commit**

```
feat(dev): add draft/scheduled status banner in dev mode
```

---

### Task 4: Write feature documentation

**Files:**
- Create: `docs/draft_pages.md`

- [ ] **Step 1: Write the docs**

```markdown
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
```

- [ ] **Step 2: Commit**

```
docs: add draft and scheduled pages documentation
```

---

### Task 5: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: no new warnings in our files

- [ ] **Step 3: Verify with example site (if available)**

Create a test page with `draft: true` in its frontmatter. Run `cargo run -- build`. Verify the page does NOT appear in `dist/`. Run `cargo run -- dev` and verify the page DOES appear with the DRAFT banner.
