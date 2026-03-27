use eyre::{Result, WrapErr, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Top-level site configuration parsed from `site.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SiteConfig {
    pub site: SiteMeta,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub sitemap: SitemapConfig,
    #[serde(default)]
    pub robots: RobotsConfig,
    pub analytics: Option<AnalyticsConfig>,
    #[serde(default)]
    pub assets: AssetsConfig,
    #[serde(default)]
    pub sources: HashMap<String, SourceConfig>,
    /// Plugin configuration tables.  Each key is a plugin name with its
    /// plugin-specific TOML table.  Stored as raw `toml::Value` so plugins
    /// can parse their own config.
    #[serde(default)]
    pub plugins: HashMap<String, toml::Value>,
    /// Feed generation configuration. Each key is a feed name
    /// with its feed-specific config table.
    #[serde(default)]
    pub feed: HashMap<String, FeedConfig>,
    /// robots.txt generation configuration.
    /// When present, a robots.txt file is generated during build.
    /// When absent (`None`), no robots.txt is generated.
    #[serde(default)]
    pub robots: Option<RobotsConfig>,
    /// Audit configuration for post-build HTML quality checks.
    /// When present, checks are run after rendering.
    #[serde(default)]
    pub audit: Option<AuditConfig>,
}

/// Metadata about the site itself.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SiteMeta {
    pub name: String,
    pub base_url: String,
    /// Site-level SEO defaults for Open Graph and Twitter Card tags.
    #[serde(default)]
    pub seo: SiteSeoConfig,
    /// Site-level structured data (JSON-LD) defaults.
    #[serde(default)]
    pub schema: SiteSchemaConfig,
    /// Arbitrary extra fields from `[site]` — exposed to templates as `site.*`.
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

/// Build-related configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct BuildConfig {
    /// Whether to generate HTML fragments alongside full pages.
    #[serde(default = "default_true")]
    pub fragments: bool,
    /// Directory name for fragments inside `dist/`.
    #[serde(default = "default_fragment_dir")]
    pub fragment_dir: String,
    /// The default block name to extract as a fragment.
    #[serde(default = "default_content_block")]
    pub content_block: String,
    /// Block names to include as HTMX out-of-band swaps in every content
    /// fragment.  Each listed block is appended to the content fragment with
    /// `hx-swap-oob="outerHTML"` on its root element.
    #[serde(default)]
    pub oob_blocks: Vec<String>,
    /// Whether to minify HTML (including inline CSS and JS) output.
    #[serde(default = "default_true")]
    pub minify: bool,
    /// Whether to use clean URLs: pages are written as `about/index.html`
    /// instead of `about.html`, so browsers show `/about` instead of `/about.html`.
    /// Requires a web server (e.g. Cloudflare Pages, nginx) that serves
    /// `index.html` for directory requests. Default: false.
    #[serde(default)]
    pub clean_urls: bool,
    /// Critical CSS inlining configuration.
    #[serde(default)]
    pub critical_css: CriticalCssConfig,
    /// Preload/prefetch resource hints configuration.
    #[serde(default)]
    pub hints: HintsConfig,
    /// Content hashing for static assets.
    #[serde(default)]
    pub content_hash: ContentHashConfig,
    /// CSS/JS bundling and tree-shaking configuration.
    #[serde(default)]
    pub bundling: BundlingConfig,
    /// View Transitions API configuration.
    #[serde(default)]
    pub view_transitions: ViewTransitionsConfig,
    /// Whether to generate a `404.html` page. When enabled and no `404.html`
    /// template exists in `templates/`, a built-in default page is written to
    /// `dist/404.html`. If a `404.html` template exists it is rendered normally
    /// and this flag just controls whether the feature is active at all.
    /// Default: true.
    #[serde(default = "default_true")]
    pub not_found: bool,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            fragments: true,
            fragment_dir: default_fragment_dir(),
            content_block: default_content_block(),
            oob_blocks: Vec::new(),
            minify: true,
            clean_urls: false,
            critical_css: CriticalCssConfig::default(),
            hints: HintsConfig::default(),
            content_hash: ContentHashConfig::default(),
            bundling: BundlingConfig::default(),
            view_transitions: ViewTransitionsConfig::default(),
            not_found: true,
        }
    }
}

/// Sitemap generation configuration.
///
/// Located under `[sitemap]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct SitemapConfig {
    /// Whether to generate `sitemap.xml`. Default: true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether to use clean URLs in the sitemap (e.g. `/about/` instead of
    /// `/about.html`). Default: false.
    #[serde(default)]
    pub clean_urls: bool,
}

impl Default for SitemapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            clean_urls: false,
        }
    }
}

/// Robots.txt generation configuration.
///
/// Located under `[robots]` in site.toml.
///
/// Priority when `enabled = true`:
/// 1. `static/robots.txt` — copied as-is if present (owner's file wins)
/// 2. `rules` in this config — generated from TOML rules if non-empty
/// 3. hardcoded default — `User-agent: *\nAllow: /`
#[derive(Debug, Clone, Deserialize)]
pub struct RobotsConfig {
    /// Whether to generate `robots.txt`. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Whether to include a `Sitemap:` directive pointing to the generated
    /// `sitemap.xml`. Default: true.
    #[serde(default = "default_true")]
    pub sitemap: bool,
    /// Additional absolute sitemap URLs to append as `Sitemap:` directives.
    #[serde(default)]
    pub extra_sitemaps: Vec<String>,
    /// User-agent rule groups. If non-empty, robots.txt is generated from
    /// these rules (unless `static/robots.txt` exists).
    #[serde(default)]
    pub rules: Vec<RobotsRule>,
}

impl Default for RobotsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: Vec::new(),
        }
    }
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

/// Google Analytics configuration.
///
/// Located under `[analytics]` in site.toml. When present with a
/// `tracking_id`, the gtag.js snippet is injected into every rendered page
/// before `</body>`.
///
/// Absent or missing `tracking_id` = analytics disabled.
#[derive(Debug, Clone, Deserialize)]
pub struct AnalyticsConfig {
    /// Google Analytics measurement ID, e.g. `"G-XXXXXXXXXX"`.
    pub tracking_id: String,
}

