//! SEO audit checks.

use super::super::{Category, Finding, Fix, Scope, Severity};
use crate::config::SiteConfig;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Site-level SEO checks (config + dist inspection).
pub fn site_checks(config: &SiteConfig, dist_path: &Path) -> Vec<Finding> {
    let mut results = Vec::new();

    // seo/sitemap — check if dist/sitemap.xml exists.
    if !dist_path.join("sitemap.xml").exists() {
        results.push(Finding {
            id: "seo/sitemap",
            category: Category::Seo,
            severity: Severity::Critical,
            scope: Scope::Site,
            message: "No sitemap.xml found in dist/".into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Ensure the build generates a sitemap.xml in the output directory"
                    .into(),
            },
        });
    }

    // seo/robots-txt — check if config.robots is None.
    if config.robots.is_none() {
        results.push(Finding {
            id: "seo/robots-txt",
            category: Category::Seo,
            severity: Severity::High,
            scope: Scope::Site,
            message: "No robots.txt configuration found".into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Add a [robots] section to site.toml to generate robots.txt".into(),
            },
        });
    }

    // seo/feed — check if config.feed is empty.
    if config.feed.is_empty() {
        results.push(Finding {
            id: "seo/feed",
            category: Category::Seo,
            severity: Severity::Low,
            scope: Scope::Site,
            message: "No feeds configured".into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Add a [feed.<name>] section to site.toml to generate an Atom feed"
                    .into(),
            },
        });
    }

    results
}

