# robots.txt Generation -- Design Spec

Date: 2026-03-17

## Motivation and Goals

Every production website should have a `robots.txt` file that tells search
engine crawlers which paths they may or may not access, and where to find
the sitemap. Without one, crawlers use default behavior (crawl everything),
and the sitemap URL must be submitted manually to each search engine.

Eigen already generates `sitemap.xml` at build time. Generating a
`robots.txt` that references the sitemap URL and contains configurable
allow/disallow rules is a natural complement.

**Goals:**

- Generate a valid `robots.txt` file at `dist/robots.txt` during build.
- Allow users to configure allow/disallow rules per user-agent via a
  `[robots]` section in `site.toml`.
- Automatically reference the generated `sitemap.xml` URL using the
  configured `site.base_url`.
- Provide sensible defaults: allow all crawlers, reference the sitemap.
- Zero impact on builds that do not configure `[robots]`.

**Non-goals:**

- Parsing or validating existing `robots.txt` files in `static/`.
  If a user places a `robots.txt` in `static/`, it will be copied to
  `dist/` during the static asset copy phase. The generated `robots.txt`
  will overwrite it, since generation runs later. This is intentional
  and documented -- users should use the config, not a static file.
- Supporting the full robots.txt extensions (crawl-delay, host, etc.).
  These are non-standard and rarely needed. The initial implementation
  covers `User-agent`, `Allow`, `Disallow`, and `Sitemap`.
- Per-page `<meta name="robots">` tag injection. That is a separate
  concern (SEO meta tags) and out of scope here.
- Complex sitemap index support. Eigen generates a single `sitemap.xml`.
  The config supports `extra_sitemaps` for manually specifying additional
  sitemap URLs, but automatic sitemap index generation is out of scope.

## robots.txt Format

