# Audit

Eigen includes a built-in audit system that checks rendered pages for SEO,
performance, accessibility, and best-practice issues. It runs automatically
in dev mode and is available as a standalone CLI command.

Each finding includes a stable check ID, severity, and an actionable fix
with the file to edit and what to change.

## Quick Start

Run `eigen dev`. The audit runs automatically after every rebuild. Visit
`/_audit` in the browser for a styled, filterable report. A floating badge
on every page shows the issue count for that page.

## Dev Mode Endpoints

When the dev server is running, three files are written to `dist/` and
served as static assets:

| Path | Format | Use case |
|------|--------|----------|
| `/_audit` | HTML | Interactive report with severity/category filters |
| `/_audit.json` | JSON | Structured data for scripts and AI tools |
| `/_audit.md` | Markdown | Copy-paste into an AI chat or issue tracker |

These files are regenerated on every rebuild.

## Overlay Badge

In dev mode, a floating badge is injected before `</body>` on every
rendered page. It shows the number of audit findings for that page and
is color-coded by worst severity (red = critical, orange = high,
yellow = medium, green = none).

Clicking the badge expands a panel listing each finding with its severity,
check ID, and message. The panel includes a link to the full `/_audit`
report. Expanded/collapsed state is persisted in `localStorage`.

## CLI: `eigen audit`

Run the audit outside the dev server. By default it builds the site first,
then audits the output.

```bash
# Markdown report to stdout (default)
eigen audit --project .

# JSON report to stdout
eigen audit --project . --format json

# Write report.json and report.md files
eigen audit --project . --output report

# Skip build, audit existing dist/
eigen audit --project . --no-build
```

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--project` / `-p` | `.` | Path to the project root |
| `--format` / `-f` | `markdown` | Output format: `markdown` or `json` |
| `--output` / `-o` | (none) | Write `<path>.json` and `<path>.md` instead of printing to stdout |
| `--no-build` | false | Skip build step, audit the existing `dist/` directory |

## Configuration

Add an `[audit]` table to `site.toml`. The audit runs even when the table
is absent; the table is only needed to ignore specific checks.

```toml
[audit]
ignore = ["seo/twitter-tags", "a11y/color-contrast-hint"]
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `ignore` | `Vec<String>` | `[]` | Check IDs to suppress from the report |

## Check Reference

Every check has a stable slash-delimited ID of the form `category/name`.

### SEO

| ID | Severity | Description |
|----|----------|-------------|
| `seo/sitemap` | critical | No sitemap.xml generated |
| `seo/robots-txt` | high | No robots.txt configured |
| `seo/feed` | low | No Atom feed configured |
| `seo/meta-title` | high | Missing `<title>` tag |
| `seo/meta-description` | high | Missing `<meta name="description">` |
| `seo/canonical` | high | Missing `<link rel="canonical">` |
| `seo/og-tags` | medium | Missing Open Graph tags |
| `seo/twitter-tags` | low | Missing Twitter Card tags |
| `seo/structured-data` | medium | Missing JSON-LD structured data |
| `seo/heading-hierarchy` | medium | Missing or multiple `<h1>` tags |

### Performance

| ID | Severity | Description |
|----|----------|-------------|
| `perf/image-optimization` | high | Image optimization disabled |
| `perf/image-formats` | medium | No modern image formats configured |
| `perf/minification` | high | HTML minification disabled |
| `perf/critical-css` | medium | Critical CSS inlining not enabled |
| `perf/bundling` | medium | CSS/JS bundling not enabled |
| `perf/content-hash` | low | Content hashing not enabled |
| `perf/preload-hints` | medium | Resource hints disabled |
| `perf/render-blocking-scripts` | high | Script in head without defer/async |
| `perf/large-image` | high | Image without srcset |
| `perf/font-preload` | medium | Font file not preloaded |

### Accessibility

| ID | Severity | Description |
|----|----------|-------------|
| `a11y/html-lang` | critical | Missing lang attribute on `<html>` |
| `a11y/viewport-meta` | critical | Missing or misconfigured viewport |
| `a11y/img-alt-text` | high | Image missing alt attribute |
| `a11y/link-text` | medium | Link with empty text |
| `a11y/color-contrast-hint` | low | Reminder to check contrast |

### Best Practices

| ID | Severity | Description |
|----|----------|-------------|
| `bp/base-url` | critical | `base_url` not set properly |
| `bp/favicon` | medium | No favicon found |
| `bp/https-links` | medium | HTTP links found |

## AI Workflow

The audit output is designed to be fed directly to AI tools.

### From the dev server

```bash
# Grab the markdown report and paste into an AI chat
curl http://localhost:3000/_audit.md

# Or the structured JSON
curl http://localhost:3000/_audit.json
```

### From the CLI

```bash
# Pipe JSON directly to an AI tool
eigen audit --format json | ai-tool fix-issues
```

### JSON structure

The JSON output includes `fix.file` and `fix.instruction` for every
finding, giving AI tools the exact file to edit and what to do:

```json
{
  "summary": { "total": 2, "by_severity": { "high": 1, "medium": 1 } },
  "site_findings": [
    {
      "id": "seo/robots-txt",
      "severity": "high",
      "message": "No robots.txt configured",
      "fix": {
        "file": "site.toml",
        "instruction": "Add a [robots] table"
      }
    }
  ],
  "page_findings": {
    "/about/index.html": [
      {
        "id": "seo/meta-description",
        "severity": "medium",
        "message": "Missing meta description",
        "fix": {
          "file": "templates/about.html",
          "instruction": "Add <meta name=\"description\" content=\"...\">"
        }
      }
    ]
  }
}
```

## Architecture

```
src/build/audit/
  mod.rs                 -- AuditReport, run_audit(), core types
  overlay.rs             -- overlay badge injection for dev mode
  checks/
    mod.rs               -- check registry, run_site_checks / run_page_checks
    seo.rs               -- SEO checks (site + page level)
    performance.rs       -- performance checks (site + page level)
    accessibility.rs     -- accessibility checks (site + page level)
    best_practices.rs    -- best-practice checks (site + page level)
  output/
    mod.rs               -- write_all() convenience function
    html.rs              -- styled HTML report with filter UI
    json.rs              -- JSON serialization via serde
    markdown.rs          -- markdown report for AI/human consumption
```

## Build Pipeline Position

```
render pages -> sitemap -> robots.txt -> feeds -> bundling
  -> audit checks -> write _audit.{html,json,md} -> inject overlay badges
```

The audit runs as the final phase after all rendering and optimization.
In dev mode, overlay badges are injected into the HTML files on disk
after the audit report files are written.
