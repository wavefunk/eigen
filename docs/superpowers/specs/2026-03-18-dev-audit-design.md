# Dev Audit: SEO / PageSpeed / Accessibility Checker

## Overview

A built-in audit system for eigen that automatically checks rendered pages and site configuration for SEO, performance, accessibility, and best-practice issues during development. Results are surfaced via a per-page overlay badge, a full site report page, and machine-readable endpoints for AI-driven fixes.

## Goals

- Help developers catch SEO/performance/accessibility gaps **before deployment**
- Provide AI-friendly output so an AI agent can read the audit and apply fixes automatically
- Zero config to start — runs automatically in dev mode with sensible defaults
- Non-intrusive — overlay is minimal, audit never affects production builds

## Non-Goals

- Not a replacement for Lighthouse or full accessibility auditors — this is a fast, built-in first pass
- No runtime/browser-based checks (e.g., actual paint timing, real contrast measurement)
- No auto-fixing — the audit **reports** issues with actionable fix instructions, but doesn't modify files

---

## Architecture

### Approach: Build-Time Audit (Approach A)

The audit runs as the **final build phase** after all rendering, sitemap generation, robots.txt, feeds, bundling, and content hashing are complete. It inspects the fully-built `dist/` directory and the loaded `SiteConfig`.

```
... existing build phases ...
Phase 10: Content hash rewrite
Phase 11: Audit (dev mode + `eigen audit` CLI)
  1. Run site-level checks (config inspection)
  2. Walk dist/**/*.html, run page-level checks on each
  3. Filter out ignored check IDs from [audit] config
  4. Collect all findings into AuditReport
  5. Write _audit.html, _audit.json, _audit.md to dist/
  6. Inject per-page overlay badge into each HTML file (dev mode only)
```

### CLI Integration

Two entry points invoke the same audit module:

1. **`eigen dev`** — audit runs automatically after each rebuild. Serves `/_audit`, `/_audit.json`, `/_audit.md` routes. Injects overlay badge into every page.
2. **`eigen audit`** — standalone CLI command for CI/pre-deploy/AI workflows:
   - `eigen audit --project .` — prints markdown report to stdout
   - `eigen audit --project . --format json` — prints JSON to stdout
   - `eigen audit --project . --output audit-report` — writes `audit-report.json` + `audit-report.md` to disk

The audit module is shared between both entry points. The `eigen audit` command performs a full build first (or reads existing `dist/` if `--no-build` flag is passed), then runs the audit phase.

#### CLI Definition

```rust
/// Run SEO, performance, and accessibility audit
Audit {
    /// Path to the project root (default: current directory)
    #[arg(short, long, default_value = ".")]
    project: PathBuf,

    /// Output format: "markdown" (default) or "json"
    #[arg(short, long, default_value = "markdown")]
    format: String,

    /// Write report files to this path prefix (e.g., "audit-report" writes audit-report.json + audit-report.md)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Skip building and audit existing dist/ directory
    #[arg(long)]
    no_build: bool,
}
```

---

## Module Structure

```
src/build/audit/
  mod.rs          — public API: run_audit(), AuditReport, types
  checks/
    mod.rs        — check registry, CheckFn trait
    seo.rs        — SEO checks
    performance.rs — performance checks
    accessibility.rs — accessibility checks
    best_practices.rs — best practices checks
  output/
    mod.rs        — output format dispatch
    html.rs       — _audit.html renderer
    json.rs       — _audit.json renderer
    markdown.rs   — _audit.md renderer
  overlay.rs      — per-page overlay badge HTML/CSS/JS generation
```

---

## Core Types

```rust
pub struct AuditReport {
    pub site_findings: Vec<Finding>,
    pub page_findings: BTreeMap<String, Vec<Finding>>,  // page path -> findings
    pub summary: AuditSummary,
}

pub struct Finding {
    pub id: &'static str,        // "seo/meta-description"
    pub category: Category,
    pub severity: Severity,
    pub scope: Scope,
    pub message: String,         // human-readable issue description
    pub fix: Fix,                // actionable fix info
}

/// Derives: Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize
pub enum Category {
    Seo,
    Performance,
    Accessibility,
    BestPractices,
}

/// Derives: Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize
/// Ordering: Critical > High > Medium > Low (Critical sorts first in BTreeMap)
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

pub enum Scope {
    Site,
    Page,
}

pub struct Fix {
    pub file: String,            // "site.toml" or "templates/about.html"
    pub instruction: String,     // "Add description = \"...\" under [site.seo]"
}

pub struct AuditSummary {
    pub total: usize,
    pub by_severity: BTreeMap<Severity, usize>,
    pub by_category: BTreeMap<Category, usize>,
}
```

