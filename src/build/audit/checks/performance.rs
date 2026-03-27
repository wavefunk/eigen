//! Performance audit checks.

use std::cell::RefCell;
use std::rc::Rc;

use super::super::{Category, Finding, Fix, Scope, Severity};
use crate::config::SiteConfig;

/// Site-level performance checks (config inspection).
pub fn site_checks(config: &SiteConfig) -> Vec<Finding> {
    let mut findings = Vec::new();

    // perf/image-optimization
    if !config.assets.images.optimize {
        findings.push(Finding {
            id: "perf/image-optimization",
            category: Category::Performance,
            severity: Severity::High,
            scope: Scope::Site,
            message: "Image optimization is disabled. Images will not be \
                      compressed or converted to modern formats."
                .into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set [assets.images] optimize = true".into(),
            },
        });
    }

    // perf/image-formats
    if config.assets.images.optimize && config.assets.images.formats.is_empty() {
        findings.push(Finding {
            id: "perf/image-formats",
            category: Category::Performance,
            severity: Severity::Medium,
            scope: Scope::Site,
            message: "Image optimization is enabled but no target formats \
                      are configured. No format conversion will occur."
                .into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set [assets.images] formats = [\"webp\", \"avif\"]".into(),
            },
        });
    }

    // perf/minification
    if !config.build.minify {
        findings.push(Finding {
            id: "perf/minification",
            category: Category::Performance,
            severity: Severity::High,
            scope: Scope::Site,
            message: "HTML minification is disabled. Output files will be \
                      larger than necessary."
                .into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set [build] minify = true".into(),
            },
        });
    }

    // perf/critical-css
    if !config.build.critical_css.enabled {
        findings.push(Finding {
            id: "perf/critical-css",
            category: Category::Performance,
            severity: Severity::Medium,
            scope: Scope::Site,
            message: "Critical CSS inlining is disabled. Above-the-fold \
                      content may be blocked by stylesheet loading."
                .into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set [build.critical_css] enabled = true".into(),
            },
        });
    }

    // perf/bundling
    if !config.build.bundling.enabled {
        findings.push(Finding {
            id: "perf/bundling",
            category: Category::Performance,
            severity: Severity::Medium,
            scope: Scope::Site,
            message: "CSS/JS bundling is disabled. Multiple separate files \
                      increase the number of HTTP requests."
                .into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set [build.bundling] enabled = true".into(),
            },
        });
    }

    // perf/content-hash
    if !config.build.content_hash.enabled {
        findings.push(Finding {
            id: "perf/content-hash",
            category: Category::Performance,
            severity: Severity::Low,
            scope: Scope::Site,
            message: "Content hashing is disabled. Static assets cannot use \
                      immutable cache headers."
                .into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set [build.content_hash] enabled = true".into(),
            },
        });
    }

    // perf/preload-hints
    if !config.build.hints.enabled {
        findings.push(Finding {
            id: "perf/preload-hints",
            category: Category::Performance,
            severity: Severity::Medium,
            scope: Scope::Site,
            message: "Resource preload/prefetch hints are disabled. The \
                      browser cannot begin fetching critical resources early."
                .into(),
            fix: Fix {
                file: "site.toml".into(),
                instruction: "Set [build.hints] enabled = true".into(),
            },
        });
    }

    findings
}

/// Font file extensions that should be preloaded.
const FONT_EXTENSIONS: &[&str] = &[".woff", ".woff2", ".ttf", ".otf"];

