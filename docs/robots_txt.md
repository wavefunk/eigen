# robots.txt Generation

Eigen can auto-generate a `robots.txt` file at build time. The file is
written to `dist/robots.txt` and follows the
[Robots Exclusion Standard (RFC 9309)](https://www.rfc-editor.org/rfc/rfc9309).

## Configuration

Add a `[robots]` table to `site.toml`. When the table is present (even
empty), a `robots.txt` is generated. When absent, no file is generated.

### Minimal (all defaults)

```toml
[robots]
```

Produces:

```
User-agent: *
Allow: /

Sitemap: https://example.com/sitemap.xml
```

### Full example

```toml
[robots]
sitemap = true
extra_sitemaps = ["https://example.com/news-sitemap.xml"]

[[robots.rules]]
user_agent = "*"
allow = ["/"]
disallow = ["/admin/", "/private/"]

[[robots.rules]]
user_agent = "BadBot"
disallow = ["/"]
```

### Config fields

| Field | Type | Default | Description |
|---|---|---|---|
| `sitemap` | `bool` | `true` | Include `Sitemap:` directive for generated sitemap.xml |
| `extra_sitemaps` | `Vec<String>` | `[]` | Additional absolute sitemap URLs |
| `rules` | `Vec<RobotsRule>` | `[{user_agent: "*", allow: ["/"]}]` | Rule groups |

### RobotsRule fields

| Field | Type | Default | Description |
|---|---|---|---|
| `user_agent` | `String` | (required) | User-agent this rule applies to |
| `allow` | `Vec<String>` | `[]` | Paths to allow |
| `disallow` | `Vec<String>` | `[]` | Paths to disallow |

## Validation

- Each rule must have a non-empty `user_agent`.
- `extra_sitemaps` entries must be absolute URLs (`http://` or `https://`).
- A rule with no `allow` or `disallow` directives triggers a warning
  (valid per spec but pointless).

## Build pipeline position

```
render pages -> sitemap -> robots.txt -> feeds -> post-build hooks -> bundling
```

## Architecture

- Config structs: `src/config/mod.rs` (`RobotsConfig`, `RobotsRule`)
- Generation: `src/build/robots.rs`
- Pipeline call: `src/build/render.rs`

## Notes

- If a `robots.txt` exists in `static/`, the generated file overwrites it.
  Use the config instead of a static file.
- The sitemap URL is built from `site.base_url` + `/sitemap.xml`.
  Trailing slashes on `base_url` are handled automatically.