### `Fix.file` Semantics

For **site-level checks**, `Fix.file` points to `site.toml` since these checks inspect config.

For **page-level checks**, `Fix.file` points to the source template path (e.g., `templates/about.html`). The audit receives the list of rendered pages from the build pipeline, which includes the source template path alongside the output path. This avoids needing a separate dist-to-source mapping — the build already tracks this.

### Config Type

```rust
/// Optional audit configuration in site.toml.
/// Added to SiteConfig as: `pub audit: Option<AuditConfig>`
/// with `#[serde(default)]`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuditConfig {
    /// Check IDs to suppress (e.g., ["seo/twitter-tags", "a11y/color-contrast-hint"])
    #[serde(default)]
    pub ignore: Vec<String>,
}
```

---

## Check Registry

Checks are implemented as functions registered in a central registry. Each check function receives context and returns `Vec<Finding>`.

### Site-Level Check Signature

```rust
fn check(config: &SiteConfig, dist_path: &Path) -> Vec<Finding>
```

Receives the full site config and path to `dist/`. Used for config-level checks and checks that scan across all pages (e.g., "no sitemap exists").

### Page-Level Check Signature

```rust
fn check(html: &str, page_path: &str, config: &SiteConfig) -> Vec<Finding>
```

Receives the rendered HTML content, the page's output path (e.g., `about.html`), and config for context. Called once per HTML file in `dist/`.

### Adding New Checks

Add a function to the appropriate category module, register it in `checks/mod.rs`. No other changes needed.

---

## Initial Check List

### SEO (`seo/`)

| ID | Severity | Scope | Description |
|----|----------|-------|-------------|
| `seo/meta-description` | high | page | Missing `<meta name="description">` |
| `seo/meta-title` | high | page | Missing `<title>` tag |
| `seo/og-tags` | medium | page | Missing Open Graph tags (title, description, image) |
| `seo/twitter-tags` | low | page | Missing Twitter Card tags |
| `seo/canonical` | high | page | Missing `<link rel="canonical">` |
| `seo/structured-data` | medium | page | Missing JSON-LD structured data |
| `seo/sitemap` | critical | site | No sitemap.xml generated |
| `seo/robots-txt` | high | site | No robots.txt configured |
| `seo/heading-hierarchy` | medium | page | Missing `<h1>` or multiple `<h1>` tags |
| `seo/feed` | low | site | No Atom feed configured |

### Performance (`perf/`)

| ID | Severity | Scope | Description |
|----|----------|-------|-------------|
| `perf/image-optimization` | high | site | Image optimization disabled |
| `perf/image-formats` | medium | site | Modern formats (webp/avif) not configured |
| `perf/minification` | high | site | HTML minification disabled |
| `perf/critical-css` | medium | site | Critical CSS inlining not enabled |
| `perf/bundling` | medium | site | CSS/JS bundling not enabled |
| `perf/content-hash` | low | site | Content hashing (cache busting) not enabled |
| `perf/preload-hints` | medium | site | Resource hints disabled |
| `perf/large-image` | high | page | Image without responsive srcset |
| `perf/render-blocking-scripts` | high | page | `<script>` in `<head>` without `defer`/`async` |
| `perf/font-preload` | medium | page | Font files referenced but not preloaded |

### Accessibility (`a11y/`)

| ID | Severity | Scope | Description |
|----|----------|-------|-------------|
| `a11y/img-alt-text` | high | page | `<img>` missing `alt` attribute |
| `a11y/html-lang` | critical | page | `<html>` missing `lang` attribute |
| `a11y/viewport-meta` | critical | page | Missing or misconfigured `<meta name="viewport">` (covers both presence and mobile-friendliness) |
| `a11y/link-text` | medium | page | Links with empty or generic text |
| `a11y/color-contrast-hint` | low | site | Advisory reminder to verify color contrast (not an automated check — eigen cannot measure rendered contrast without a browser) |

### Best Practices (`bp/`)

| ID | Severity | Scope | Description |
|----|----------|-------|-------------|
| `bp/https-links` | medium | page | Page contains `http://` links |
| `bp/favicon` | medium | site | No favicon detected |
| `bp/base-url` | critical | site | `site.base_url` not set |

Note: viewport meta is handled by `a11y/viewport-meta` which covers both presence and mobile configuration. No separate `bp/mobile-viewport` check.

---

## Configuration

### Ignoring Checks

Users can suppress specific check IDs in `site.toml`:

