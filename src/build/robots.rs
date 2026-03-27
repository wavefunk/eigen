//! Robots.txt generation.
//!
//! When `[robots] enabled = true`, the following priority applies:
//! 1. `static/robots.txt` — copied as-is if present (owner's file wins)
//! 2. `rules` in `site.toml` — generated from config if non-empty
//! 3. hardcoded default — `User-agent: *\nAllow: /`

use eyre::{Result, WrapErr};
use std::path::Path;

use crate::config::{RobotsConfig, RobotsRule, SiteConfig};

const DEFAULT_ROBOTS_TXT: &str = "User-agent: *\nAllow: /\n";

/// Write `dist/robots.txt` according to the priority rules.
pub fn write(project_root: &Path, dist_dir: &Path, config: &SiteConfig) -> Result<()> {
    let custom = project_root.join("static").join("robots.txt");
    let dest = dist_dir.join("robots.txt");

    if custom.exists() {
        std::fs::copy(&custom, &dest)
            .wrap_err_with(|| format!("Failed to copy robots.txt to {}", dest.display()))?;
        tracing::info!("Copying robots.txt... ✓");
    } else if !config.robots.rules.is_empty() {
        let content = build_content(&config.robots, &config.site.base_url);
        std::fs::write(&dest, &content)
            .wrap_err_with(|| format!("Failed to write robots.txt to {}", dest.display()))?;
        tracing::info!("Generating robots.txt from config... ✓");
    } else {
        std::fs::write(&dest, DEFAULT_ROBOTS_TXT)
            .wrap_err_with(|| format!("Failed to write robots.txt to {}", dest.display()))?;
        tracing::info!("Generating default robots.txt... ✓");
    }

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
        SiteSchemaConfig, SiteSeoConfig, SitemapConfig
    };
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn test_config_with_robots(robots: RobotsConfig) -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://example.com".into(),
            },
            build: BuildConfig::default(),
            sitemap: SitemapConfig::default(),
            robots,
            assets: Default::default(),
            sources: HashMap::new(),
            analytics: None,
            plugins: HashMap::new(),
        }
    }

    fn setup(custom_robots: Option<&str>) -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("dist")).unwrap();
        if let Some(content) = custom_robots {
            fs::create_dir_all(tmp.path().join("static")).unwrap();
            fs::write(tmp.path().join("static/robots.txt"), content).unwrap();
        }
        tmp
    }

    #[test]
    fn test_static_file_wins_over_rules() {
        let tmp = setup(Some("User-agent: *\nDisallow: /secret/\n"));
        let mut config = make_config(RobotsConfig {
            enabled: true,
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: vec![],
            }],
            ..Default::default()
        });
        config.site.base_url = "https://example.com".into();
        write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        // static file wins — contains Disallow: /secret/, not Allow: /
        assert!(content.contains("Disallow: /secret/"));
        assert!(!content.contains("Allow: /"));
    }

    #[test]
    fn test_rules_used_when_no_static_file() {
        let tmp = setup(None);
        let config = make_config(RobotsConfig {
            enabled: true,
            sitemap: false,
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec![],
                disallow: vec!["/admin/".into()],
            }],
            ..Default::default()
        });
        write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Disallow: /admin/"));
    }

    #[test]
    fn test_default_when_no_static_and_no_rules() {
        let tmp = setup(None);
        let config = make_config(RobotsConfig::default());
        write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Allow: /"));
    }

    #[test]
    fn test_sitemap_directive_included() {
        let tmp = setup(None);
        let config = make_config(RobotsConfig {
            enabled: true,
            sitemap: true,
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: vec![],
            }],
            ..Default::default()
        });
        write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml"));
    }

    #[test]
    fn test_sitemap_directive_excluded() {
        let tmp = setup(None);
        let config = make_config(RobotsConfig {
            enabled: true,
            sitemap: false,
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: vec![],
            }],
            ..Default::default()
        });
        write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert!(!content.contains("Sitemap:"));
    }

    #[test]
    fn test_extra_sitemaps() {
        let tmp = setup(None);
        let config = make_config(RobotsConfig {
            enabled: true,
            sitemap: false,
            extra_sitemaps: vec!["https://other.example.com/sitemap.xml".into()],
            rules: vec![RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: vec![],
            }],
        });
        write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert!(content.contains("Sitemap: https://other.example.com/sitemap.xml"));
    }

    #[test]
    fn test_multiple_rules() {
        let tmp = setup(None);
        let config = make_config(RobotsConfig {
            enabled: true,
            sitemap: false,
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
        write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();

        let content = fs::read_to_string(dist.join("robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Allow: /"));
        assert!(content.contains("Sitemap: https://example.com/sitemap.xml"));
    }

#[test]
fn test_generate_robots_txt_custom_rules() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("dist")).unwrap();

    let robots = RobotsConfig {
        enabled: true,
        sitemap: true,
        extra_sitemaps: Vec::new(),
        rules: vec![
            RobotsRule {
                user_agent: "*".into(),
                allow: vec!["/".into()],
                disallow: vec!["/admin/".into()],
            },
            RobotsRule {
                user_agent: "Googlebot".into(),
                allow: vec![],
                disallow: vec!["/".into()],
            },
        ],
        ..Default::default()
    };
    let config = test_config_with_robots(robots);
    write(tmp.path(), &tmp.path().join("dist"), &config).unwrap();

    let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
    assert!(content.contains("User-agent: *"));
    assert!(content.contains("User-agent: Googlebot"));
    assert!(content.contains("Disallow: /admin/"));
    assert!(content.contains("Sitemap:"));
}