fn default_true() -> bool {
    true
}

fn default_fragment_dir() -> String {
    "_fragments".to_string()
}

fn default_content_block() -> String {
    "content".to_string()
}

/// Configuration for asset localization.
///
/// When enabled, remote URLs found in `src` attributes of `<img>`, `<video>`,
/// `<source>`, `<audio>` tags and CSS `background-image: url(...)` are
/// downloaded to `dist/assets/` and rewritten to local paths.
#[derive(Debug, Clone, Deserialize)]
pub struct AssetsConfig {
    /// Whether asset localization is enabled.
    #[serde(default = "default_true")]
    pub localize: bool,
    /// Additional CDN hostnames to skip (never download).
    /// These are added to the built-in default skip list.
    #[serde(default)]
    pub cdn_skip_hosts: Vec<String>,
    /// Hostnames to force-download even if they match the default CDN skip
    /// list. Useful when a CDN hosts your actual content images.
    #[serde(default)]
    pub cdn_allow_hosts: Vec<String>,
    /// Image optimization configuration.
    #[serde(default)]
    pub images: ImageOptimConfig,
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            localize: true,
            cdn_skip_hosts: Vec::new(),
            cdn_allow_hosts: Vec::new(),
            images: ImageOptimConfig::default(),
        }
    }
}

/// Image optimization configuration.
///
/// Controls format conversion, compression quality, and responsive image
/// generation.  Images are converted to the target formats, resized to
/// the configured widths, and `<img>` tags are rewritten to `<picture>`
/// elements with `srcset` for responsive loading.
#[derive(Debug, Clone, Deserialize)]
pub struct ImageOptimConfig {
    /// Master switch — set to `false` to disable all image optimization.
    #[serde(default = "default_true")]
    pub optimize: bool,
    /// Target output formats. Supported: `"webp"`, `"avif"`.
    /// The original format is always kept as a fallback.
    #[serde(default = "default_image_formats")]
    pub formats: Vec<String>,
    /// Compression quality (1–100). Applies to JPEG, WebP and AVIF output.
    #[serde(default = "default_image_quality")]
    pub quality: u8,
    /// Responsive widths to generate.  Each source image is resized to
    /// these widths (only if the original is wider).
    #[serde(default = "default_image_widths")]
    pub widths: Vec<u32>,
    /// Glob patterns for files/paths to exclude from optimization.
    /// Matched against the asset path relative to the site root
    /// (e.g. `"static/favicons/*"`, `"**/*.svg"`, `"**/*.gif"`).
    #[serde(default = "default_image_exclude")]
    pub exclude: Vec<String>,
}

impl Default for ImageOptimConfig {
    fn default() -> Self {
        Self {
            optimize: true,
            formats: default_image_formats(),
            quality: default_image_quality(),
            widths: default_image_widths(),
            exclude: default_image_exclude(),
        }
    }
}

fn default_image_formats() -> Vec<String> {
    vec!["webp".to_string(), "avif".to_string()]
}

fn default_image_quality() -> u8 {
    80
}

fn default_image_widths() -> Vec<u32> {
    vec![480, 768, 1200]
}

fn default_image_exclude() -> Vec<String> {
    vec![
        "**/*.svg".to_string(),
        "**/*.gif".to_string(),
    ]
}

/// Configuration for critical CSS inlining.
///
/// Located under `[build.critical_css]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct CriticalCssConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Maximum size in bytes for the inlined `<style>` block.
    /// If the critical CSS exceeds this, fall back to the original
    /// `<link>` tag (no inlining for that page).
    /// Default: 50_000 (50 KB).
    #[serde(default = "default_max_inline_size")]
    pub max_inline_size: usize,

    /// Whether to keep the original `<link>` tag for async loading of
    /// the full stylesheet. Default: true.
    /// When false, the `<link>` is removed entirely (pure tree-shaking mode).
    #[serde(default = "default_true")]
    pub preload_full: bool,

    /// Glob patterns for stylesheet paths to exclude from critical CSS
    /// processing. Matched against the href value.
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_max_inline_size() -> usize {
    50_000
}

impl Default for CriticalCssConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_inline_size: default_max_inline_size(),
            preload_full: true,
            exclude: Vec::new(),
        }
    }
}

/// Configuration for preload and prefetch resource hints.
///
/// Located under `[build.hints]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct HintsConfig {
    /// Master switch for resource hints. Default: true.
    /// When false, no preload or prefetch hints are generated.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Whether to auto-detect the hero image from rendered HTML when
    /// no `hero_image` is set in frontmatter. Default: true.
    #[serde(default = "default_true")]
    pub auto_detect_hero: bool,

    /// Whether to generate prefetch hints for navigation links.
    /// Default: true.
    #[serde(default = "default_true")]
    pub prefetch_links: bool,

    /// Maximum number of `<link rel="prefetch">` hints per page.
    /// Default: 5.
    #[serde(default = "default_max_prefetch")]
    pub max_prefetch: usize,

    /// Fallback value for the `imagesizes` attribute on hero image
    /// preload hints. Default: "100vw".
    #[serde(default = "default_image_sizes")]
    pub hero_image_sizes: String,

    /// Glob patterns for link hrefs to exclude from prefetching.
    #[serde(default)]
    pub exclude_prefetch: Vec<String>,
}

fn default_max_prefetch() -> usize {
    5
}

fn default_image_sizes() -> String {
    "100vw".to_string()
}

impl Default for HintsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_detect_hero: true,
            prefetch_links: true,
            max_prefetch: default_max_prefetch(),
            hero_image_sizes: default_image_sizes(),
            exclude_prefetch: Vec::new(),
        }
    }
}

/// Configuration for content hashing of static assets.
///
/// Located under `[build.content_hash]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentHashConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Glob patterns for files in `static/` to exclude from hashing.
    /// Matched against the path relative to `static/`.
    /// Default: common files that must keep stable names.
    #[serde(default = "default_hash_exclude")]
    pub exclude: Vec<String>,
}

fn default_hash_exclude() -> Vec<String> {
    vec![
        "favicon.ico".into(),
        "robots.txt".into(),
        "CNAME".into(),
        "_headers".into(),
        "_redirects".into(),
        ".well-known/**".into(),
    ]
}

