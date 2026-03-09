//! Step 5.5: Sitemap generation.
//!
//! After all pages are rendered, generates `dist/sitemap.xml` with URLs
//! for every rendered page (excluding fragment files).

use eyre::{Result, WrapErr};
use std::path::Path;

use crate::config::SiteConfig;

use super::render::RenderedPage;

/// Generate `sitemap.xml` and write it to `dist/sitemap.xml`.
pub fn generate_sitemap(
    dist_dir: &Path,
    pages: &[RenderedPage],
    config: &SiteConfig,
    build_time: &str,
) -> Result<()> {
    let base_url = config.site.base_url.trim_end_matches('/');

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");

    for page in pages {
        let priority = if page.is_index {
            "1.0"
        } else if page.is_dynamic {
            "0.6"
        } else {
            "0.8"
        };

        let url = format!("{}{}", base_url, normalize_url_path(&page.url_path));

        xml.push_str("  <url>\n");
        xml.push_str(&format!("    <loc>{}</loc>\n", escape_xml(&url)));
        xml.push_str(&format!("    <lastmod>{}</lastmod>\n", escape_xml(build_time)));
        xml.push_str(&format!("    <priority>{}</priority>\n", priority));
        xml.push_str("  </url>\n");
    }

    xml.push_str("</urlset>\n");

    let sitemap_path = dist_dir.join("sitemap.xml");
    std::fs::write(&sitemap_path, &xml)
        .wrap_err_with(|| format!("Failed to write {}", sitemap_path.display()))?;

    Ok(())
}

/// Ensure the URL path starts with `/`.
fn normalize_url_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

/// Escape XML special characters.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BuildConfig, SiteMeta};
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn test_config() -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://example.com".into(),
            },
            build: BuildConfig::default(),
            assets: Default::default(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
        }
    }

    #[test]
    fn test_generate_sitemap_basic() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let pages = vec![
            RenderedPage {
                url_path: "/index.html".into(),
                is_index: true,
                is_dynamic: false,
            },
            RenderedPage {
                url_path: "/about.html".into(),
                is_index: false,
                is_dynamic: false,
            },
        ];

        let config = test_config();
        generate_sitemap(&dist, &pages, &config, "2024-01-01").unwrap();

        let xml = fs::read_to_string(dist.join("sitemap.xml")).unwrap();
        assert!(xml.contains("<?xml version=\"1.0\""));
        assert!(xml.contains("<urlset"));
        assert!(xml.contains("https://example.com/index.html"));
        assert!(xml.contains("https://example.com/about.html"));
        assert!(xml.contains("<priority>1.0</priority>")); // index
        assert!(xml.contains("<priority>0.8</priority>")); // static non-index
    }

    #[test]
    fn test_generate_sitemap_dynamic_pages() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let pages = vec![
            RenderedPage {
                url_path: "/posts/hello.html".into(),
                is_index: false,
                is_dynamic: true,
            },
        ];

        let config = test_config();
        generate_sitemap(&dist, &pages, &config, "2024-01-01").unwrap();

        let xml = fs::read_to_string(dist.join("sitemap.xml")).unwrap();
        assert!(xml.contains("<priority>0.6</priority>"));
    }

    #[test]
    fn test_generate_sitemap_empty() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let config = test_config();
        generate_sitemap(&dist, &[], &config, "2024-01-01").unwrap();

        let xml = fs::read_to_string(dist.join("sitemap.xml")).unwrap();
        assert!(xml.contains("<urlset"));
        assert!(xml.contains("</urlset>"));
        // No <url> entries.
        assert!(!xml.contains("<url>"));
    }

    #[test]
    fn test_normalize_url_path() {
        assert_eq!(normalize_url_path("/about.html"), "/about.html");
        assert_eq!(normalize_url_path("about.html"), "/about.html");
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("a&b"), "a&amp;b");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
    }

    #[test]
    fn test_sitemap_trailing_slash_base_url() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();

        let mut config = test_config();
        config.site.base_url = "https://example.com/".into();

        let pages = vec![
            RenderedPage {
                url_path: "/about.html".into(),
                is_index: false,
                is_dynamic: false,
            },
        ];

        generate_sitemap(&dist, &pages, &config, "2024-01-01").unwrap();

        let xml = fs::read_to_string(dist.join("sitemap.xml")).unwrap();
        // Should not have double slash.
        assert!(xml.contains("https://example.com/about.html"));
        assert!(!xml.contains("https://example.com//about.html"));
    }
}
