# Eigen
[![Website](https://github.com/wavefunk/eigen/actions/workflows/website.yml/badge.svg)](https://github.com/wavefunk/eigen/actions/workflows/website.yml)

A fast, opinionated static site generator with first-class HTMX support. Eigen generates full HTML pages **and** HTML fragments side-by-side, enabling seamless HTMX-powered partial page loads out of the box.

Built in Rust. Uses [minijinja](https://github.com/mitsuhiko/minijinja) for templates, YAML/JSON for data, and [Axum](https://github.com/tokio-rs/axum) for the dev server.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Installation](#installation)
- [CLI Commands](#cli-commands)
  - [`eigen init`](#eigen-init)
  - [`eigen build`](#eigen-build)
  - [`eigen dev`](#eigen-dev)
- [Project Structure](#project-structure)
- [Configuration (`site.toml`)](#configuration-sitetoml)
  - [Site Metadata](#site-metadata)
  - [Build Options](#build-options)
  - [Asset Localization](#asset-localization)
  - [Data Sources](#data-sources)
  - [Environment Variable Interpolation](#environment-variable-interpolation)
- [Templates](#templates)
  - [Layouts and Partials](#layouts-and-partials)
  - [Static Pages](#static-pages)
  - [Dynamic Pages](#dynamic-pages)
  - [Template Context](#template-context)
- [Frontmatter](#frontmatter)
  - [Data Queries](#data-queries)
  - [Dynamic Page Frontmatter](#dynamic-page-frontmatter)
  - [Nested Query Interpolation](#nested-query-interpolation)
  - [Fragment Block Overrides](#fragment-block-overrides)
- [Data Layer](#data-layer)
  - [Global Data (`_data/`)](#global-data-_data)
  - [Local File Queries](#local-file-queries)
  - [Remote API Queries](#remote-api-queries)
  - [Data Transforms](#data-transforms)
- [Template Filters](#template-filters)
- [Template Functions](#template-functions)
- [HTMX & Fragments](#htmx--fragments)
  - [How Fragments Work](#how-fragments-work)
  - [The `link_to` Function](#the-link_to-function)
  - [Fragment Output Paths](#fragment-output-paths)
- [Asset Localization](#asset-localization-1)
- [Dev Server](#dev-server)
  - [Live Reload](#live-reload)
  - [API Proxy](#api-proxy)
  - [Smart Rebuild](#smart-rebuild)
- [Example Site](#example-site)

---

## Quick Start

```bash
# Create a new project
eigen init my-site

# Build the site
cd my-site
eigen build

# Or start the dev server with live reload
eigen dev
```

Your built site will be in `dist/`. Open `dist/index.html` in a browser or use `eigen dev` for a proper development experience.

---

## Installation

Eigen is a Rust project. Build it from source:

```bash
# Clone the repository
git clone <repo-url>
cd eigen

# Build in release mode
cargo build --release

# The binary is at target/release/eigen
```

If you use Nix, a dev shell is provided via `flake.nix`:

```bash
nix develop
cargo build
```

---

## CLI Commands

### `eigen init`

Scaffold a new Eigen project:

```bash
eigen init my-site
```

Creates a directory `my-site/` with a complete, buildable starter site:

```
my-site/
├── site.toml                  # Site configuration
├── templates/
│   ├── _base.html             # Base layout with HTMX CDN
│   ├── _partials/
│   │   └── nav.html           # Navigation partial
│   ├── index.html             # Home page
│   └── about.html             # About page
├── _data/
│   └── nav.yaml               # Sample navigation data
├── static/
│   └── css/
│       └── style.css          # Minimal stylesheet
└── .gitignore
```

The scaffolded site is immediately buildable: `cd my-site && eigen build`.

### `eigen build`

Build the site into the `dist/` directory:

```bash
# Build the current directory
eigen build

# Build a specific project
eigen build --project /path/to/project

# Verbose output (shows each page rendered, each data fetch)
eigen build --verbose

# Quiet mode (errors only)
eigen build --quiet
```

The build process:

1. Loads and validates `site.toml`
2. Loads global data from `_data/`
3. Discovers and classifies templates
4. Cleans and prepares `dist/`
5. Copies `static/` → `dist/`
6. Renders all static and dynamic pages
7. Extracts and writes HTML fragments (if enabled)
8. Localizes remote assets (if enabled)
9. Generates `dist/sitemap.xml`

### `eigen dev`

Start the development server with live reload:

```bash
# Default: port 3000
eigen dev

# Custom port
eigen dev --port 8080

# Specific project directory
eigen dev --project /path/to/project
```

The dev server:
- Serves files from `dist/` at `http://127.0.0.1:3000`
- Watches `templates/`, `_data/`, `static/`, and `site.toml` for changes
- Automatically rebuilds on changes (debounced at 200ms)
- Injects a live-reload script that refreshes your browser automatically
- Provides API proxy routes for configured data sources

---

## Project Structure

```
my-site/
├── site.toml              # Site configuration
├── templates/             # Page templates (Jinja2/minijinja)
│   ├── _base.html         # Layout (underscore prefix = not a page)
│   ├── _partials/         # Reusable template partials
│   │   └── nav.html
│   ├── index.html         # Static page → dist/index.html
│   ├── about.html         # Static page → dist/about.html
│   └── posts/
│       ├── index.html     # Static page → dist/posts/index.html
│       └── [post].html    # Dynamic page → dist/posts/{slug}.html
├── _data/                 # Global data files (YAML/JSON)
│   └── nav.yaml
├── static/                # Static assets (copied to dist/ as-is)
│   └── css/
│       └── style.css
└── dist/                  # Build output (generated, gitignored)
    ├── _fragments/        # HTML fragments for HTMX
    ├── css/
    ├── posts/
    ├── index.html
    ├── about.html
    └── sitemap.xml
```

### Key Conventions

| Convention | Meaning |
|---|---|
| `_` prefix on files/dirs in `templates/` | Layout or partial — not rendered as a standalone page |
| `[name].html` filename | Dynamic template — generates one page per collection item |
| `_data/` directory | Global data available to all templates |
| `static/` directory | Copied verbatim to `dist/` |
| `dist/` directory | Build output (cleaned on each build) |

---

## Configuration (`site.toml`)

All configuration lives in a single `site.toml` file at the project root.

### Site Metadata

```toml
[site]
name = "My Site"
base_url = "https://example.com"
```

| Field | Required | Description |
|---|---|---|
| `name` | Yes | Site name, available as `{{ site.name }}` in templates |
| `base_url` | Yes | Canonical base URL, used in sitemap and the `absolute` filter |

### Build Options

```toml
[build]
fragments = true              # Generate HTML fragments (default: true)
fragment_dir = "_fragments"   # Fragment output directory (default: "_fragments")
content_block = "content"     # Default block name for fragments (default: "content")
```

| Field | Default | Description |
|---|---|---|
| `fragments` | `true` | Whether to generate HTML fragments alongside full pages |
| `fragment_dir` | `"_fragments"` | Subdirectory inside `dist/` for fragment files |
| `content_block` | `"content"` | The default template block to extract as a fragment |

### Asset Localization

```toml
[assets]
localize = true                          # Download remote assets to dist/assets/
cdn_skip_hosts = ["mycdn.internal.com"]  # Additional CDN hosts to skip
cdn_allow_hosts = ["cdn.jsdelivr.net"]   # Override default skips for these hosts
```

When enabled, remote URLs in `<img>`, `<video>`, `<source>`, and `<audio>` `src` attributes — as well as CSS `background-image: url(...)` — are downloaded to `dist/assets/` and rewritten to local paths.

Known CDN hosts (jsdelivr, cdnjs, unpkg, Google Fonts, etc.) are skipped by default. You can customize this with `cdn_skip_hosts` and `cdn_allow_hosts`.

### Data Sources

Define external API endpoints that your templates can fetch data from:

```toml
[sources.blog_api]
url = "https://api.example.com"
headers = { Authorization = "Bearer ${API_TOKEN}" }

[sources.cms]
url = "https://cms.example.com/api"
```

Each source is referenced by name in template frontmatter (e.g., `source: blog_api`). During builds, Eigen performs HTTP GET requests to `source.url + query.path`, injects configured headers, and caches responses.

### Environment Variable Interpolation

Any string value in `site.toml` can reference environment variables using `${VAR_NAME}` syntax:

```toml
[sources.api]
url = "https://api.example.com"
headers = { Authorization = "Bearer ${API_TOKEN}" }
```

If a referenced variable is not set, the build fails with a clear error message.

---

## Templates

Eigen uses [minijinja](https://docs.rs/minijinja) (a Jinja2-compatible engine) for templates. All templates live in the `templates/` directory and must be `.html` files.

### Layouts and Partials

Files and directories starting with `_` are **not** rendered as standalone pages. They serve as layouts and partials:

**`templates/_base.html`** — Base layout:
```html
<!DOCTYPE html>
<html lang="en">
<head>
    <title>{% block title %}{{ site.name }}{% endblock %}</title>
    <link rel="stylesheet" href="{{ asset('css/style.css') }}">
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
</head>
<body>
    {% include "_partials/nav.html" %}
    <main id="content">
        {% block content %}{% endblock %}
    </main>
    <footer>&copy; {{ current_year() }} {{ site.name }}</footer>
</body>
</html>
```

**`templates/_partials/nav.html`** — Reusable partial:
```html
<nav>
    <ul>
        {% for item in nav %}
        <li><a {{ link_to(item.url) }}>{{ item.label }}</a></li>
        {% endfor %}
    </ul>
</nav>
```

### Static Pages

Any `.html` file in `templates/` that doesn't have a bracketed name is a **static page** — it produces exactly one output file.

**`templates/about.html`** → `dist/about.html`:
```html
---
data:
  nav:
    file: "nav.yaml"
---
{% extends "_base.html" %}

{% block title %}About — {{ site.name }}{% endblock %}

{% block content %}
<h1>About</h1>
<p>Welcome to my site.</p>
{% endblock %}
```

Output path mirrors the template path:
- `templates/index.html` → `dist/index.html`
- `templates/about.html` → `dist/about.html`
- `templates/docs/guide.html` → `dist/docs/guide.html`

### Dynamic Pages

Templates with `[name].html` filenames are **dynamic pages** — they generate one page per item in a collection.

**`templates/posts/[post].html`** → `dist/posts/{slug}.html` for each item:
```html
---
collection:
  source: blog_api
  path: /posts
slug_field: id
item_as: post
data:
  nav:
    file: "nav.yaml"
---
{% extends "_base.html" %}

{% block title %}{{ post.title }} — {{ site.name }}{% endblock %}

{% block content %}
<article>
    <h1>{{ post.title }}</h1>
    <p>{{ post.body }}</p>
</article>
{% endblock %}
```

- The parameter name (`post` from `[post].html`) is extracted from the filename
- The `collection` frontmatter query provides the list of items
- Each item's slug (from `slug_field`, default `"slug"`) becomes the output filename
- The item is exposed in the template as the `item_as` variable (default `"item"`)
- Slugs are automatically sanitized to URL-safe strings

### Template Context

Every template has access to these variables:

| Variable | Description |
|---|---|
| `site.name` | Site name from `site.toml` |
| `site.base_url` | Base URL from `site.toml` |
| `page.current_url` | Full URL path (e.g., `/about.html`) |
| `page.current_path` | Path relative to `dist/` (e.g., `about.html`) |
| `page.base_url` | Base URL (same as `site.base_url`) |
| `page.build_time` | ISO 8601 build timestamp |
| *Global data keys* | Each file in `_data/` is available by name |
| *Frontmatter data keys* | Each entry in frontmatter `data:` is available by name |
| *Dynamic item* | On dynamic pages, the current item (named by `item_as`) |

If a frontmatter `data` key has the same name as a `_data/` global key, the frontmatter data takes precedence and a warning is logged.

---

## Frontmatter

Templates can optionally start with a YAML frontmatter block delimited by `---`:

```html
---
data:
  nav:
    file: "nav.yaml"
  posts:
    source: blog_api
    path: /posts
    sort: "-date"
    limit: 5
---
{% extends "_base.html" %}
...
```

### Data Queries

The `data` field maps names to **data queries**. Each query can reference a local file or a remote API source:

```yaml
data:
  # Local file from _data/
  nav:
    file: "nav.yaml"

  # Remote API
  recent_posts:
    source: blog_api        # Must match a [sources.*] in site.toml
    path: /posts             # Appended to the source URL
    root: data.posts         # Dot path to extract from response
    sort: "-date"            # Sort by field (prefix with - for descending)
    limit: 5                 # Maximum items to return
    filter:                  # Key-value filters (keep items where item[key] == value)
      status: "published"
```

**Query fields:**

| Field | Description |
|---|---|
| `file` | Path to a file in `_data/` (`.yaml`, `.yml`, or `.json`) |
| `source` | Name of a source from `site.toml` `[sources.*]` |
| `path` | URL path appended to the source's base URL |
| `root` | Dot-separated path into the response to extract data from |
| `sort` | Sort specification: `"field"` (ascending) or `"-field"` (descending) |
| `limit` | Maximum number of items to return |
| `filter` | Key-value pairs — only items where `item[key] == value` are kept |

Transforms are applied in order: **filter → sort → limit**.

### Dynamic Page Frontmatter

Dynamic pages (`[name].html`) require a `collection` query and support additional fields:

```yaml
collection:
  source: blog_api
  path: /posts
slug_field: slug          # Field to use as the URL slug (default: "slug")
item_as: post             # Variable name for each item (default: "item")
data:
  author:
    source: blog_api
    path: /authors
    filter:
      id: "{{ post.author_id }}"
fragment_blocks:
  - content
  - sidebar
```

| Field | Default | Description |
|---|---|---|
| `collection` | *(required)* | Data query that returns the array of items |
| `slug_field` | `"slug"` | Which field on each item provides the URL slug |
| `item_as` | `"item"` | Template variable name for the current item |
| `data` | `{}` | Additional data queries (supports interpolation) |
| `fragment_blocks` | *(all blocks)* | Which blocks to extract as fragments |

### Nested Query Interpolation

In dynamic pages, data query `filter` values and `path` can reference the current item using `{{ item_as.field }}` syntax:

```yaml
item_as: post
data:
  author:
    source: blog_api
    path: /users/{{ post.author_id }}
  comments:
    source: blog_api
    path: /comments
    filter:
      postId: "{{ post.id }}"
```

Interpolation is **one level deep** — if a resolved query still contains `{{ }}` patterns, Eigen returns an error.

### Fragment Block Overrides

By default, Eigen extracts **all** `{% block %}` definitions as fragments. Use `fragment_blocks` to limit which blocks are extracted:

```yaml
fragment_blocks:
  - content
  - sidebar
```

---

## Data Layer

### Global Data (`_data/`)

Place `.yaml`, `.yml`, or `.json` files in the `_data/` directory. They're automatically loaded and made available as top-level template variables:

```
_data/nav.yaml          → {{ nav }}
_data/settings.json     → {{ settings }}
_data/footer/links.yaml → {{ footer_links }}
```

Nested directories are flattened with underscores: `_data/a/b/c.yaml` → key `a_b_c`.

### Local File Queries

Frontmatter `data` queries can reference files in `_data/`:

```yaml
data:
  nav:
    file: "nav.yaml"
```

The file is loaded once and cached for the entire build.

### Remote API Queries

Queries with `source` fetch data from configured APIs:

```yaml
data:
  posts:
    source: blog_api
    path: /posts
    root: data.posts
```

The full URL is constructed as `source.url + path`. Configured headers are sent with every request. Responses are parsed as JSON and cached by URL for the duration of the build.

### Data Transforms

All three transforms can be applied to any data query (local or remote):

**Filter** — keep items matching all key-value pairs:
```yaml
filter:
  status: "published"
  category: "tutorials"
```

**Sort** — order items by a field:
```yaml
sort: "date"       # ascending
sort: "-date"      # descending
```

**Limit** — truncate to N items:
```yaml
limit: 10
```

---

## Template Filters

All filters are available via the standard Jinja2 pipe syntax: `{{ value | filter_name }}`.

| Filter | Usage | Description |
|---|---|---|
| `markdown` | `{{ content \| markdown }}` | Render Markdown to HTML (tables, strikethrough, footnotes) |
| `date` | `{{ post.date \| date("%B %d, %Y") }}` | Parse and reformat a date string |
| `slugify` | `{{ title \| slugify }}` | Convert to a URL-friendly slug |
| `absolute` | `{{ path \| absolute }}` | Prepend `base_url` to make a full URL |
| `truncate` | `{{ text \| truncate(100) }}` | Truncate at word boundary, append `...` |
| `sort_by` | `{{ items \| sort_by("name") }}` | Sort array of objects by key (prefix `-` for desc) |
| `group_by` | `{{ items \| group_by("category") }}` | Group array by key → map of arrays |
| `json` | `{{ data \| json }}` | Serialize to pretty-printed JSON |

**`date` filter** — Accepts common input formats:
- `2024-01-15` (ISO date)
- `2024-01-15T14:30:00` (ISO datetime)
- `2024-01-15 14:30:00`
- RFC 3339 / RFC 2822

**`sort_by` filter** — Descending order with `-` prefix:
```html
{% for post in posts | sort_by("-date") %}
  {{ post.title }}
{% endfor %}
```

**`group_by` filter**:
```html
{% set groups = posts | group_by("category") %}
{% for key in groups | list %}
  <h2>{{ key }}</h2>
  {% for post in groups[key] %}
    <p>{{ post.title }}</p>
  {% endfor %}
{% endfor %}
```

---

## Template Functions

| Function | Usage | Description |
|---|---|---|
| `link_to(path, target?, block?)` | `<a {{ link_to("/about.html") }}>` | Generate HTMX-compatible link attributes |
| `current_year()` | `{{ current_year() }}` | Current year (e.g., `2025`) |
| `asset(path)` | `{{ asset("css/style.css") }}` | Normalize static asset path (ensures leading `/`) |

See the [HTMX & Fragments](#htmx--fragments) section for details on `link_to`.

---

## HTMX & Fragments

Eigen's headline feature: every page is rendered as both a **full HTML page** and a standalone **HTML fragment**. This enables HTMX to swap just the content area on navigation, avoiding full page reloads.

### How Fragments Work

1. **During build**, Eigen injects invisible HTML comment markers around every `{% block %}` in your templates
2. **After rendering**, it extracts the content between the markers as fragments
3. **The markers are stripped** from the full page output — they never appear in served HTML
4. **Fragments are written** to `dist/_fragments/` mirroring the page structure

For a page at `dist/about.html` with this template:
```html
{% extends "_base.html" %}
{% block content %}
<h1>About</h1>
<p>Content here.</p>
{% endblock %}
```

Eigen produces:
- `dist/about.html` — Full HTML document (with `<html>`, `<head>`, etc.)
- `dist/_fragments/about.html` — Just the content block: `<h1>About</h1><p>Content here.</p>`

### The `link_to` Function

Use `link_to()` in templates to generate HTMX-powered navigation links:

```html
<a {{ link_to("/about.html") }}>About</a>
```

This renders to:
```html
<a href="/about.html" hx-get="/_fragments/about.html" hx-target="#content" hx-push-url="/about.html">About</a>
```

**Parameters:**

| Parameter | Default | Description |
|---|---|---|
| `path` | *(required)* | The page URL |
| `target` | `"#content"` | CSS selector for `hx-target` |
| `block` | `"content"` | Which fragment block to load |

**Examples:**
```html
<!-- Default: loads content fragment into #content -->
<a {{ link_to("/about.html") }}>About</a>

<!-- Custom target element -->
<a {{ link_to("/about.html", "#main") }}>About</a>

<!-- Load a specific block fragment -->
<a {{ link_to("/about.html", "#sidebar", "sidebar") }}>About Sidebar</a>
```

When fragments are disabled in `site.toml` (`fragments = false`), `link_to` renders a plain `href` attribute with no HTMX attributes.

### Fragment Output Paths

| Page Path | Block | Fragment Path |
|---|---|---|
| `/about.html` | `content` (default) | `/_fragments/about.html` |
| `/posts/hello.html` | `content` (default) | `/_fragments/posts/hello.html` |
| `/about.html` | `sidebar` | `/_fragments/about/sidebar.html` |

The default `content` block mirrors the page path. Non-default blocks get a subdirectory.

---

## Asset Localization

When `[assets] localize = true` (the default), Eigen downloads remote images, videos, and audio referenced in your rendered HTML and saves them locally:

**What gets localized:**
- `<img src="https://...">`
- `<video src="https://...">`
- `<source src="https://...">`
- `<audio src="https://...">`
- `background-image: url(https://...)` in CSS

**What gets skipped:**
- Relative URLs
- URLs already under `/assets/`
- Known CDN hosts (jsdelivr, cdnjs, unpkg, Google Fonts, Font Awesome, etc.)
- Hosts in your `cdn_skip_hosts` list

**Override CDN skipping** with `cdn_allow_hosts`:
```toml
[assets]
cdn_allow_hosts = ["cdn.jsdelivr.net"]  # Force download from this CDN
```

Downloaded assets are cached in `.eigen_cache/` and copied to `dist/assets/`.

---

## Dev Server

### Live Reload

During `eigen dev`, a small script is injected before `</body>` in every page:

```html
<script>
  const es = new EventSource("/_reload");
  es.addEventListener("reload", () => window.location.reload());
  es.onerror = () => setTimeout(() => window.location.reload(), 1000);
</script>
```

This connects to the `/_reload` SSE (Server-Sent Events) endpoint. When a rebuild completes, all connected browsers automatically refresh. The script is **never** injected during `eigen build`.

### API Proxy

For each `[sources.*]` in `site.toml`, the dev server mounts a reverse proxy:

```
/_proxy/{source_name}/* → source.url/*
```

For example, with:
```toml
[sources.blog_api]
url = "https://api.example.com"
headers = { Authorization = "Bearer token123" }
```

A request to `http://localhost:3000/_proxy/blog_api/posts` is proxied to `https://api.example.com/posts` with the configured headers. This eliminates CORS issues during local development.

### Smart Rebuild

The file watcher categorizes changes to minimize rebuild work:

| Change | Rebuild Scope |
|---|---|
| `site.toml` | **Full** — reload config, re-fetch all data, rebuild all pages |
| `_data/*` | **Data** — invalidate file cache, re-render all pages |
| `templates/*` | **Templates** — re-render (cached API data is reused) |
| `templates/_*` (layouts/partials) | **Full** — layout changes can affect any page |
| `static/*` | **Static only** — just re-copy assets, no re-render |

Multiple file changes within 200ms are debounced into a single rebuild.

---

## Example Site

The repository includes an `example_site/` demonstrating real-world usage with an external API ([JSONPlaceholder](https://jsonplaceholder.typicode.com)):

```
example_site/
├── site.toml                      # Configured with blog_api source
├── templates/
│   ├── _base.html                 # Base layout with HTMX
│   ├── _partials/nav.html         # Navigation using _data/nav.yaml
│   ├── index.html                 # Home page with recent posts from API
│   ├── about.html                 # Static about page
│   └── posts/
│       ├── index.html             # Posts listing page
│       └── [post].html            # Dynamic post pages from API
└── _data/
    └── nav.yaml                   # Navigation links
```

Build and serve it:

```bash
eigen build --project example_site
# or
eigen dev --project example_site
```

The dynamic template `posts/[post].html` fetches posts from `https://jsonplaceholder.typicode.com/posts` and generates one page per post (e.g., `dist/posts/1.html`, `dist/posts/2.html`, etc.).

---

## Error Handling

Eigen provides detailed, contextual error messages:

- **Missing `site.toml`** → suggests running `eigen init`
- **Missing environment variable** → names the variable and the file referencing it
- **Invalid frontmatter YAML** → includes the template file path
- **Dynamic page missing `collection`** → names the template and parameter
- **Unknown data source** → lists available sources
- **Undefined template variable** → strict mode catches these at build time
- **Duplicate slugs** → names the slug and template
- **Output path collision** → identifies the conflicting templates
- **HTTP fetch errors** → includes the URL and status code
- **Missing root path in data** → lists available keys at the point of failure

---

## License

See [LICENSE](LICENSE) for details.