```toml
[audit]
ignore = ["seo/twitter-tags", "a11y/color-contrast-hint"]
```

The `[audit]` table is optional. When absent, all checks run. The `ignore` list uses the stable check IDs.

---

## Output Formats

### `/_audit.html` (Human-Readable Report Page)

A self-contained HTML page (no external dependencies) showing:

- Summary bar: total issues by severity (critical: N, high: N, etc.)
- Grouped by category (SEO, Performance, Accessibility, Best Practices)
- Within each category, sorted by severity (critical first)
- Each finding shows: ID, severity badge, message, fix instruction
- Page-level findings grouped by page path
- Filterable by category and severity via simple JS toggles

Styled with inline CSS, scoped to avoid conflicts. Dark/light theme based on `prefers-color-scheme`.

### `/_audit.json` (AI/Programmatic)

```json
{
  "summary": {
    "total": 12,
    "by_severity": { "critical": 1, "high": 4, "medium": 5, "low": 2 },
    "by_category": { "seo": 5, "performance": 3, "accessibility": 2, "best_practices": 2 }
  },
  "site_findings": [
    {
      "id": "seo/sitemap",
      "category": "seo",
      "severity": "critical",
      "message": "No sitemap.xml generated. Search engines use sitemaps to discover pages.",
      "fix": {
        "file": "site.toml",
        "instruction": "Sitemap is generated automatically when pages are rendered. Verify the build completes successfully."
      }
    }
  ],
  "page_findings": {
    "index.html": [
      {
        "id": "seo/meta-description",
        "category": "seo",
        "severity": "high",
        "message": "Missing <meta name=\"description\"> tag.",
        "fix": {
          "file": "site.toml",
          "instruction": "Add description under [site.seo]: description = \"Your site description\""
        }
      }
    ]
  }
}
```

### `/_audit.md` (Markdown for AI Chat / Human Reading)

```markdown
# Eigen Audit Report

## Summary
- **Critical:** 1
- **High:** 4
- **Medium:** 5
- **Low:** 2

## Site-Wide Issues

### [CRITICAL] seo/sitemap
No sitemap.xml generated. Search engines use sitemaps to discover pages.
**Fix:** In `site.toml`, verify the build completes successfully. Sitemap is generated automatically.

## Page Issues

### index.html

#### [HIGH] seo/meta-description
Missing <meta name="description"> tag.
**Fix:** In `site.toml`, add `description = "Your site description"` under `[site.seo]`.
```

---

## Dev Overlay Badge

### Injection

Injected via the same mechanism as the live-reload script (`src/dev/inject.rs`), appended before `</body>` in dev builds only. The page-specific findings are embedded as a JSON blob in the injected `<script>` tag — no async fetch needed since they're known at build time.

### Behavior

- **Badge:** Small floating element in the bottom-right corner
  - Shows issue count and color-coded by worst severity (red = critical, orange = high, yellow = medium, green = all clear)
  - Example: `"4 issues"` with orange background
- **Expanded panel:** Clicking the badge opens a panel showing page-specific findings for the current page
  - Grouped by category, sorted by severity
  - Each finding shows severity badge, message, fix instruction
  - Link to `/_audit` for the full site report
- **State:** Collapsed/expanded state remembered via `localStorage`
- **Scoping:** All CSS uses a unique prefix (`__eigen_audit_`) to avoid conflicts with site styles
- **Not in production:** The overlay is never injected during `eigen build`

### Styling

Minimal, non-intrusive. Uses `position: fixed`, high `z-index`, `prefers-color-scheme` for dark/light. Shadow DOM or prefixed classes to isolate from site CSS.

---

## Build Pipeline Integration

### In `eigen dev` (dev mode)

After the existing final build phase:
1. `audit::run_audit(config, dist_path, rendered_pages)` → `AuditReport`
2. `audit::output::write_html(report, dist_path)` → `dist/_audit.html`
3. `audit::output::write_json(report, dist_path)` → `dist/_audit.json`
4. `audit::output::write_markdown(report, dist_path)` → `dist/_audit.md`
5. `audit::overlay::inject_badges(report, dist_path, rendered_pages)` → modifies each page HTML file in `dist/`

The overlay injection happens **after** the audit report files are written but uses the same `AuditReport` data. It must run after minification (since it modifies HTML) — this is already the case since audit is the last phase.

**Overlay injection** re-reads each page HTML from disk, appends the overlay script before `</body>` (same approach as `inject_reload_script`), and writes it back. This is a second pass over page HTML files but is acceptable in dev mode. The overlay is injected **after** the reload script — order does not matter since both are independent `<script>` blocks before `</body>`.

