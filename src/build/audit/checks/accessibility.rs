//! Accessibility audit checks.

use std::cell::RefCell;
use std::rc::Rc;

use super::super::{Category, Finding, Fix, Scope, Severity};

/// Site-level accessibility checks (advisory).
pub fn site_checks() -> Vec<Finding> {
    vec![Finding {
        id: "a11y/color-contrast-hint",
        category: Category::Accessibility,
        severity: Severity::Low,
        scope: Scope::Site,
        message: "Remember to verify color contrast ratios meet WCAG AA (4.5:1 for text)."
            .into(),
        fix: Fix {
            file: "templates/**/*.html".into(),
            instruction: "Use a contrast checker to verify all text meets WCAG AA ratios."
                .into(),
        },
    }]
}

/// Page-level accessibility checks (HTML inspection via lol_html).
pub fn page_checks(html: &str, page_path: &str, template_path: &str) -> Vec<Finding> {
    let has_lang = Rc::new(RefCell::new(false));
    let has_viewport = Rc::new(RefCell::new(false));
    let imgs_missing_alt: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    // For link-text: collect (href, has_aria_label) for each anchor,
    // then check accumulated text when the next anchor starts.
    let empty_links: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let current_link_text: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let current_link_href: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let current_link_has_aria = Rc::new(RefCell::new(false));

    let has_lang_c = has_lang.clone();
    let has_viewport_c = has_viewport.clone();
    let imgs_c = imgs_missing_alt.clone();

    // Clones for the anchor element handler.
    let link_text_c = current_link_text.clone();
    let link_href_c = current_link_href.clone();
    let link_aria_c = current_link_has_aria.clone();
    let empty_links_c = empty_links.clone();

    // Clone for text handler.
    let link_text_t = current_link_text.clone();

    /// Check the previous anchor's accumulated text and record if empty.
    fn flush_anchor(
        href: &Rc<RefCell<Option<String>>>,
        text: &Rc<RefCell<String>>,
        has_aria: &Rc<RefCell<bool>>,
        empty_links: &Rc<RefCell<Vec<String>>>,
    ) {
        if let Some(h) = href.borrow_mut().take() {
            if text.borrow().trim().is_empty() && !*has_aria.borrow() {
                empty_links.borrow_mut().push(h);
            }
        }
        text.borrow_mut().clear();
    }

    let result = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("html", move |el| {
                    if el.get_attribute("lang").is_some() {
                        *has_lang_c.borrow_mut() = true;
                    }
                    Ok(())
                }),
                lol_html::element!("meta[name='viewport']", move |el| {
                    if let Some(content) = el.get_attribute("content")
                        && content.contains("width=device-width")
                    {
                        *has_viewport_c.borrow_mut() = true;
                    }
                    Ok(())
                }),
                lol_html::element!("img", move |el| {
                    if !el.has_attribute("alt") {
                        let src = el.get_attribute("src").unwrap_or_default();
                        imgs_c.borrow_mut().push(src);
                    }
                    Ok(())
                }),
                // On each <a>, flush the previous anchor, then start tracking new one.
                lol_html::element!("a[href]", move |el| {
                    flush_anchor(&link_href_c, &link_text_c, &link_aria_c, &empty_links_c);
                    *link_href_c.borrow_mut() = el.get_attribute("href");
                    *link_aria_c.borrow_mut() = el.has_attribute("aria-label");
                    link_text_c.borrow_mut().clear();
                    Ok(())
                }),
                lol_html::text!("a", move |text| {
                    link_text_t.borrow_mut().push_str(text.as_str());
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    // If lol_html fails, return empty — don't crash the audit.
    if let Err(e) = result {
        tracing::warn!("Accessibility audit: lol_html parsing failed: {e}");
        return Vec::new();
    }

    // Flush the last anchor (if any).
    flush_anchor(
        &current_link_href,
        &current_link_text,
        &current_link_has_aria,
        &empty_links,
    );

    let mut findings = Vec::new();

    if !*has_lang.borrow() {
        findings.push(Finding {
            id: "a11y/html-lang",
            category: Category::Accessibility,
            severity: Severity::Critical,
            scope: Scope::Page,
            message: format!("{page_path}: <html> element is missing the `lang` attribute."),
            fix: Fix {
                file: template_path.into(),
                instruction: "Add lang attribute: <html lang=\"en\">".into(),
            },
        });
    }

    if !*has_viewport.borrow() {
        findings.push(Finding {
            id: "a11y/viewport-meta",
            category: Category::Accessibility,
            severity: Severity::Critical,
            scope: Scope::Page,
            message: format!(
                "{page_path}: missing <meta name=\"viewport\"> with width=device-width."
            ),
            fix: Fix {
                file: template_path.into(),
                instruction: "Add to <head>: <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">".into(),
            },
        });
    }

    for src in imgs_missing_alt.borrow().iter() {
        findings.push(Finding {
            id: "a11y/img-alt-text",
            category: Category::Accessibility,
            severity: Severity::High,
            scope: Scope::Page,
            message: format!("{page_path}: <img src=\"{src}\"> is missing an `alt` attribute."),
            fix: Fix {
                file: template_path.into(),
                instruction: format!(
                    "Add alt text: <img src=\"{src}\" alt=\"descriptive text\">. \
                     Use alt=\"\" for decorative images."
                ),
            },
        });
    }

    for href in empty_links.borrow().iter() {
        findings.push(Finding {
            id: "a11y/link-text",
            category: Category::Accessibility,
            severity: Severity::Medium,
            scope: Scope::Page,
            message: format!(
                "{page_path}: <a href=\"{href}\"> has no accessible text content."
            ),
            fix: Fix {
                file: template_path.into(),
                instruction: "Add visible link text or an aria-label attribute.".into(),
            },
        });
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_contrast_hint_always_emitted() {
        let findings = site_checks();
        assert!(findings.iter().any(|f| f.id == "a11y/color-contrast-hint"));
    }

    #[test]
    fn test_missing_html_lang() {
        let html = "<html><head></head><body></body></html>";
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(findings.iter().any(|f| f.id == "a11y/html-lang"));
    }

    #[test]
    fn test_has_html_lang() {
        let html = r#"<html lang="en"><head></head><body></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "a11y/html-lang"));
    }

    #[test]
    fn test_missing_viewport() {
        let html = "<html><head></head><body></body></html>";
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(findings.iter().any(|f| f.id == "a11y/viewport-meta"));
    }

    #[test]
    fn test_has_viewport() {
        let html = r#"<html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "a11y/viewport-meta"));
    }

    #[test]
    fn test_img_missing_alt() {
        let html = r#"<html><head></head><body><img src="photo.jpg"></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(findings.iter().any(|f| f.id == "a11y/img-alt-text"));
    }

    #[test]
    fn test_img_has_alt() {
        let html =
            r#"<html><head></head><body><img src="photo.jpg" alt="A photo"></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "a11y/img-alt-text"));
    }

    #[test]
    fn test_img_empty_alt_decorative() {
        let html =
            r#"<html><head></head><body><img src="spacer.gif" alt=""></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "a11y/img-alt-text"));
    }

    #[test]
    fn test_empty_link_text() {
        let html = r#"<html><head></head><body><a href="/foo"></a></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(findings.iter().any(|f| f.id == "a11y/link-text"));
    }

    #[test]
    fn test_link_with_text() {
        let html =
            r#"<html><head></head><body><a href="/foo">Click here</a></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "a11y/link-text"));
    }

    #[test]
    fn test_link_with_aria_label_no_text() {
        let html = r#"<html><head></head><body><a href="/foo" aria-label="Home"></a></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(!findings.iter().any(|f| f.id == "a11y/link-text"));
    }

    #[test]
    fn test_viewport_missing_width_device_width() {
        let html = r#"<html><head><meta name="viewport" content="initial-scale=1"></head><body></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        assert!(
            findings.iter().any(|f| f.id == "a11y/viewport-meta"),
            "viewport without width=device-width should be flagged"
        );
    }

    #[test]
    fn test_multiple_imgs_some_missing_alt() {
        let html = r#"<html><head></head><body><img src="a.jpg" alt="ok"><img src="b.jpg"><img src="c.jpg" alt=""></body></html>"#;
        let findings = page_checks(html, "/test.html", "templates/test.html");
        let alt_findings: Vec<_> =
            findings.iter().filter(|f| f.id == "a11y/img-alt-text").collect();
        assert_eq!(alt_findings.len(), 1, "only b.jpg should be flagged");
    }
}