impl Default for ContentHashConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            exclude: default_hash_exclude(),
        }
    }
}

/// Configuration for CSS/JS bundling and tree-shaking.
///
/// Located under `[build.bundling]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct BundlingConfig {
    /// Master switch. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Whether to bundle CSS files. Default: true (when bundling is enabled).
    #[serde(default = "default_true")]
    pub css: bool,

    /// Whether to tree-shake CSS (remove unused selectors). Default: true.
    /// Only applies when CSS bundling is enabled.
    /// When false, CSS files are merged but no selectors are removed.
    #[serde(default = "default_true")]
    pub tree_shake_css: bool,

    /// Whether to bundle JS files. Default: true (when bundling is enabled).
    #[serde(default = "default_true")]
    pub js: bool,

    /// Output filename for the bundled CSS file.
    /// Written to `dist/{css_output}`. Default: "css/bundle.css".
    #[serde(default = "default_css_output")]
    pub css_output: String,

    /// Output filename for the bundled JS file.
    /// Written to `dist/{js_output}`. Default: "js/bundle.js".
    #[serde(default = "default_js_output")]
    pub js_output: String,

    /// Glob patterns for stylesheet/script paths to exclude from bundling.
    /// Matched against the href/src value. Excluded files remain as
    /// separate `<link>`/`<script>` tags in the HTML.
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_css_output() -> String {
    "css/bundle.css".to_string()
}

fn default_js_output() -> String {
    "js/bundle.js".to_string()
}

impl Default for BundlingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            css: true,
            tree_shake_css: true,
            js: true,
            css_output: default_css_output(),
            js_output: default_js_output(),
            exclude: Vec::new(),
        }
    }
}

/// View Transitions API configuration.
///
/// When enabled, injects a `<meta name="view-transition">` tag and an
/// inline script that enables `htmx.config.globalViewTransitions` for
/// smooth animated transitions between HTMX partial swaps.
#[derive(Debug, Clone, Deserialize)]
pub struct ViewTransitionsConfig {
    /// Whether to inject view transition meta tag, script, and
    /// transition names into rendered pages.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for ViewTransitionsConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

/// Configuration for a single Atom feed.
///
/// Located under `[feed.<name>]` in site.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedConfig {
    // -- Data source (inline DataQuery-like fields) --

    /// Local file in `_data/`, e.g. `"posts.json"`.
    pub file: Option<String>,
    /// Source name from `[sources.*]`.
    pub source: Option<String>,
    /// URL path appended to the source's base URL.
    /// Named `query_path` to avoid collision with the output `path` field.
    pub query_path: Option<String>,
    /// Dot-path into the response JSON to extract the array from.
    pub root: Option<String>,
    /// Sort spec: `"field"` ascending, `"-field"` descending.
    pub sort: Option<String>,

    // -- Feed metadata --

    /// Feed title. Defaults to `site.name` at generation time.
    pub title: Option<String>,
    /// Output path relative to `dist/`. Defaults to `"feed.xml"`.
    #[serde(default = "default_feed_path")]
    pub path: String,
    /// Feed author name. Defaults to `site.schema.author` at generation time.
    pub author: Option<String>,
    /// Maximum number of entries. Defaults to 50.
    #[serde(default = "default_feed_limit")]
    pub limit: usize,

    // -- Entry field mapping --

    /// Field on each item for the entry `<title>`. Defaults to `"title"`.
    #[serde(default = "default_title_field")]
    pub title_field: String,
    /// Field on each item for the entry `<updated>` date. Defaults to `"date"`.
    #[serde(default = "default_date_field")]
    pub date_field: String,
    /// Field on each item for `<summary>`. Omitted from entries when unset.
    pub summary_field: Option<String>,
    /// Field on each item for the URL slug. Defaults to `"slug"`.
    #[serde(default = "default_slug_field")]
    pub slug_field: String,
    /// URL path prefix for entry links. E.g. `"blog"` produces
    /// `{base_url}/blog/{slug}.html`.
    pub link_prefix: Option<String>,
}

fn default_feed_path() -> String {
    "feed.xml".to_string()
}

fn default_feed_limit() -> usize {
    50
}

fn default_title_field() -> String {
    "title".to_string()
}

fn default_date_field() -> String {
    "date".to_string()
}

fn default_slug_field() -> String {
    "slug".to_string()
}

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

/// Audit configuration for post-build HTML quality checks.
///
/// Located under `[audit]` in site.toml. When present, the build will
/// run SEO, performance, accessibility, and best-practices checks on
/// rendered pages.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuditConfig {
    /// Check IDs to ignore (e.g. `["seo-title", "perf-image-size"]`).
    #[serde(default)]
    pub ignore: Vec<String>,
}

fn default_robots_rules() -> Vec<RobotsRule> {
    vec![RobotsRule {
        user_agent: "*".to_string(),
        allow: vec!["/".to_string()],
        disallow: Vec::new(),
    }]
}

/// Site-level SEO defaults for Open Graph and Twitter Card meta tags.
///
/// Located under `[site.seo]` in site.toml. These provide fallback
/// values for pages that do not set explicit `[seo]` in frontmatter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SiteSeoConfig {
    /// Default page title for `og:title` / `twitter:title`.
    /// Falls back to `site.name` if not set.
    pub title: Option<String>,

    /// Default meta description for pages without one.
    pub description: Option<String>,

    /// Default share image URL (absolute or site-relative path).
    /// Used when a page has no `seo.image` in frontmatter.
    pub image: Option<String>,

    /// Default `og:type`. Default: "website".
    #[serde(default = "default_og_type")]
    pub og_type: String,

    /// Twitter/X @handle for `twitter:site`.
    /// Example: "@mysite"
    pub twitter_site: Option<String>,

    /// Default `twitter:card` type when an image IS available.
    /// Default: "summary_large_image".
    ///
    /// When no image is available (neither from frontmatter nor from
    /// this config), `twitter:card` is forced to `"summary"` regardless
    /// of this setting.
    #[serde(default = "default_twitter_card")]
    pub twitter_card: String,
}

fn default_og_type() -> String {
    "website".to_string()
}