**Page source:** The audit uses the `rendered_pages` list from the build pipeline (not a `dist/` directory walk) to know which HTML files are actual pages vs. generated files (sitemap, robots.txt, audit reports). This avoids scanning `_audit.*` files, `_fragments/`, or other non-page HTML.

**Fragment exclusion:** Overlay badges are NOT injected into fragment files (`_fragments/`). Fragments are partial HTML without `<html>`/`<body>` structure. Page-level checks also skip fragments.

**Build failure handling:** If the build fails (e.g., template error), the audit phase is skipped entirely. The `DevBuildState` already tracks build success/failure — audit only runs on successful builds.

**Sync execution:** `run_audit` is synchronous (not async). It runs on the same blocking OS thread as the dev build (the build uses `reqwest::blocking::Client` and cannot run on the async runtime). All audit operations (file I/O, HTML parsing) are naturally sync.

**File watcher safety:** Writing `_audit.*` files to `dist/` does not trigger the file watcher, since the watcher monitors `templates/`, `_data/`, `static/`, and `site.toml` — not `dist/`.

### In `eigen audit` (CLI command)

1. Build the site (full `eigen build` pipeline) OR read existing `dist/` if `--no-build` flag
2. `audit::run_audit(config, dist_path, rendered_pages)` → `AuditReport`
3. Output based on flags:
   - Default: print markdown to stdout
   - `--format json`: print JSON to stdout
   - `--output <path>`: write `<path>.json` + `<path>.md` to disk

### In `eigen build` (production)

Audit does **not** run. No `_audit.*` files are written. No overlay is injected.

---

## HTML Parsing for Page-Level Checks

Page-level checks use `lol_html` (already a dependency) for streaming HTML inspection, consistent with the rest of the codebase (`seo.rs`, `json_ld.rs`, `hints/`, `bundling/`, `critical_css/`). Each page-level check function runs a single `lol_html::rewrite_str` pass over the HTML using element and text content handlers to detect issues.

For checks that need to track state across multiple elements (e.g., heading hierarchy counting `h1` through `h6`), use stateful `lol_html` handlers that accumulate findings during the streaming pass.

All page-level checks for a given page should be combined into a single `lol_html::rewrite_str` call with multiple handlers to avoid redundant parsing passes. The check registry collects handlers from each check module and runs them together.

### Dev Mode vs. Production: False Positive Prevention

In `eigen dev`, several post-render phases are skipped (image optimization, critical CSS, hints, minification, bundling, content hashing). Page-level HTML checks that inspect features only present after these phases would produce false positives.

**Strategy:** Checks that inspect HTML for features applied by eigen's own build phases are classified as **config-only** checks. They inspect `SiteConfig` fields (site-level scope), not the HTML output. This applies to:
- `perf/image-optimization`, `perf/image-formats` — check `config.assets.images.optimize`
- `perf/minification` — check `config.build.minify`
- `perf/critical-css` — check `config.build.critical_css.enabled`
- `perf/bundling` — check `config.build.bundling.enabled`
- `perf/content-hash` — check `config.build.content_hash.enabled`
- `perf/preload-hints` — check `config.build.hints.enabled`

Checks that inspect HTML for things the **user** controls (not eigen's pipeline) are safe to run on dev output:
- `perf/render-blocking-scripts` — user-authored `<script>` tags
- `perf/large-image` — checks for `<img>` without `srcset` when image optimization is **disabled** (if enabled, eigen adds srcset in production, so this check is suppressed)
- `perf/font-preload` — checks for font `<link>` tags without preload (only fires when hints are disabled)

---

## Known Limitations

- **No browser rendering:** Checks inspect static HTML only. Contrast ratios, actual paint timing, and JS-dependent content cannot be verified.
- **HTML entities in link text:** Checks for generic link text (e.g., "click here") match plain text, not HTML entities like `&#8230;`. Accepted for a first-pass tool.
- **Dev mode skips some build phases:** Image optimization, critical CSS, hints, minification, bundling, and content hashing don't run in dev mode. The audit handles this by checking config flags (site-level) rather than HTML output for these features. See "Dev Mode vs. Production" section.

---

## Future Extensions

- **Custom checks via plugins** — allow the plugin system to register custom audit checks
- **Severity overrides** — let users change severity of specific checks in `[audit]` config
- **Baseline file** — `eigen audit --baseline` to snapshot current issues, then only report new ones
- **Watch mode for `eigen audit`** — re-run on file changes (useful for CI-like local workflows)

These are explicitly **not in scope** for the initial implementation.
