//! robots.txt generation.
//!
//! When `[robots]` is configured in site.toml, generates a `robots.txt`
//! file in `dist/` with user-agent rules and sitemap references.

use eyre::{Result, WrapErr};
use std::path::Path;

use crate::config::{RobotsConfig, RobotsRule, SiteConfig};

/// Generate `robots.txt` and write it to `dist/robots.txt`.
///
/// Only called when `config.robots` is `Some`. The caller checks this.
pub fn generate_robots_txt(
    dist_dir: &Path,
    config: &SiteConfig,
) -> Result<()> {
    let robots = match &config.robots {
        Some(r) => r,
        None => return Ok(()),
    };

    let base_url = config.site.base_url.trim_end_matches('/');
    let content = build_robots_content(robots, base_url);

    let robots_path = dist_dir.join("robots.txt");
    std::fs::write(&robots_path, &content)
        .wrap_err_with(|| format!("Failed to write {}", robots_path.display()))?;

    Ok(())
}

/// Build the full robots.txt content string.
fn build_robots_content(robots: &RobotsConfig, base_url: &str) -> String {
    let mut content = String::with_capacity(256);

    // Write rule groups.
    for (i, rule) in robots.rules.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        format_rule(&mut content, rule);
    }

    // Write Sitemap directives.
    let has_sitemaps = robots.sitemap || !robots.extra_sitemaps.is_empty();
    if has_sitemaps && !robots.rules.is_empty() {
        content.push('\n');
    }

    if robots.sitemap {
        content.push_str("Sitemap: ");
        content.push_str(base_url);
        content.push_str("/sitemap.xml\n");
    }

    for url in &robots.extra_sitemaps {
        content.push_str("Sitemap: ");
        content.push_str(url);
        content.push('\n');
    }

    content
}

/// Append a single rule group to the output string.
fn format_rule(out: &mut String, rule: &RobotsRule) {
    out.push_str("User-agent: ");
    out.push_str(&rule.user_agent);
    out.push('\n');

    for path in &rule.allow {
        out.push_str("Allow: ");
        out.push_str(path);
        out.push('\n');
    }

    for path in &rule.disallow {
        out.push_str("Disallow: ");
        out.push_str(path);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        BuildConfig, RobotsConfig, RobotsRule, SiteConfig, SiteMeta,
        SiteSchemaConfig, SiteSeoConfig,
    };
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn test_config_with_robots(robots: RobotsConfig) -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://example.com".into(),
                seo: SiteSeoConfig::default(),
                schema: SiteSchemaConfig::default(),
                extra: std::collections::HashMap::new(),
            },
            build: BuildConfig::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
            feed: HashMap::new(),
            robots: Some(robots),
            audit: None,
        }
    }

    fn default_robots() -> RobotsConfig {
        RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: Vec::new(),
            }],
        }
    }

    // --- build_robots_content ---

    #[test]
    fn test_build_content_default() {
        let robots = default_robots();
        let content = build_robots_content(&robots, "https://example.com");
        assert_eq!(
            content,
            "User-agent: *\nAllow: /\n\nSitemap: https://example.com/sitemap.xml\n"
        );
    }

    #[test]
    fn test_build_content_custom_rules() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![
                RobotsRule {
                    user_agent: "*".into(),
                    allow: vec!["/".into()],
                    disallow: vec!["/admin/".into(), "/private/".into()],
                },
                RobotsRule {
                    user_agent: "Googlebot".into(),
                    allow: vec!["/".into()],
                    disallow: Vec::new(),
                },
            ],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(content.contains("User-agent: *\nAllow: /\nDisallow: /admin/\nDisallow: /private/\n"));
        assert!(content.contains("User-agent: Googlebot\nAllow: /\n"));
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml\n"));
    }

    #[test]
    fn test_build_content_no_sitemap() {
        let robots = RobotsConfig {
            sitemap: false,
            extra_sitemaps: Vec::new(),
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: Vec::new(),
            }],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(!content.contains("Sitemap:"));
    }

    #[test]
    fn test_build_content_extra_sitemaps() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: vec!["https://example.com/news-sitemap.xml".into()],
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: Vec::new(),
            }],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml\n"));
        assert!(content.contains("Sitemap: https://example.com/news-sitemap.xml\n"));
    }

    #[test]
    fn test_build_content_trailing_slash_base_url() {
        let robots = default_robots();
        let content = build_robots_content(&robots, "https://example.com");
        // base_url is already trimmed by the caller, but verify no double slash.
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml"));
        assert!(!content.contains("https://example.com//sitemap.xml"));
    }

    #[test]
    fn test_build_content_disallow_only() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: Vec::new(),
                disallow: vec!["/secret/".into()],
            }],
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(content.contains("User-agent: *\nDisallow: /secret/\n"));
        assert!(!content.contains("Allow:"));
    }

    #[test]
    fn test_build_content_empty_rules() {
        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: Vec::new(),
        };
        let content = build_robots_content(&robots, "https://example.com");
        assert!(!content.contains("User-agent:"));
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml\n"));
    }

    // --- format_rule ---

    #[test]
    fn test_format_rule_basic() {
        let rule = RobotsRule {
            user_agent: "*".into(),
            allow: vec!["/".into()],
            disallow: Vec::new(),
        };
        let mut out = String::new();
        format_rule(&mut out, &rule);
        assert_eq!(out, "User-agent: *\nAllow: /\n");
    }

    #[test]
    fn test_format_rule_multiple_paths() {
        let rule = RobotsRule {
            user_agent: "Googlebot".into(),
            allow: vec!["/public/".into(), "/blog/".into()],
            disallow: vec!["/admin/".into()],
        };
        let mut out = String::new();
        format_rule(&mut out, &rule);
        assert_eq!(
            out,
            "User-agent: Googlebot\nAllow: /public/\nAllow: /blog/\nDisallow: /admin/\n"
        );
    }

    // --- generate_robots_txt (integration with filesystem) ---

    #[test]
    fn test_generate_robots_txt_default() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let config = test_config_with_robots(default_robots());
        generate_robots_txt(&dist, &config).unwrap();

        let content = fs::read_to_string(dist.join("robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Allow: /"));
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml"));
    }

    #[test]
    fn test_generate_robots_txt_none() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let config = SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://example.com".into(),
                seo: SiteSeoConfig::default(),
                schema: SiteSchemaConfig::default(),
                extra: std::collections::HashMap::new(),
            },
            build: BuildConfig::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
            feed: HashMap::new(),
            robots: None,
            audit: None,
        };
        generate_robots_txt(&dist, &config).unwrap();

        // No file should be written.
        assert!(!dist.join("robots.txt").exists());
    }

    #[test]
    fn test_generate_robots_txt_custom_rules() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let robots = RobotsConfig {
            sitemap: true,
            extra_sitemaps: Vec::new(),
            rules: vec![
                RobotsRule {
                    user_agent: "*".into(),
                    allow: vec!["/".into()],
                    disallow: vec!["/admin/".into()],
                },
                RobotsRule {
                    user_agent: "BadBot".into(),
                    allow: Vec::new(),
                    disallow: vec!["/".into()],
                },
            ],
        };
        let config = test_config_with_robots(robots);
        generate_robots_txt(&dist, &config).unwrap();

        let content = fs::read_to_string(dist.join("robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Disallow: /admin/"));
        assert!(content.contains("User-agent: BadBot"));
        assert!(content.contains("Sitemap:"));
    }
}