/// Page-level SEO checks (HTML inspection).
pub fn page_checks(html: &str, page_path: &str, template_path: &str) -> Vec<Finding> {
    let has_title = Rc::new(RefCell::new(false));
    let has_meta_description = Rc::new(RefCell::new(false));
    let has_canonical = Rc::new(RefCell::new(false));
    let has_og_title = Rc::new(RefCell::new(false));
    let has_og_description = Rc::new(RefCell::new(false));
    let has_og_image = Rc::new(RefCell::new(false));
    let has_twitter_card = Rc::new(RefCell::new(false));
    let has_structured_data = Rc::new(RefCell::new(false));
    let h1_count = Rc::new(RefCell::new(0u32));

    // Clone handles for the closures.
    let ht = has_title.clone();
    let hmd = has_meta_description.clone();
    let hc = has_canonical.clone();
    let hot = has_og_title.clone();
    let hod = has_og_description.clone();
    let hoi = has_og_image.clone();
    let htc = has_twitter_card.clone();
    let hsd = has_structured_data.clone();
    let h1c = h1_count.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                // <title>
                lol_html::element!("title", move |_el| {
                    *ht.borrow_mut() = true;
                    Ok(())
                }),
                // <meta name="description">
                lol_html::element!("meta[name=\"description\"]", move |_el| {
                    *hmd.borrow_mut() = true;
                    Ok(())
                }),
                // <link rel="canonical">
                lol_html::element!("link[rel=\"canonical\"]", move |_el| {
                    *hc.borrow_mut() = true;
                    Ok(())
                }),
                // <meta property="og:title">
                lol_html::element!("meta[property=\"og:title\"]", move |_el| {
                    *hot.borrow_mut() = true;
                    Ok(())
                }),
                // <meta property="og:description">
                lol_html::element!("meta[property=\"og:description\"]", move |_el| {
                    *hod.borrow_mut() = true;
                    Ok(())
                }),
                // <meta property="og:image">
                lol_html::element!("meta[property=\"og:image\"]", move |_el| {
                    *hoi.borrow_mut() = true;
                    Ok(())
                }),
                // <meta name="twitter:card">
                lol_html::element!("meta[name=\"twitter:card\"]", move |_el| {
                    *htc.borrow_mut() = true;
                    Ok(())
                }),
                // <script type="application/ld+json">
                lol_html::element!("script[type=\"application/ld+json\"]", move |_el| {
                    *hsd.borrow_mut() = true;
                    Ok(())
                }),
                // <h1>
                lol_html::element!("h1", move |_el| {
                    *h1c.borrow_mut() += 1;
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let mut results = Vec::new();
    let fix_file = template_path.to_string();

    // seo/meta-title
    if !*has_title.borrow() {
        results.push(Finding {
            id: "seo/meta-title",
            category: Category::Seo,
            severity: Severity::High,
            scope: Scope::Page,
            message: format!("{page_path}: missing <title> tag"),
            fix: Fix {
                file: fix_file.clone(),
                instruction: "Add a <title> element inside <head>".into(),
            },
        });
    }

    // seo/meta-description
    if !*has_meta_description.borrow() {
        results.push(Finding {
            id: "seo/meta-description",
            category: Category::Seo,
            severity: Severity::High,
            scope: Scope::Page,
            message: format!("{page_path}: missing <meta name=\"description\">"),
            fix: Fix {
                file: fix_file.clone(),
                instruction: "Add <meta name=\"description\" content=\"...\"> inside <head>".into(),
            },
        });
    }

    // seo/canonical
    if !*has_canonical.borrow() {
        results.push(Finding {
            id: "seo/canonical",
            category: Category::Seo,
            severity: Severity::High,
            scope: Scope::Page,
            message: format!("{page_path}: missing <link rel=\"canonical\">"),
            fix: Fix {
                file: fix_file.clone(),
                instruction: "Add <link rel=\"canonical\" href=\"...\"> inside <head>".into(),
            },
        });
    }

    // seo/og-tags — report if any of og:title, og:description, og:image is missing.
    {
        let mut missing_og: Vec<&str> = Vec::new();
        if !*has_og_title.borrow() {
            missing_og.push("og:title");
        }
        if !*has_og_description.borrow() {
            missing_og.push("og:description");
        }
        if !*has_og_image.borrow() {
            missing_og.push("og:image");
        }
        if !missing_og.is_empty() {
            results.push(Finding {
                id: "seo/og-tags",
                category: Category::Seo,
                severity: Severity::Medium,
                scope: Scope::Page,
                message: format!(
                    "{page_path}: missing Open Graph tags: {}",
                    missing_og.join(", ")
                ),
                fix: Fix {
                    file: fix_file.clone(),
                    instruction: "Add the missing <meta property=\"og:...\"> tags inside <head>"
                        .into(),
                },
            });
        }
    }

    // seo/twitter-tags
    if !*has_twitter_card.borrow() {
        results.push(Finding {
            id: "seo/twitter-tags",
            category: Category::Seo,
            severity: Severity::Low,
            scope: Scope::Page,
            message: format!("{page_path}: missing <meta name=\"twitter:card\">"),
            fix: Fix {
                file: fix_file.clone(),
                instruction: "Add <meta name=\"twitter:card\" content=\"summary_large_image\"> inside <head>".into(),
            },
        });
    }

    // seo/structured-data
    if !*has_structured_data.borrow() {
        results.push(Finding {
            id: "seo/structured-data",
            category: Category::Seo,
            severity: Severity::Medium,
            scope: Scope::Page,
            message: format!(
                "{page_path}: missing <script type=\"application/ld+json\"> structured data"
            ),
            fix: Fix {
                file: fix_file.clone(),
                instruction:
                    "Add a <script type=\"application/ld+json\"> block with schema.org markup"
                        .into(),
            },
        });
    }

    // seo/heading-hierarchy — no <h1> or multiple <h1> tags.
    let count = *h1_count.borrow();
    if count == 0 {
        results.push(Finding {
            id: "seo/heading-hierarchy",
            category: Category::Seo,
            severity: Severity::Medium,
            scope: Scope::Page,
            message: format!("{page_path}: no <h1> tag found"),
            fix: Fix {
                file: fix_file,
                instruction: "Add exactly one <h1> element to the page".into(),
            },
        });
    } else if count > 1 {
        results.push(Finding {
            id: "seo/heading-hierarchy",
            category: Category::Seo,
            severity: Severity::Medium,
            scope: Scope::Page,
            message: format!("{page_path}: multiple <h1> tags found ({count})"),
            fix: Fix {
                file: fix_file,
                instruction: "Use exactly one <h1> element per page; demote extras to <h2> or lower".into(),
            },
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteConfig;

    /// Create a minimal SiteConfig from TOML (no robots, no feeds).
    fn minimal_config() -> SiteConfig {
        toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            "#,
        )
        .unwrap()
    }

    /// Create a SiteConfig with robots configured.
    fn config_with_robots() -> SiteConfig {
        toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [robots]
            "#,
        )
        .unwrap()
    }

    /// Create a SiteConfig with a feed configured.
    fn config_with_feed() -> SiteConfig {
        toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [feed.blog]
            file = "posts.json"
            "#,
        )
        .unwrap()
    }

    // ---- Site-level checks ----

    #[test]
    fn sitemap_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let findings = site_checks(&minimal_config(), tmp.path());
        assert!(findings.iter().any(|f| f.id == "seo/sitemap"));
    }

    #[test]
    fn sitemap_present() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("sitemap.xml"), "<urlset/>").unwrap();
        let findings = site_checks(&minimal_config(), tmp.path());
        assert!(!findings.iter().any(|f| f.id == "seo/sitemap"));
    }

    #[test]
    fn robots_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let findings = site_checks(&minimal_config(), tmp.path());
        assert!(findings.iter().any(|f| f.id == "seo/robots-txt"));
    }

    #[test]
    fn robots_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let findings = site_checks(&config_with_robots(), tmp.path());
        assert!(!findings.iter().any(|f| f.id == "seo/robots-txt"));
    }

    #[test]
    fn feed_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let findings = site_checks(&minimal_config(), tmp.path());
        assert!(findings.iter().any(|f| f.id == "seo/feed"));
    }

    #[test]
    fn feed_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let findings = site_checks(&config_with_feed(), tmp.path());
        assert!(!findings.iter().any(|f| f.id == "seo/feed"));
    }

    #[test]
    fn site_checks_severity_and_category() {
        let tmp = tempfile::tempdir().unwrap();
        let findings = site_checks(&minimal_config(), tmp.path());

        let sitemap = findings.iter().find(|f| f.id == "seo/sitemap").unwrap();
        assert_eq!(sitemap.severity, Severity::Critical);
        assert_eq!(sitemap.category, Category::Seo);
        assert_eq!(sitemap.scope, Scope::Site);

        let robots = findings.iter().find(|f| f.id == "seo/robots-txt").unwrap();
        assert_eq!(robots.severity, Severity::High);

        let feed = findings.iter().find(|f| f.id == "seo/feed").unwrap();
        assert_eq!(feed.severity, Severity::Low);
    }

    // ---- Page-level checks ----

    /// Fully valid HTML with all SEO elements.
    const GOOD_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
    <title>My Page</title>
    <meta name="description" content="A great page">
    <link rel="canonical" href="https://example.com/page">
    <meta property="og:title" content="My Page">
    <meta property="og:description" content="A great page">
    <meta property="og:image" content="https://example.com/img.jpg">
    <meta name="twitter:card" content="summary_large_image">
    <script type="application/ld+json">{"@type":"WebPage"}</script>