/// Page-level performance checks (HTML inspection).
pub fn page_checks(
    html: &str,
    page_path: &str,
    template_path: &str,
    config: &SiteConfig,
) -> Vec<Finding> {
    let findings: Rc<RefCell<Vec<Finding>>> = Rc::new(RefCell::new(Vec::new()));
    let in_head = Rc::new(RefCell::new(false));

    let mut handlers = Vec::new();

    // Track <head> entry.
    {
        let ih = in_head.clone();
        handlers.push(lol_html::element!("head", move |_el| {
            *ih.borrow_mut() = true;
            Ok(())
        }));
    }

    // Track <body> — signals that <head> is done.
    {
        let ih = in_head.clone();
        handlers.push(lol_html::element!("body", move |_el| {
            *ih.borrow_mut() = false;
            Ok(())
        }));
    }

    // perf/render-blocking-scripts: <script src="..."> in <head> without
    // defer or async.
    {
        let f = findings.clone();
        let ih = in_head.clone();
        let pp = page_path.to_owned();
        let tp = template_path.to_owned();
        handlers.push(lol_html::element!("script", move |el| {
            if !*ih.borrow() {
                return Ok(());
            }
            // Only flag external scripts (those with src attribute).
            let src = match el.get_attribute("src") {
                Some(s) => s,
                None => return Ok(()),
            };
            if el.get_attribute("defer").is_some() || el.get_attribute("async").is_some() {
                return Ok(());
            }
            f.borrow_mut().push(Finding {
                id: "perf/render-blocking-scripts",
                category: Category::Performance,
                severity: Severity::High,
                scope: Scope::Page,
                message: format!(
                    "{}: render-blocking script <script src=\"{}\"> in <head> \
                     without defer or async",
                    pp, src
                ),
                fix: Fix {
                    file: tp.clone(),
                    instruction: "Add defer or async attribute to the script tag".into(),
                },
            });
            Ok(())
        }));
    }

    // perf/large-image: <img> without srcset when image optimization is off.
    if !config.assets.images.optimize {
        let f = findings.clone();
        let pp = page_path.to_owned();
        let tp = template_path.to_owned();
        handlers.push(lol_html::element!("img", move |el| {
            if el.get_attribute("src").is_none() {
                return Ok(());
            }
            if el.get_attribute("srcset").is_some() {
                return Ok(());
            }
            f.borrow_mut().push(Finding {
                id: "perf/large-image",
                category: Category::Performance,
                severity: Severity::High,
                scope: Scope::Page,
                message: format!(
                    "{}: <img> without srcset attribute (image optimization \
                     is disabled)",
                    pp
                ),
                fix: Fix {
                    file: tp.clone(),
                    instruction: "Add a srcset attribute or enable image \
                                  optimization in site.toml"
                        .into(),
                },
            });
            Ok(())
        }));
    }

    // perf/font-preload: <link> with font href that lacks rel="preload",
    // only when hints are disabled.
    if !config.build.hints.enabled {
        let f = findings.clone();
        let pp = page_path.to_owned();
        let tp = template_path.to_owned();
        handlers.push(lol_html::element!("link", move |el| {
            let href = match el.get_attribute("href") {
                Some(h) => h,
                None => return Ok(()),
            };
            let is_font = FONT_EXTENSIONS.iter().any(|ext| href.ends_with(ext));
            if !is_font {
                return Ok(());
            }
            let rel = el.get_attribute("rel").unwrap_or_default();
            if rel == "preload" {
                return Ok(());
            }
            f.borrow_mut().push(Finding {
                id: "perf/font-preload",
                category: Category::Performance,
                severity: Severity::Medium,
                scope: Scope::Page,
                message: format!(
                    "{}: font file \"{}\" loaded without rel=\"preload\"",
                    pp, href
                ),
                fix: Fix {
                    file: tp.clone(),
                    instruction: "Add rel=\"preload\" as=\"font\" \
                                  crossorigin to the link tag, or enable \
                                  [build.hints] in site.toml"
                        .into(),
                },
            });
            Ok(())
        }));
    }

    // Run the rewriter (we discard the output — we only collect findings).
    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: handlers,
            ..lol_html::RewriteStrSettings::new()
        },
    );

    // Extract findings — Rc should have exactly one reference after rewrite_str completes.
    match Rc::try_unwrap(findings) {
        Ok(cell) => cell.into_inner(),
        Err(rc) => rc.borrow().clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteConfig;

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

    // ---------------------------------------------------------------
    // Site-level check tests
    // ---------------------------------------------------------------

    // perf/image-optimization

    #[test]
    fn image_optimization_disabled_flags() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [assets.images]
            optimize = false
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(findings.iter().any(|f| f.id == "perf/image-optimization"));
    }

    #[test]
    fn image_optimization_enabled_no_flag() {
        let config = minimal_config();
        let findings = site_checks(&config);
        assert!(!findings.iter().any(|f| f.id == "perf/image-optimization"));
    }

    // perf/image-formats

    #[test]
    fn image_formats_empty_flags() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [assets.images]
            optimize = true
            formats = []
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(findings.iter().any(|f| f.id == "perf/image-formats"));
    }

    #[test]
    fn image_formats_present_no_flag() {
        let config = minimal_config();
        let findings = site_checks(&config);
        // Default formats = ["webp", "avif"], so should NOT flag.
        assert!(!findings.iter().any(|f| f.id == "perf/image-formats"));
    }

    #[test]
    fn image_formats_not_flagged_when_optimization_off() {
        // If optimize is false and formats is empty, only the optimization
        // check fires — not the formats check.
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [assets.images]
            optimize = false
            formats = []
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(!findings.iter().any(|f| f.id == "perf/image-formats"));
    }

    // perf/minification

    #[test]
    fn minification_disabled_flags() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build]
            minify = false
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(findings.iter().any(|f| f.id == "perf/minification"));
    }

    #[test]
    fn minification_enabled_no_flag() {
        let config = minimal_config();
        let findings = site_checks(&config);
        assert!(!findings.iter().any(|f| f.id == "perf/minification"));
    }

    // perf/critical-css

    #[test]
    fn critical_css_disabled_flags() {
        // Default is disabled, so minimal config should flag.
        let config = minimal_config();
        let findings = site_checks(&config);
        assert!(findings.iter().any(|f| f.id == "perf/critical-css"));
    }

    #[test]
    fn critical_css_enabled_no_flag() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.critical_css]
            enabled = true
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(!findings.iter().any(|f| f.id == "perf/critical-css"));
    }

    // perf/bundling

    #[test]
    fn bundling_disabled_flags() {
        // Default is disabled.
        let config = minimal_config();
        let findings = site_checks(&config);
        assert!(findings.iter().any(|f| f.id == "perf/bundling"));
    }

    #[test]
    fn bundling_enabled_no_flag() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.bundling]
            enabled = true
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(!findings.iter().any(|f| f.id == "perf/bundling"));
    }

    // perf/content-hash

    #[test]
    fn content_hash_disabled_flags() {
        // Default is disabled.
        let config = minimal_config();
        let findings = site_checks(&config);
        assert!(findings.iter().any(|f| f.id == "perf/content-hash"));
    }

    #[test]
    fn content_hash_enabled_no_flag() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.content_hash]
            enabled = true
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(!findings.iter().any(|f| f.id == "perf/content-hash"));
    }

    // perf/preload-hints

    #[test]
    fn preload_hints_disabled_flags() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.hints]
            enabled = false
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(findings.iter().any(|f| f.id == "perf/preload-hints"));
    }

    #[test]
    fn preload_hints_enabled_no_flag() {
        // Default is enabled.
        let config = minimal_config();
        let findings = site_checks(&config);
        assert!(!findings.iter().any(|f| f.id == "perf/preload-hints"));
    }

    // ---------------------------------------------------------------
    // Page-level check tests
    // ---------------------------------------------------------------

    // perf/render-blocking-scripts

    #[test]
    fn render_blocking_script_in_head_flags() {
        let html = r#"<html><head><script src="/js/app.js"></script></head><body></body></html>"#;
        let config = minimal_config();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(findings
            .iter()
            .any(|f| f.id == "perf/render-blocking-scripts"));
    }

    #[test]
    fn deferred_script_in_head_no_flag() {
        let html =
            r#"<html><head><script src="/js/app.js" defer></script></head><body></body></html>"#;
        let config = minimal_config();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings
            .iter()
            .any(|f| f.id == "perf/render-blocking-scripts"));
    }

    #[test]
    fn async_script_in_head_no_flag() {
        let html =
            r#"<html><head><script src="/js/app.js" async></script></head><body></body></html>"#;
        let config = minimal_config();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings
            .iter()
            .any(|f| f.id == "perf/render-blocking-scripts"));
    }

    #[test]
    fn inline_script_in_head_no_flag() {
        let html =
            r#"<html><head><script>console.log("hi")</script></head><body></body></html>"#;
        let config = minimal_config();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings
            .iter()
            .any(|f| f.id == "perf/render-blocking-scripts"));
    }

    #[test]
    fn script_in_body_no_flag() {
        let html =
            r#"<html><head></head><body><script src="/js/app.js"></script></body></html>"#;
        let config = minimal_config();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings
            .iter()
            .any(|f| f.id == "perf/render-blocking-scripts"));
    }

    // perf/large-image

    #[test]
    fn img_without_srcset_flags_when_optimize_off() {
        let html =
            r#"<html><head></head><body><img src="/img/hero.jpg"></body></html>"#;
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [assets.images]
            optimize = false
        "#,
        )
        .unwrap();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(findings.iter().any(|f| f.id == "perf/large-image"));
    }

    #[test]
    fn img_with_srcset_no_flag_when_optimize_off() {
        let html = r#"<html><head></head><body><img src="/img/hero.jpg" srcset="/img/hero-480.jpg 480w, /img/hero-768.jpg 768w"></body></html>"#;
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [assets.images]
            optimize = false
        "#,
        )
        .unwrap();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings.iter().any(|f| f.id == "perf/large-image"));
    }

    #[test]
    fn img_without_srcset_no_flag_when_optimize_on() {
        let html =
            r#"<html><head></head><body><img src="/img/hero.jpg"></body></html>"#;
        let config = minimal_config(); // optimize defaults to true
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings.iter().any(|f| f.id == "perf/large-image"));
    }

    // perf/font-preload

    #[test]
    fn font_without_preload_flags_when_hints_off() {
        let html = r#"<html><head><link href="/fonts/inter.woff2" rel="stylesheet"></head><body></body></html>"#;
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.hints]
            enabled = false
        "#,
        )
        .unwrap();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(findings.iter().any(|f| f.id == "perf/font-preload"));
    }

    #[test]
    fn font_with_preload_no_flag_when_hints_off() {
        let html = r#"<html><head><link href="/fonts/inter.woff2" rel="preload" as="font" crossorigin></head><body></body></html>"#;
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.hints]
            enabled = false
        "#,
        )
        .unwrap();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings.iter().any(|f| f.id == "perf/font-preload"));
    }

    #[test]
    fn font_without_preload_no_flag_when_hints_on() {
        let html = r#"<html><head><link href="/fonts/inter.woff2" rel="stylesheet"></head><body></body></html>"#;
        let config = minimal_config(); // hints.enabled defaults to true
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings.iter().any(|f| f.id == "perf/font-preload"));
    }

    #[test]
    fn font_preload_checks_all_extensions() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.hints]
            enabled = false
        "#,
        )
        .unwrap();
        for ext in &[".woff", ".woff2", ".ttf", ".otf"] {
            let html = format!(
                r#"<html><head><link href="/fonts/font{}" rel="stylesheet"></head><body></body></html>"#,
                ext
            );
            let findings = page_checks(&html, "/index.html", "index.html", &config);
            assert!(
                findings.iter().any(|f| f.id == "perf/font-preload"),
                "Expected perf/font-preload for extension {}",
                ext
            );
        }
    }

    #[test]
    fn non_font_link_no_flag() {
        let html = r#"<html><head><link href="/css/style.css" rel="stylesheet"></head><body></body></html>"#;
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build.hints]
            enabled = false
        "#,
        )
        .unwrap();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        assert!(!findings.iter().any(|f| f.id == "perf/font-preload"));
    }

    // ---------------------------------------------------------------
    // Combined / integration tests
    // ---------------------------------------------------------------

    #[test]
    fn default_config_site_findings() {
        let config = minimal_config();
        let findings = site_checks(&config);
        let ids: Vec<&str> = findings.iter().map(|f| f.id).collect();

        // Defaults: optimize=true, formats=["webp","avif"], minify=true,
        // critical_css.enabled=false, bundling.enabled=false,
        // content_hash.enabled=false, hints.enabled=true.
        assert!(!ids.contains(&"perf/image-optimization"));
        assert!(!ids.contains(&"perf/image-formats"));
        assert!(!ids.contains(&"perf/minification"));
        assert!(ids.contains(&"perf/critical-css"));
        assert!(ids.contains(&"perf/bundling"));
        assert!(ids.contains(&"perf/content-hash"));
        assert!(!ids.contains(&"perf/preload-hints"));
    }

    #[test]
    fn all_enabled_config_no_site_findings() {
        let config: SiteConfig = toml::from_str(
            r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
            [build]
            minify = true
            [build.critical_css]
            enabled = true
            [build.bundling]
            enabled = true
            [build.content_hash]
            enabled = true
            [build.hints]
            enabled = true
            [assets.images]
            optimize = true
            formats = ["webp"]
        "#,
        )
        .unwrap();
        let findings = site_checks(&config);
        assert!(
            findings.is_empty(),
            "Expected no findings but got: {:?}",
            findings.iter().map(|f| f.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn multiple_render_blocking_scripts() {
        let html = r#"<html><head>
            <script src="/js/a.js"></script>
            <script src="/js/b.js"></script>
            <script src="/js/c.js" defer></script>
        </head><body></body></html>"#;
        let config = minimal_config();
        let findings = page_checks(html, "/index.html", "index.html", &config);
        let blocking: Vec<_> = findings
            .iter()
            .filter(|f| f.id == "perf/render-blocking-scripts")
            .collect();
        assert_eq!(blocking.len(), 2);
    }
}