fn default_twitter_card() -> String {
    "summary_large_image".to_string()
}

impl Default for SiteSeoConfig {
    fn default() -> Self {
        Self {
            title: None,
            description: None,
            image: None,
            og_type: default_og_type(),
            twitter_site: None,
            twitter_card: default_twitter_card(),
        }
    }
}

/// Site-level structured data (JSON-LD) defaults.
///
/// Located under `[site.schema]` in site.toml.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SiteSchemaConfig {
    /// Default author name for Article schemas.
    /// Used when a page does not specify an author in frontmatter.
    pub author: Option<String>,

    /// Default schema types to apply to all pages.
    /// Example: ["BreadcrumbList"]
    /// Pages can override this with their own `schema` frontmatter field.
    #[serde(default)]
    pub default_types: Vec<String>,
}

/// Configuration for an external data source (API).
#[derive(Debug, Clone, Deserialize)]
pub struct SourceConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

/// Load and parse `site.toml` from the given project root.
///
/// After parsing, all string values are scanned for `${ENV_VAR}` patterns
/// which are replaced with the corresponding environment variable value.
pub fn load_config(project_root: &Path) -> Result<SiteConfig> {
    let config_path = project_root.join("site.toml");
    if !config_path.exists() {
        bail!(
            "No site.toml found at {}. Run `eigen init` to create a new project.",
            config_path.display()
        );
    }

    let raw = std::fs::read_to_string(&config_path)
        .wrap_err_with(|| format!("Failed to read {}", config_path.display()))?;

    // Perform env var interpolation on the raw TOML string before parsing.
    let interpolated = interpolate_env_vars(&raw)
        .wrap_err("Failed to interpolate environment variables in site.toml")?;

    let config: SiteConfig = toml::from_str(&interpolated)
        .wrap_err("Failed to parse site.toml")?;

    validate_config(&config)?;

    Ok(config)
}

/// Replace all `${VAR_NAME}` occurrences in `input` with the value of the
/// corresponding environment variable. Returns an error if any referenced
/// variable is not set.
pub fn interpolate_env_vars(input: &str) -> Result<String> {
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    let mut result = input.to_string();
    let mut errors: Vec<String> = Vec::new();

    // Collect all matches first to avoid borrowing issues.
    let captures: Vec<(String, String)> = re
        .captures_iter(input)
        .map(|cap| {
            let full_match = cap[0].to_string();
            let var_name = cap[1].to_string();
            (full_match, var_name)
        })
        .collect();

    for (full_match, var_name) in &captures {
        match std::env::var(var_name) {
            Ok(value) => {
                result = result.replace(full_match.as_str(), &value);
            }
            Err(_) => {
                errors.push(var_name.clone());
            }
        }
    }

    if !errors.is_empty() {
        bail!(
            "Missing environment variable(s): {}",
            errors.join(", ")
        );
    }

    Ok(result)
}

/// Validate the parsed configuration for required fields and consistency.
fn validate_config(config: &SiteConfig) -> Result<()> {
    if config.site.base_url.is_empty() {
        bail!("site.base_url must not be empty in site.toml");
    }
    if config.site.name.is_empty() {
        bail!("site.name must not be empty in site.toml");
    }
    validate_feed_configs(config)?;
    validate_robots_config(config)?;
    validate_audit_config(config)?;
    Ok(())
}

/// Validate feed configurations.
fn validate_feed_configs(config: &SiteConfig) -> Result<()> {
    for (name, feed) in &config.feed {
        // Must have at least one data source.
        if feed.file.is_none() && feed.source.is_none() {
            bail!(
                "Feed '{}' must specify either `file` or `source` \
                 for its data source.",
                name,
            );
        }

        // Source must exist in [sources.*].
        if let Some(ref source_name) = feed.source
            && !config.sources.contains_key(source_name)
        {
            let available: Vec<&str> = config.sources.keys()
                .map(|s| s.as_str()).collect();
            bail!(
                "Feed '{}' references source '{}', but it is not \
                 defined in site.toml [sources.*].\n\
                 Available sources: {}",
                name,
                source_name,
                if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available.join(", ")
                },
            );
        }

        // Path must not be empty.
        if feed.path.is_empty() {
            bail!("Feed '{}' has an empty `path`.", name);
        }

        // Limit must be > 0.
        if feed.limit == 0 {
            bail!(
                "Feed '{}' has `limit = 0`. Must be at least 1.",
                name,
            );
        }

        // slug_field must not be empty.
        if feed.slug_field.is_empty() {
            bail!("Feed '{}' has an empty `slug_field`.", name);
        }
    }

    Ok(())
}

/// Validate robots.txt configuration.
fn validate_robots_config(config: &SiteConfig) -> Result<()> {
    let robots = match &config.robots {
        Some(r) => r,
        None => return Ok(()),
    };

    for (i, rule) in robots.rules.iter().enumerate() {
        if rule.user_agent.is_empty() {
            bail!(
                "robots.rules[{}] has an empty `user_agent`. \
                 Each rule must specify a user-agent string.",
                i,
            );
        }

        if rule.allow.is_empty() && rule.disallow.is_empty() {
            tracing::warn!(
                "robots.rules[{}] (user-agent '{}') has no allow or \
                 disallow directives. This rule has no effect.",
                i,
                rule.user_agent,
            );
        }
    }

    for (i, url) in robots.extra_sitemaps.iter().enumerate() {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            bail!(
                "robots.extra_sitemaps[{}] = '{}' is not an absolute URL. \
                 Sitemap URLs must start with http:// or https://.",
                i,
                url,
            );
        }
    }

    Ok(())
}

/// Validate audit configuration.
fn validate_audit_config(config: &SiteConfig) -> Result<()> {
    let _audit = match &config.audit {
        Some(a) => a,
        None => return Ok(()),
    };

    // Currently no constraints to validate — the ignore list accepts
    // arbitrary check IDs and unknown ones are silently skipped at
    // runtime.  This hook exists so future constraints (e.g. validating
    // known check IDs) can be added without touching `validate_config`.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_toml(input: &str) -> Result<SiteConfig> {
        toml::from_str(input).map_err(Into::into)
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[site]
name = "My Site"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.site.name, "My Site");
        assert_eq!(config.site.base_url, "https://example.com");
        assert!(config.build.fragments);
        assert_eq!(config.build.fragment_dir, "_fragments");
        assert_eq!(config.build.content_block, "content");
        assert!(config.sources.is_empty());
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[site]
name = "My Blog"
base_url = "https://blog.example.com"

