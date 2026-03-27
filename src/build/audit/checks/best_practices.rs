//! Best-practices audit checks.

use super::super::{Category, Finding, Fix, Scope, Severity};
use crate::config::SiteConfig;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Site-level best practices checks.
pub fn site_checks(config: &SiteConfig, dist_path: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // bp/base-url: base_url must be set, not localhost, not example.com
    let url = &config.site.base_url;
    if url.is_empty() || url.contains("localhost") || url.contains("example.com") {
        findings.push(Finding {
            id: "bp/base-url",
            category: Category::BestPractices,
            severity: Severity::Critical,
            scope: Scope::Site,
            message: format!(
                "base_url is invalid: \"{}\". Set a real production URL.",
                url,
            ),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set site.base_url to your production URL (e.g. \"https://mysite.com\")".into(),
            },
        });
    }

    // bp/favicon: at least one favicon file must exist in dist
    let has_favicon = ["favicon.ico", "favicon.png", "favicon.svg"]
        .iter()
        .any(|name| dist_path.join(name).exists());

    if !has_favicon {
        findings.push(Finding {
            id: "bp/favicon",
            category: Category::BestPractices,
            severity: Severity::Medium,
            scope: Scope::Site,
            message: "No favicon found (favicon.ico, favicon.png, or favicon.svg)".into(),
            fix: Fix {
                file: "static/".into(),
                instruction: "Add a favicon.ico, favicon.png, or favicon.svg to your static directory".into(),
            },
        });
    }

    findings
}

/// Page-level best practices checks (HTML inspection).
pub fn page_checks(html: &str, page_path: &str, template_path: &str) -> Vec<Finding> {
    let mut findings = Vec::new();

    // bp/https-links: flag <a href="http://..."> (excluding localhost/127.0.0.1)
    let http_links: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let hl = http_links.clone();

    let _ = lol_html::rewrite_str(html, lol_html::RewriteStrSettings {
        element_content_handlers: vec![
            lol_html::element!("a[href]", move |el| {
                if let Some(href) = el.get_attribute("href")
                    && href.starts_with("http://")
                    && !href.starts_with("http://localhost")
                    && !href.starts_with("http://127.0.0.1")
                {
                    hl.borrow_mut().push(href);
                }
                Ok(())
            }),
        ],
        ..lol_html::RewriteStrSettings::new()
    });

    let insecure = http_links.borrow();
    for href in insecure.iter() {
        findings.push(Finding {
            id: "bp/https-links",
            category: Category::BestPractices,
            severity: Severity::Medium,
            scope: Scope::Page,
            message: format!(
                "{}: insecure HTTP link found: {}",
                page_path, href,
            ),
            fix: Fix {
                file: template_path.into(),
                instruction: format!("Change \"{}\" to use https://", href),
            },
        });
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_config() -> SiteConfig {
        toml::from_str(r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
        "#).unwrap()
    }

    #[test]
    fn test_base_url_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.site.base_url = String::new();
        let findings = site_checks(&config, tmp.path());
        assert!(findings.iter().any(|f| f.id == "bp/base-url"));
    }

    #[test]
    fn test_base_url_localhost() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.site.base_url = "http://localhost:3000".into();
        let findings = site_checks(&config, tmp.path());
        assert!(findings.iter().any(|f| f.id == "bp/base-url"));
    }

    #[test]
    fn test_base_url_valid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = minimal_config();
        config.site.base_url = "https://mysite.com".into();
        let findings = site_checks(&config, tmp.path());
        assert!(!findings.iter().any(|f| f.id == "bp/base-url"));
    }

    #[test]
    fn test_no_favicon() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = minimal_config();
        let findings = site_checks(&config, tmp.path());
        assert!(findings.iter().any(|f| f.id == "bp/favicon"));
    }

    #[test]
    fn test_has_favicon() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("favicon.ico"), "fake").unwrap();
        let config = minimal_config();
        let findings = site_checks(&config, tmp.path());
        assert!(!findings.iter().any(|f| f.id == "bp/favicon"));
    }

    #[test]
    fn test_http_links() {
        let html = r#"<html><head></head><body><a href="http://insecure.com">Link</a></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(findings.iter().any(|f| f.id == "bp/https-links"));
    }

    #[test]
    fn test_https_links_ok() {
        let html = r#"<html><head></head><body><a href="https://secure.com">Link</a></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "bp/https-links"));
    }

    #[test]
    fn test_localhost_http_not_flagged() {
        let html = r#"<html><head></head><body><a href="http://localhost:3000">Dev</a></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "bp/https-links"));
    }
}
