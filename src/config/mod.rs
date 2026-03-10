use eyre::{Result, WrapErr, bail};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level site configuration parsed from `site.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SiteConfig {
    pub site: SiteMeta,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub assets: AssetsConfig,
    #[serde(default)]
    pub sources: HashMap<String, SourceConfig>,
    /// Plugin configuration tables.  Each key is a plugin name with its
    /// plugin-specific TOML table.  Stored as raw `toml::Value` so plugins
    /// can parse their own config.
    #[serde(default)]
    pub plugins: HashMap<String, toml::Value>,
}

/// Metadata about the site itself.
#[derive(Debug, Clone, Deserialize)]
pub struct SiteMeta {
    pub name: String,
    pub base_url: String,
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
    /// Whether to minify HTML (including inline CSS and JS) output.
    #[serde(default = "default_true")]
    pub minify: bool,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            fragments: true,
            fragment_dir: default_fragment_dir(),
            content_block: default_content_block(),
            minify: true,
        }
    }
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
fn interpolate_env_vars(input: &str) -> Result<String> {
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
            "Missing environment variable(s) referenced in site.toml: {}",
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
}