The generated file follows the [Robots Exclusion Standard](https://www.rfc-editor.org/rfc/rfc9309).

### Default output (empty `[robots]` table, all defaults):

```
User-agent: *
Allow: /

Sitemap: https://example.com/sitemap.xml
```

### Configured output example:

```toml
[robots]
sitemap = true                    # default: true

[[robots.rules]]
user_agent = "*"
allow = ["/"]
disallow = ["/admin/", "/private/"]

[[robots.rules]]
user_agent = "Googlebot"
allow = ["/"]
disallow = []
```

Produces:

```
User-agent: *
Allow: /
Disallow: /admin/
Disallow: /private/

User-agent: Googlebot
Allow: /

Sitemap: https://example.com/sitemap.xml
```

### Format Rules

1. Each rule group starts with a `User-agent:` line.
2. `Allow:` and `Disallow:` directives follow, one per line.
3. Rule groups are separated by a blank line.
4. The `Sitemap:` directive appears at the end, after all rule groups,
   separated by a blank line. It uses the full absolute URL.
5. No trailing whitespace. Unix line endings (`\n`).

## Configuration

### site.toml Schema

A new optional top-level `[robots]` table.

**Minimal (uses all defaults):**

```toml
[robots]
```

This is equivalent to the default output -- allow all, reference sitemap.
Because the `robots` field on `SiteConfig` is `Option<RobotsConfig>`,
omitting `[robots]` entirely means no `robots.txt` is generated. Including
an empty `[robots]` table triggers generation with all defaults.

**Full example:**

```toml
[robots]
sitemap = true
extra_sitemaps = ["https://example.com/news-sitemap.xml"]

[[robots.rules]]
user_agent = "*"
allow = ["/"]
disallow = ["/admin/", "/private/", "/tmp/"]

[[robots.rules]]
user_agent = "BadBot"
disallow = ["/"]
```

### Config Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `sitemap` | `bool` | `true` | Whether to include a `Sitemap:` directive referencing the generated sitemap.xml |
| `extra_sitemaps` | `Vec<String>` | `[]` | Additional absolute sitemap URLs to include as extra `Sitemap:` directives |
| `rules` | `Vec<RobotsRule>` | `[{user_agent: "*", allow: ["/"], disallow: []}]` | List of rule groups |

### RobotsRule Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `user_agent` | `String` | (required) | The user-agent this rule group applies to |
| `allow` | `Vec<String>` | `[]` | Paths to allow |
| `disallow` | `Vec<String>` | `[]` | Paths to disallow |

### Config Structs

```rust
/// Configuration for robots.txt generation.
///
/// Located under `[robots]` in site.toml. When present (even as an
/// empty table), a robots.txt file is generated during build.
#[derive(Debug, Clone, Deserialize)]
pub struct RobotsConfig {
    /// Whether to include a `Sitemap:` directive for the generated sitemap.xml.
    #[serde(default = "default_true")]
    pub sitemap: bool,

    /// Additional absolute sitemap URLs to include as `Sitemap:` directives.
    #[serde(default)]
    pub extra_sitemaps: Vec<String>,

    /// Rule groups. Defaults to a single rule allowing all crawlers.
    #[serde(default = "default_robots_rules")]
    pub rules: Vec<RobotsRule>,
}

/// A single user-agent rule group in robots.txt.
#[derive(Debug, Clone, Deserialize)]
pub struct RobotsRule {
    /// The user-agent string, e.g. `"*"` or `"Googlebot"`.
    pub user_agent: String,

    /// Paths to allow.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Paths to disallow.
    #[serde(default)]
    pub disallow: Vec<String>,
}

fn default_robots_rules() -> Vec<RobotsRule> {
    vec![RobotsRule {
        user_agent: "*".to_string(),
        allow: vec!["/".to_string()],
        disallow: Vec::new(),
    }]
}
```

### Validation

A new `validate_robots_config` function called from `validate_config`:

1. Each rule must have a non-empty `user_agent`.
2. Each rule must have at least one `allow` or `disallow` directive
   (a rule with neither is valid per the spec but pointless, so we
   warn but do not error).
3. `extra_sitemaps` entries must be absolute URLs (start with `http://`
   or `https://`). Relative sitemap paths are a common mistake.

## Architecture

### New Module: `src/build/robots.rs`

A single new module following the pattern of `src/build/sitemap.rs`.

**Public API:**

```rust
/// Generate `robots.txt` and write it to `dist/robots.txt`.
pub fn generate_robots_txt(
    dist_dir: &Path,
    config: &SiteConfig,
) -> Result<()>
```

The function:
1. Reads `config.robots` (returns early if `None`).
2. Builds the text content from the rules.
3. Appends `Sitemap:` directives.
4. Writes to `dist/robots.txt`.

**Internal functions:**

```rust
/// Format a single rule group as robots.txt text.
fn format_rule(rule: &RobotsRule) -> String

/// Build the sitemap URL from base_url.
fn build_sitemap_url(base_url: &str) -> String
```

### Pipeline Position

robots.txt generation runs after sitemap generation (since it references
the sitemap URL, the sitemap should exist first for logical consistency)
and before feed generation:

```
render all pages
  -> generate sitemap        (build::sitemap)
  -> generate robots.txt     (build::robots)    <-- NEW
  -> generate feeds          (build::feed)
  -> post-build plugin hooks
  -> CSS/JS bundling
  -> content hash rewrite
```

This position is chosen because:

1. robots.txt references the sitemap URL but does not depend on the
   sitemap file existing on disk (it is just a URL reference). Placing
   it after sitemap keeps the logical ordering clear.
2. robots.txt is a simple text file with no dependencies on rendered
   pages, feeds, or bundled assets.
3. It must run after the `dist/` directory is set up and static assets
   are copied (so any static `robots.txt` is overwritten).

### Integration into `build()` in `src/build/render.rs`

After the sitemap generation call (line 199-200), before the feed
generation block, add:

```rust
// Generate robots.txt.
if config.robots.is_some() {
    robots::generate_robots_txt(&dist_dir, &config)?;
    tracing::info!("Generating robots.txt... done");
}
```

### Text Generation

robots.txt is plain text, not XML. The generation is straightforward
string building with no special escaping needed. Paths in robots.txt
are used as-is.

### Content Hash Exclusion

The `robots.txt` file must have a stable name and path. The existing
`default_hash_exclude()` in `ContentHashConfig` already includes
`"robots.txt"` in its exclusion list (line 315 of `config/mod.rs`),
so if a user places a `robots.txt` in `static/`, it would not be
hashed. The generated `robots.txt` is written directly to `dist/`
and is not subject to content hashing regardless.

## Impact on Existing Code

### Files Modified

| File | Change |
|---|---|
| `src/config/mod.rs` | Add `RobotsConfig`, `RobotsRule`, `robots` field on `SiteConfig`, defaults, validation |
| `src/build/mod.rs` | Add `pub mod robots;` |
| `src/build/render.rs` | Call `robots::generate_robots_txt` after sitemap, import `robots` |

### Files Created

| File | Description |
|---|---|
| `src/build/robots.rs` | robots.txt generation module |

### Test Helper Updates

Adding an `Option<RobotsConfig>` field to `SiteConfig` with
`#[serde(default)]` means it defaults to `None`. Since all existing
test helpers either use `toml::from_str` (which auto-derives `None`)
or construct `SiteConfig` manually, the manual constructors need
`robots: None` added. These helpers exist in:

- `src/build/sitemap.rs` (line 79-93)
- `src/build/context.rs`
- `src/discovery/mod.rs`
- `src/template/functions.rs`
- `src/template/environment.rs`

Each needs `robots: None` added. This is mechanical.

### Files NOT Modified

- `src/build/sitemap.rs` -- no changes needed. robots.txt references
  the sitemap URL, not the sitemap module code.
- `src/frontmatter/mod.rs` -- robots.txt is configured in `site.toml`,
  not in template frontmatter.
- `src/data/` -- robots.txt does not fetch external data.
- `src/template/` -- robots.txt is plain text, not rendered via minijinja.

## Test Plan

### Unit Tests (in `src/build/robots.rs`)

| Test | What it verifies |
|---|---|
| `test_generate_robots_default` | Default config produces `User-agent: *`, `Allow: /`, `Sitemap:` |
| `test_generate_robots_custom_rules` | Multiple user-agent groups with allow/disallow |
| `test_generate_robots_no_sitemap` | `sitemap = false` omits `Sitemap:` directive |
| `test_generate_robots_extra_sitemaps` | Additional sitemap URLs appended |
| `test_generate_robots_trailing_slash_base_url` | No double slash in sitemap URL |
| `test_generate_robots_disallow_only` | Rule with only disallow directives |
| `test_generate_robots_empty_rules` | Empty rules vec produces file with only sitemap |
| `test_format_rule_basic` | Single rule group formatted correctly |
| `test_format_rule_multiple_paths` | Multiple allow/disallow paths |
| `test_build_sitemap_url` | Correct URL construction from base_url |

### Config Tests (in `src/config/mod.rs`)

| Test | What it verifies |
|---|---|
| `test_robots_config_absent` | No `[robots]` -> `config.robots` is `None` |
| `test_robots_config_empty` | Empty `[robots]` table -> defaults |
| `test_robots_config_full` | All fields parsed correctly |
| `test_robots_config_no_sitemap` | `sitemap = false` parsed |
| `test_robots_config_multiple_rules` | Multiple `[[robots.rules]]` parsed |
| `test_robots_validation_empty_user_agent` | Error on empty user_agent |
| `test_robots_validation_bad_extra_sitemap` | Error on relative URL in extra_sitemaps |

## What Is NOT In Scope

1. **`Crawl-delay` directive.** Non-standard, not universally supported,
   and rarely needed for static sites. Can be added later if requested.

2. **`Host` directive.** Yandex-specific, non-standard. Out of scope.

3. **Static file conflict detection.** If a user has both a `[robots]`
   config and a `static/robots.txt` file, the generated file overwrites
   the static one silently. We could warn, but this is consistent with
   how the sitemap behaves (generated sitemap.xml overwrites any static
   one). No special handling needed.

4. **Per-page `<meta name="robots">` tags.** This is a separate SEO
   concern handled by frontmatter and the SEO module, not robots.txt.

5. **robots.txt validation service.** The output is simple enough to be
   correct by construction. External validation tools exist for users
   who want extra assurance.

## Performance Considerations

- robots.txt generation is a single string concatenation and file write.
  It adds negligible time to the build process (sub-millisecond).
- No external data fetching, no network calls, no template rendering.
- The generated file is typically < 1 KB.
- No new external dependencies required.