[build]
fragments = false
fragment_dir = "_frags"
content_block = "main"

[sources.blog_api]
url = "https://api.example.com"
headers = { Authorization = "Bearer token123" }

[sources.cms]
url = "https://cms.example.com/api"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.site.name, "My Blog");
        assert!(!config.build.fragments);
        assert_eq!(config.build.fragment_dir, "_frags");
        assert_eq!(config.build.content_block, "main");

        assert_eq!(config.sources.len(), 2);
        let blog = &config.sources["blog_api"];
        assert_eq!(blog.url, "https://api.example.com");
        assert_eq!(blog.headers["Authorization"], "Bearer token123");

        let cms = &config.sources["cms"];
        assert_eq!(cms.url, "https://cms.example.com/api");
        assert!(cms.headers.is_empty());
    }

    #[test]
    fn test_env_interpolation() {
        // SAFETY: test runner may run tests in parallel, but these use unique
        // env var names so there's no real data race concern in practice.
        unsafe { std::env::set_var("EIGEN_TEST_TOKEN", "secret123") };
        let input = r#"token = "${EIGEN_TEST_TOKEN}""#;
        let result = interpolate_env_vars(input).unwrap();
        assert_eq!(result, r#"token = "secret123""#);
        unsafe { std::env::remove_var("EIGEN_TEST_TOKEN") };
    }

    #[test]
    fn test_env_interpolation_multiple() {
        unsafe {
            std::env::set_var("EIGEN_HOST", "example.com");
            std::env::set_var("EIGEN_PORT", "8080");
        }
        let input = r#"url = "https://${EIGEN_HOST}:${EIGEN_PORT}/api""#;
        let result = interpolate_env_vars(input).unwrap();
        assert_eq!(result, r#"url = "https://example.com:8080/api""#);
        unsafe {
            std::env::remove_var("EIGEN_HOST");
            std::env::remove_var("EIGEN_PORT");
        }
    }

    #[test]
    fn test_env_interpolation_missing_var() {
        let input = r#"token = "${THIS_VAR_DEFINITELY_DOES_NOT_EXIST}""#;
        let result = interpolate_env_vars(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("THIS_VAR_DEFINITELY_DOES_NOT_EXIST"));
    }

    #[test]
    fn test_missing_base_url() {
        let toml_str = r#"
[site]
name = "My Site"
base_url = ""
"#;
        let config = parse_toml(toml_str).unwrap();
        let result = validate_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_config(dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("site.toml"));
    }

    // --- Plugin config tests ---

    #[test]
    fn test_parse_config_with_plugins() {
        let toml_str = r#"
[site]
name = "Plugin Test"
base_url = "https://example.com"

[plugins.strapi]
sources = ["cms"]
media_base_url = "http://localhost:1337"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.plugins.len(), 1);
        assert!(config.plugins.contains_key("strapi"));

        // Verify the raw TOML values are accessible.
        let strapi = config.plugins.get("strapi").unwrap();
        assert_eq!(
            strapi.get("media_base_url").unwrap().as_str().unwrap(),
            "http://localhost:1337"
        );
    }

    #[test]
    fn test_parse_config_without_plugins() {
        let toml_str = r#"
[site]
name = "No Plugins"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.plugins.is_empty());
    }

    #[test]
    fn test_parse_config_empty_plugins() {
        let toml_str = r#"
[site]
name = "Empty Plugins"
base_url = "https://example.com"

[plugins]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.plugins.is_empty());
    }

    #[test]
    fn test_parse_config_custom_plugin_name() {
        // Unknown plugin names should parse fine — they're just TOML tables.
        let toml_str = r#"
[site]
name = "Custom Plugin"
base_url = "https://example.com"

[plugins.my_custom_plugin]
option1 = "value1"
option2 = 42
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.plugins.len(), 1);
        let custom = config.plugins.get("my_custom_plugin").unwrap();
        assert_eq!(custom.get("option1").unwrap().as_str().unwrap(), "value1");
        assert_eq!(custom.get("option2").unwrap().as_integer().unwrap(), 42);
    }

    // --- Image optimization config tests ---

    #[test]
    fn test_image_config_defaults() {
        let toml_str = r#"
[site]
name = "Img Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.assets.images.optimize);
        assert_eq!(config.assets.images.formats, vec!["webp", "avif"]);
        assert_eq!(config.assets.images.quality, 80);
        assert_eq!(config.assets.images.widths, vec![480, 768, 1200]);
        assert_eq!(config.assets.images.exclude, vec!["**/*.svg", "**/*.gif"]);
    }

    #[test]
    fn test_image_config_custom() {
        let toml_str = r#"
[site]
name = "Img Custom"
base_url = "https://example.com"

[assets.images]
optimize = true
formats = ["webp"]
quality = 60
widths = [320, 640, 1024]
exclude = ["static/favicons/*", "**/*.svg", "**/*.gif", "logo.png"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.assets.images.optimize);
        assert_eq!(config.assets.images.formats, vec!["webp"]);
        assert_eq!(config.assets.images.quality, 60);
        assert_eq!(config.assets.images.widths, vec![320, 640, 1024]);
        assert_eq!(config.assets.images.exclude.len(), 4);
        assert!(config.assets.images.exclude.contains(&"static/favicons/*".to_string()));
    }

    #[test]
    fn test_minify_defaults_to_true() {
        let toml_str = r#"
[site]
name = "Minify Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.minify);
    }

    #[test]
    fn test_minify_disabled() {
        let toml_str = r#"
[site]
name = "Minify Disabled"
base_url = "https://example.com"

[build]
minify = false
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.minify);
    }

    // --- Critical CSS config tests ---

    #[test]
    fn test_critical_css_config_defaults() {
        let toml_str = r#"
[site]
name = "CSS Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.critical_css.enabled);
        assert_eq!(config.build.critical_css.max_inline_size, 50_000);
        assert!(config.build.critical_css.preload_full);
        assert!(config.build.critical_css.exclude.is_empty());
    }

    #[test]
    fn test_critical_css_config_custom() {
        let toml_str = r#"
[site]
name = "CSS Custom"
base_url = "https://example.com"

[build.critical_css]
enabled = true
max_inline_size = 30000
preload_full = false
exclude = ["**/vendor/**", "**/print.css"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.critical_css.enabled);
        assert_eq!(config.build.critical_css.max_inline_size, 30_000);
        assert!(!config.build.critical_css.preload_full);
        assert_eq!(config.build.critical_css.exclude.len(), 2);
    }

    #[test]
    fn test_critical_css_enabled_only() {
        let toml_str = r#"
[site]
name = "CSS Enabled"
base_url = "https://example.com"

[build.critical_css]
enabled = true
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.critical_css.enabled);
        // Other fields should have defaults.
        assert_eq!(config.build.critical_css.max_inline_size, 50_000);
        assert!(config.build.critical_css.preload_full);
    }

    #[test]
    fn test_image_config_disabled() {
        let toml_str = r#"
[site]
name = "Img Disabled"
base_url = "https://example.com"

[assets.images]
optimize = false
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.assets.images.optimize);
        // Other fields should still have defaults.
        assert_eq!(config.assets.images.quality, 80);
    }

    // --- Hints config tests ---

    #[test]
    fn test_hints_config_defaults() {
        let toml_str = r#"
[site]
name = "Hints Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.hints.enabled);
        assert!(config.build.hints.auto_detect_hero);
        assert!(config.build.hints.prefetch_links);
        assert_eq!(config.build.hints.max_prefetch, 5);
        assert_eq!(config.build.hints.hero_image_sizes, "100vw");
        assert!(config.build.hints.exclude_prefetch.is_empty());
    }

    #[test]
    fn test_hints_config_custom() {
        let toml_str = r#"
[site]
name = "Hints Custom"
base_url = "https://example.com"

[build.hints]
enabled = true
auto_detect_hero = false
prefetch_links = true
max_prefetch = 3
hero_image_sizes = "(max-width: 1200px) 100vw, 1200px"
exclude_prefetch = ["**/archive/**"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.hints.enabled);
        assert!(!config.build.hints.auto_detect_hero);
        assert!(config.build.hints.prefetch_links);
        assert_eq!(config.build.hints.max_prefetch, 3);
        assert_eq!(
            config.build.hints.hero_image_sizes,
            "(max-width: 1200px) 100vw, 1200px"
        );
        assert_eq!(config.build.hints.exclude_prefetch.len(), 1);
    }

    #[test]
    fn test_hints_config_disabled() {
        let toml_str = r#"
[site]
name = "Hints Off"
base_url = "https://example.com"

[build.hints]
enabled = false
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.hints.enabled);
        // Other fields should have defaults.
        assert!(config.build.hints.auto_detect_hero);
        assert_eq!(config.build.hints.max_prefetch, 5);
    }

    // --- Content hash config tests ---

    #[test]
    fn test_content_hash_config_defaults() {
        let toml_str = r#"
[site]
name = "Hash Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.content_hash.enabled);
        assert_eq!(config.build.content_hash.exclude.len(), 6);
        assert!(config.build.content_hash.exclude.contains(&"favicon.ico".to_string()));
        assert!(config.build.content_hash.exclude.contains(&"CNAME".to_string()));
    }

    #[test]
    fn test_content_hash_config_enabled() {
        let toml_str = r#"
[site]
name = "Hash Enabled"
base_url = "https://example.com"

[build.content_hash]
enabled = true
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.content_hash.enabled);
        // Other fields should have defaults.
        assert_eq!(config.build.content_hash.exclude.len(), 6);
    }

    #[test]
    fn test_content_hash_config_custom_exclude() {
        let toml_str = r#"
[site]
name = "Hash Custom"
base_url = "https://example.com"

[build.content_hash]
enabled = true
exclude = ["favicon.ico", "sw.js", "manifest.json"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.content_hash.enabled);
        assert_eq!(config.build.content_hash.exclude.len(), 3);
        assert!(config.build.content_hash.exclude.contains(&"sw.js".to_string()));
    }

    // --- Bundling config tests ---

    #[test]
    fn test_bundling_config_defaults() {
        let toml_str = r#"
[site]
name = "Bundle Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(!config.build.bundling.enabled);
        assert!(config.build.bundling.css);
        assert!(config.build.bundling.tree_shake_css);
        assert!(config.build.bundling.js);
        assert_eq!(config.build.bundling.css_output, "css/bundle.css");
        assert_eq!(config.build.bundling.js_output, "js/bundle.js");
        assert!(config.build.bundling.exclude.is_empty());
    }

    #[test]
    fn test_bundling_config_enabled_only() {
        let toml_str = r#"
[site]
name = "Bundle Enabled"
base_url = "https://example.com"

[build.bundling]
enabled = true
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.bundling.enabled);
        // Other fields should have defaults.
        assert!(config.build.bundling.css);
        assert!(config.build.bundling.tree_shake_css);
        assert!(config.build.bundling.js);
        assert_eq!(config.build.bundling.css_output, "css/bundle.css");
    }

    #[test]
    fn test_bundling_config_custom() {
        let toml_str = r#"
[site]
name = "Bundle Custom"
base_url = "https://example.com"

[build.bundling]
enabled = true
css = true
tree_shake_css = false
js = false
css_output = "assets/styles.css"
js_output = "assets/scripts.js"
exclude = ["**/vendor/**", "**/print.css"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.bundling.enabled);
        assert!(config.build.bundling.css);
        assert!(!config.build.bundling.tree_shake_css);
        assert!(!config.build.bundling.js);
        assert_eq!(config.build.bundling.css_output, "assets/styles.css");
        assert_eq!(config.build.bundling.js_output, "assets/scripts.js");
        assert_eq!(config.build.bundling.exclude.len(), 2);
    }

    #[test]
    fn test_bundling_config_css_only() {
        let toml_str = r#"
[site]
name = "CSS Only"
base_url = "https://example.com"

[build.bundling]
enabled = true
js = false
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.build.bundling.enabled);
        assert!(config.build.bundling.css);
        assert!(!config.build.bundling.js);
    }

    // --- Site SEO config tests ---

    #[test]
    fn test_site_seo_config_defaults() {
        let toml_str = r#"
[site]
name = "SEO Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.site.seo.title.is_none());
        assert!(config.site.seo.description.is_none());
        assert!(config.site.seo.image.is_none());
        assert_eq!(config.site.seo.og_type, "website");
        assert!(config.site.seo.twitter_site.is_none());
        assert_eq!(config.site.seo.twitter_card, "summary_large_image");
    }

    #[test]
    fn test_site_seo_config_custom() {
        let toml_str = r#"
[site]
name = "SEO Custom"
base_url = "https://example.com"

[site.seo]
title = "My Custom Title"
description = "A description of the site"
image = "/assets/default-share.jpg"
og_type = "article"
twitter_site = "@mysite"
twitter_card = "summary"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.site.seo.title.as_deref(), Some("My Custom Title"));
        assert_eq!(config.site.seo.description.as_deref(), Some("A description of the site"));
        assert_eq!(config.site.seo.image.as_deref(), Some("/assets/default-share.jpg"));
        assert_eq!(config.site.seo.og_type, "article");
        assert_eq!(config.site.seo.twitter_site.as_deref(), Some("@mysite"));
        assert_eq!(config.site.seo.twitter_card, "summary");
    }

    #[test]
    fn test_site_seo_config_partial() {
        let toml_str = r#"
[site]
name = "SEO Partial"
base_url = "https://example.com"

[site.seo]
description = "Only description set"
twitter_site = "@partial"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.site.seo.title.is_none());
        assert_eq!(config.site.seo.description.as_deref(), Some("Only description set"));
        assert!(config.site.seo.image.is_none());
        assert_eq!(config.site.seo.og_type, "website");
        assert_eq!(config.site.seo.twitter_site.as_deref(), Some("@partial"));
        assert_eq!(config.site.seo.twitter_card, "summary_large_image");
    }

    // --- Site schema config tests ---

    #[test]
    fn test_site_schema_config_defaults() {
        let toml_str = r#"
[site]
name = "Schema Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.site.schema.author.is_none());
        assert!(config.site.schema.default_types.is_empty());
    }

    #[test]
    fn test_site_schema_config_custom() {
        let toml_str = r#"
[site]
name = "Schema Custom"
base_url = "https://example.com"

[site.schema]
author = "Jane Doe"
default_types = ["BreadcrumbList"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.site.schema.author.as_deref(), Some("Jane Doe"));
        assert_eq!(config.site.schema.default_types, vec!["BreadcrumbList"]);
    }

    // --- Feed config tests ---

    #[test]
    fn test_feed_config_defaults() {
        let toml_str = r#"
[site]
name = "Feed Default"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.feed.is_empty());
    }

    #[test]
    fn test_feed_config_parsing() {
        let toml_str = r#"
[site]
name = "Feed Test"
base_url = "https://example.com"

[feed.blog]
file = "posts.json"
title = "Blog Feed"
path = "blog/feed.xml"
author = "Jane Doe"
limit = 20
title_field = "name"
date_field = "publishedAt"
summary_field = "excerpt"
slug_field = "id"
link_prefix = "blog"
sort = "-publishedAt"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.feed.len(), 1);
        let blog = &config.feed["blog"];
        assert_eq!(blog.file.as_deref(), Some("posts.json"));
        assert_eq!(blog.title.as_deref(), Some("Blog Feed"));
        assert_eq!(blog.path, "blog/feed.xml");
        assert_eq!(blog.author.as_deref(), Some("Jane Doe"));
        assert_eq!(blog.limit, 20);
        assert_eq!(blog.title_field, "name");
        assert_eq!(blog.date_field, "publishedAt");
        assert_eq!(blog.summary_field.as_deref(), Some("excerpt"));
        assert_eq!(blog.slug_field, "id");
        assert_eq!(blog.link_prefix.as_deref(), Some("blog"));
        assert_eq!(blog.sort.as_deref(), Some("-publishedAt"));
    }

    #[test]
    fn test_feed_config_multiple() {
        let toml_str = r#"
[site]
name = "Multi Feed"
base_url = "https://example.com"

[feed.blog]
file = "posts.json"

[feed.changelog]
file = "releases.json"
path = "changelog/feed.xml"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert_eq!(config.feed.len(), 2);
        assert!(config.feed.contains_key("blog"));
        assert!(config.feed.contains_key("changelog"));
        // blog should have default path
        assert_eq!(config.feed["blog"].path, "feed.xml");
        assert_eq!(config.feed["changelog"].path, "changelog/feed.xml");
    }

    #[test]
    fn test_feed_config_field_defaults() {
        let toml_str = r#"
[site]
name = "Feed Defaults"
base_url = "https://example.com"

[feed.blog]
file = "posts.json"
"#;
        let config = parse_toml(toml_str).unwrap();
        let blog = &config.feed["blog"];
        assert_eq!(blog.path, "feed.xml");
        assert_eq!(blog.limit, 50);
        assert_eq!(blog.title_field, "title");
        assert_eq!(blog.date_field, "date");
        assert_eq!(blog.slug_field, "slug");
        assert!(blog.title.is_none());
        assert!(blog.author.is_none());
        assert!(blog.summary_field.is_none());
        assert!(blog.link_prefix.is_none());
        assert!(blog.source.is_none());
        assert!(blog.query_path.is_none());
        assert!(blog.root.is_none());
        assert!(blog.sort.is_none());
    }

    #[test]
    fn test_feed_config_with_source() {
        let toml_str = r#"
[site]
name = "Feed Source"
base_url = "https://example.com"

[sources.cms]
url = "https://cms.example.com/api"

[feed.blog]
source = "cms"
query_path = "/posts"
root = "data.posts"
sort = "-date"
link_prefix = "blog"
"#;
        let config = parse_toml(toml_str).unwrap();
        let blog = &config.feed["blog"];
        assert_eq!(blog.source.as_deref(), Some("cms"));
        assert_eq!(blog.query_path.as_deref(), Some("/posts"));
        assert_eq!(blog.root.as_deref(), Some("data.posts"));
    }

    #[test]
    fn test_feed_validation_no_source() {
        let toml_str = r#"
[site]
name = "Bad Feed"
base_url = "https://example.com"

[feed.blog]
title = "Blog"
"#;
        let config = parse_toml(toml_str).unwrap();
        let result = validate_feed_configs(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("blog"));
        assert!(err.contains("file"));
    }

    #[test]
    fn test_feed_validation_bad_source() {
        let toml_str = r#"
[site]
name = "Bad Source Feed"
base_url = "https://example.com"

[feed.blog]
source = "nonexistent"
"#;
        let config = parse_toml(toml_str).unwrap();
        let result = validate_feed_configs(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent"));
    }

    #[test]
    fn test_feed_validation_empty_path() {
        let toml_str = r#"
[site]
name = "Empty Path Feed"
base_url = "https://example.com"

[feed.blog]
file = "posts.json"
path = ""
"#;
        let config = parse_toml(toml_str).unwrap();
        let result = validate_feed_configs(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_feed_validation_zero_limit() {
        let toml_str = r#"
[site]
name = "Zero Limit Feed"
base_url = "https://example.com"

[feed.blog]
file = "posts.json"
limit = 0
"#;
        let config = parse_toml(toml_str).unwrap();
        let result = validate_feed_configs(&config);
        assert!(result.is_err());
    }

    // --- Robots config tests ---

    #[test]
    fn test_robots_config_absent() {
        let toml_str = r#"
[site]
name = "No Robots"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.robots.is_none());
    }

    #[test]
    fn test_robots_config_empty_table() {
        let toml_str = r#"
[site]
name = "Robots Default"
base_url = "https://example.com"

[robots]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.robots.is_some());
        let robots = config.robots.unwrap();
        assert!(robots.sitemap);
        assert!(robots.extra_sitemaps.is_empty());
        assert_eq!(robots.rules.len(), 1);
        assert_eq!(robots.rules[0].user_agent, "*");
        assert_eq!(robots.rules[0].allow, vec!["/"]);
        assert!(robots.rules[0].disallow.is_empty());
    }

    #[test]
    fn test_robots_config_full() {
        let toml_str = r#"
[site]
name = "Robots Full"
base_url = "https://example.com"

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
"#;
        let config = parse_toml(toml_str).unwrap();
        let robots = config.robots.unwrap();
        assert!(robots.sitemap);
        assert_eq!(robots.extra_sitemaps, vec!["https://example.com/news-sitemap.xml"]);
        assert_eq!(robots.rules.len(), 2);
        assert_eq!(robots.rules[0].user_agent, "*");
        assert_eq!(robots.rules[0].allow, vec!["/"]);
        assert_eq!(robots.rules[0].disallow, vec!["/admin/", "/private/"]);
        assert_eq!(robots.rules[1].user_agent, "BadBot");
        assert!(robots.rules[1].allow.is_empty());
        assert_eq!(robots.rules[1].disallow, vec!["/"]);
    }

    #[test]
    fn test_robots_config_no_sitemap() {
        let toml_str = r#"
[site]
name = "Robots No Sitemap"
base_url = "https://example.com"

[robots]
sitemap = false
"#;
        let config = parse_toml(toml_str).unwrap();
        let robots = config.robots.unwrap();
        assert!(!robots.sitemap);
    }

    #[test]
    fn test_robots_config_multiple_rules() {
        let toml_str = r#"
[site]
name = "Robots Multi"
base_url = "https://example.com"

[[robots.rules]]
user_agent = "Googlebot"
allow = ["/"]

[[robots.rules]]
user_agent = "Bingbot"
allow = ["/public/"]
disallow = ["/private/"]
"#;
        let config = parse_toml(toml_str).unwrap();
        let robots = config.robots.unwrap();
        assert_eq!(robots.rules.len(), 2);
    }

    #[test]
    fn test_robots_validation_empty_user_agent() {
        let toml_str = r#"
[site]
name = "Bad Robots"
base_url = "https://example.com"

[[robots.rules]]
user_agent = ""
allow = ["/"]
"#;
        let config = parse_toml(toml_str).unwrap();
        let result = validate_robots_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("user_agent"));
    }

    // --- Audit config tests ---

    #[test]
    fn test_audit_config_default_absent() {
        let toml_str = r#"
[site]
name = "No Audit"
base_url = "https://example.com"
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.audit.is_none());
    }

    #[test]
    fn test_audit_config_with_ignore() {
        let toml_str = r#"
[site]
name = "Audit Ignore"
base_url = "https://example.com"

[audit]
ignore = ["seo-title", "perf-image-size"]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.audit.is_some());
        let audit = config.audit.unwrap();
        assert_eq!(audit.ignore, vec!["seo-title", "perf-image-size"]);
    }

    #[test]
    fn test_audit_config_empty_table() {
        let toml_str = r#"
[site]
name = "Audit Empty"
base_url = "https://example.com"

[audit]
"#;
        let config = parse_toml(toml_str).unwrap();
        assert!(config.audit.is_some());
        let audit = config.audit.unwrap();
        assert!(audit.ignore.is_empty());
    }

    #[test]
    fn test_robots_validation_bad_extra_sitemap() {
        let toml_str = r#"
[site]
name = "Bad Sitemap URL"
base_url = "https://example.com"

[robots]
extra_sitemaps = ["/sitemap-news.xml"]
"#;
        let config = parse_toml(toml_str).unwrap();
        let result = validate_robots_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("absolute URL"));
    }

    // --- View transitions config tests ---

    #[test]
    fn test_view_transitions_default_disabled() {
        let toml_str = r#"
[site]
name = "Test"
base_url = "https://example.com"
"#;
        let config: SiteConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.build.view_transitions.enabled);
    }

    #[test]
    fn test_view_transitions_enabled() {
        let toml_str = r#"
[site]
name = "Test"
base_url = "https://example.com"
[build.view_transitions]
enabled = true
"#;
        let config: SiteConfig = toml::from_str(toml_str).unwrap();
        assert!(config.build.view_transitions.enabled);
    }
}