</head>
<body>
    <h1>Hello World</h1>
</body>
</html>"#;

    /// Minimal HTML missing all SEO elements.
    const BAD_HTML: &str = "<html><head></head><body><p>hello</p></body></html>";

    #[test]
    fn good_html_no_findings() {
        let findings = page_checks(GOOD_HTML, "/page.html", "templates/page.html");
        assert!(findings.is_empty(), "expected no findings, got: {findings:?}");
    }

    #[test]
    fn bad_html_all_findings() {
        let findings = page_checks(BAD_HTML, "/page.html", "templates/page.html");
        let ids: Vec<&str> = findings.iter().map(|f| f.id).collect();
        assert!(ids.contains(&"seo/meta-title"));
        assert!(ids.contains(&"seo/meta-description"));
        assert!(ids.contains(&"seo/canonical"));
        assert!(ids.contains(&"seo/og-tags"));
        assert!(ids.contains(&"seo/twitter-tags"));
        assert!(ids.contains(&"seo/structured-data"));
        assert!(ids.contains(&"seo/heading-hierarchy"));
    }

    #[test]
    fn missing_title() {
        let html = "<html><head><meta name=\"description\" content=\"x\"></head><body><h1>H</h1></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(findings.iter().any(|f| f.id == "seo/meta-title"));
    }

    #[test]
    fn has_title() {
        let html = "<html><head><title>Hello</title></head><body><h1>H</h1></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(!findings.iter().any(|f| f.id == "seo/meta-title"));
    }

    #[test]
    fn missing_meta_description() {
        let html = "<html><head><title>T</title></head><body><h1>H</h1></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(findings.iter().any(|f| f.id == "seo/meta-description"));
    }

    #[test]
    fn has_meta_description() {
        let html =
            "<html><head><meta name=\"description\" content=\"x\"></head><body></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(!findings.iter().any(|f| f.id == "seo/meta-description"));
    }

    #[test]
    fn missing_canonical() {
        let html = "<html><head><title>T</title></head><body><h1>H</h1></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(findings.iter().any(|f| f.id == "seo/canonical"));
    }

    #[test]
    fn has_canonical() {
        let html = "<html><head><link rel=\"canonical\" href=\"https://x.com\"></head><body></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(!findings.iter().any(|f| f.id == "seo/canonical"));
    }

    #[test]
    fn missing_og_tags_partial() {
        // Has og:title but missing og:description and og:image.
        let html = "<html><head><meta property=\"og:title\" content=\"T\"></head><body></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        let og = findings.iter().find(|f| f.id == "seo/og-tags").unwrap();
        assert!(og.message.contains("og:description"));
        assert!(og.message.contains("og:image"));
        assert!(!og.message.contains("og:title"));
    }

    #[test]
    fn has_all_og_tags() {
        let html = r#"<html><head>
            <meta property="og:title" content="T">
            <meta property="og:description" content="D">
            <meta property="og:image" content="I">
        </head><body></body></html>"#;
        let findings = page_checks(html, "/p", "t.html");
        assert!(!findings.iter().any(|f| f.id == "seo/og-tags"));
    }

    #[test]
    fn missing_twitter_card() {
        let html = "<html><head></head><body></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(findings.iter().any(|f| f.id == "seo/twitter-tags"));
    }

    #[test]
    fn has_twitter_card() {
        let html = "<html><head><meta name=\"twitter:card\" content=\"summary\"></head><body></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(!findings.iter().any(|f| f.id == "seo/twitter-tags"));
    }

    #[test]
    fn missing_structured_data() {
        let html = "<html><head></head><body></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(findings.iter().any(|f| f.id == "seo/structured-data"));
    }

    #[test]
    fn has_structured_data() {
        let html = "<html><head><script type=\"application/ld+json\">{}</script></head><body></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(!findings.iter().any(|f| f.id == "seo/structured-data"));
    }

    #[test]
    fn no_h1() {
        let html = "<html><head></head><body><h2>Sub</h2></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        let h = findings
            .iter()
            .find(|f| f.id == "seo/heading-hierarchy")
            .unwrap();
        assert!(h.message.contains("no <h1>"));
    }

    #[test]
    fn one_h1() {
        let html = "<html><head></head><body><h1>Title</h1></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        assert!(!findings.iter().any(|f| f.id == "seo/heading-hierarchy"));
    }

    #[test]
    fn multiple_h1() {
        let html = "<html><head></head><body><h1>One</h1><h1>Two</h1><h1>Three</h1></body></html>";
        let findings = page_checks(html, "/p", "t.html");
        let h = findings
            .iter()
            .find(|f| f.id == "seo/heading-hierarchy")
            .unwrap();
        assert!(h.message.contains("multiple <h1>"));
        assert!(h.message.contains("3"));
    }

    #[test]
    fn page_checks_fix_file_matches_template() {
        let findings = page_checks(BAD_HTML, "/about.html", "templates/about.html");
        for f in &findings {
            assert_eq!(f.fix.file, "templates/about.html");
        }
    }

    #[test]
    fn page_checks_message_includes_page_path() {
        let findings = page_checks(BAD_HTML, "/about.html", "templates/about.html");
        for f in &findings {
            assert!(
                f.message.contains("/about.html"),
                "finding {} message should contain page path, got: {}",
                f.id,
                f.message
            );
        }
    }
}
