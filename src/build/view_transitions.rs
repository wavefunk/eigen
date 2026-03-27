//! View Transitions API injection.
//!
//! Injects a `<meta name="view-transition">` tag, an inline script that
//! enables `htmx.config.globalViewTransitions`, and `view-transition-name`
//! styles on elements whose `id` matches a fragment block name.
//!
//! This makes HTMX partial swaps animate smoothly via the browser's
//! View Transitions API. Progressive enhancement: browsers without
//! support get instant swaps as before.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// The meta tag for cross-document view transitions.
const VIEW_TRANSITION_META: &str =
    r#"<meta name="view-transition" content="same-origin">"#;

/// The inline script that enables HTMX's built-in view transitions.
///
/// Waits for DOMContentLoaded to ensure HTMX is loaded. Guards on
/// both `htmx` existing and `document.startViewTransition` being
/// supported (progressive enhancement).
const VIEW_TRANSITION_SCRIPT: &str = r#"<script>
document.addEventListener("DOMContentLoaded", function() {
  if (typeof htmx !== "undefined" && document.startViewTransition) {
    htmx.config.globalViewTransitions = true;
  }
});
</script>"#;

/// Check if the HTML already contains a `<meta name="view-transition">` tag.
///
/// Uses lol_html selector matching, following the convention in
/// `seo::has_canonical_link` and `json_ld::has_existing_json_ld`.
fn has_view_transition_meta(html: &str) -> bool {
    let found: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_clone = found.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!(
                "meta[name='view-transition']",
                move |_el| {
                    *found_clone.borrow_mut() = true;
                    Ok(())
                }
            )],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    *found.borrow()
}

/// Build the HTML to inject into `<head>`.
fn build_head_injection(has_meta: bool) -> String {
    let mut out = String::new();
    if !has_meta {
        out.push_str(VIEW_TRANSITION_META);
        out.push('\n');
    }
    out.push_str(VIEW_TRANSITION_SCRIPT);
    out.push('\n');
    out
}

/// Inject content into `<head>` and add `view-transition-name` to
/// elements whose `id` matches a fragment block name.
fn rewrite_html(
    html: &str,
    head_html: &str,
    block_names: &HashSet<String>,
) -> Result<String, lol_html::errors::RewritingError> {
    let head_owned = head_html.to_string();
    let names = block_names.clone();

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                // Inject meta + script into <head>.
                lol_html::element!("head", move |el| {
                    el.append(
                        &head_owned,
                        lol_html::html_content::ContentType::Html,
                    );
                    Ok(())
                }),
                // Add view-transition-name to elements with matching IDs.
                lol_html::element!("*[id]", move |el| {
                    if let Some(id) = el.get_attribute("id") && names.contains(&id) {
                        let existing_style =
                            el.get_attribute("style").unwrap_or_default();

                        // Skip if already has a view-transition-name.
                        if existing_style.contains("view-transition-name") {
                            return Ok(());
                        }

                        let new_style = if existing_style.is_empty() {
                            format!("view-transition-name: {};", id)
                        } else {
                            let trimmed =
                                existing_style.trim_end().trim_end_matches(';');
                            format!(
                                "{}; view-transition-name: {};",
                                trimmed, id
                            )
                        };

                        if let Err(e) = el.set_attribute("style", &new_style) {
                            tracing::warn!(
                                "Failed to set style on #{}: {}",
                                id, e
                            );
                        }
                    }
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    )
}

/// Inject view transition meta tag, HTMX config script, and
/// transition names on fragment target elements.
///
/// `fragment_block_names` is the list of block names extracted from
/// the page's fragment markers. Elements with matching `id`
/// attributes get `view-transition-name` added to their `style`.
///
/// Infallible by design: returns original HTML on any error.
pub fn inject_view_transitions(
    html: &str,
    fragment_block_names: &[String],
) -> String {
    let has_meta = has_view_transition_meta(html);
    let head_html = build_head_injection(has_meta);

    let block_names: HashSet<String> =
        fragment_block_names.iter().cloned().collect();

    match rewrite_html(html, &head_html, &block_names) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("Failed to inject view transitions: {}", e);
            html.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_meta_tag() {
        let html = "<html><head><title>Test</title></head><body></body></html>";
        let result = inject_view_transitions(html, &[]);
        assert!(
            result.contains(r#"<meta name="view-transition" content="same-origin">"#),
            "Should inject view-transition meta tag"
        );
    }

    #[test]
    fn test_inject_script() {
        let html = "<html><head><title>Test</title></head><body></body></html>";
        let result = inject_view_transitions(html, &[]);
        assert!(
            result.contains("htmx.config.globalViewTransitions"),
            "Should inject HTMX globalViewTransitions script"
        );
        assert!(
            result.contains("DOMContentLoaded"),
            "Should use DOMContentLoaded to wait for HTMX"
        );
        assert!(
            result.contains("document.startViewTransition"),
            "Should guard with startViewTransition check"
        );
    }

    #[test]
    fn test_inject_transition_names() {
        let html = r#"<html><head></head><body><div id="content">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains(r#"style="view-transition-name: content;""#),
            "Should add view-transition-name to element with matching id. Got: {}", result
        );
    }

    #[test]
    fn test_existing_inline_style() {
        let html = r#"<html><head></head><body><div id="content" style="color: red;">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains("color: red; view-transition-name: content;"),
            "Should append to existing style. Got: {}", result
        );
    }

    #[test]
    fn test_existing_inline_style_trailing_semicolon() {
        let html = r#"<html><head></head><body><div id="content" style="color: red;">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            !result.contains(";;"),
            "Should not produce double semicolons. Got: {}", result
        );
    }

    #[test]
    fn test_existing_view_transition_name() {
        let html = r#"<html><head></head><body><div id="content" style="view-transition-name: custom;">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains("view-transition-name: custom;"),
            "Should preserve existing view-transition-name"
        );
        let count = result.matches("view-transition-name").count();
        assert_eq!(count, 1, "Should not duplicate view-transition-name. Got: {}", result);
    }

    #[test]
    fn test_no_head() {
        let html = "<div>Fragment content</div>";
        let result = inject_view_transitions(html, &["content".into()]);
        assert_eq!(result, html, "Should return unchanged when no <head>");
    }

    #[test]
    fn test_no_matching_ids() {
        let html = r#"<html><head></head><body><div id="other">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &["content".into()]);
        assert!(
            result.contains(r#"<meta name="view-transition""#),
            "Meta tag should still be injected"
        );
        assert!(
            !result.contains("view-transition-name"),
            "No transition name should be added when no IDs match"
        );
    }

    #[test]
    fn test_multiple_fragment_names() {
        let html = r#"<html><head></head><body><div id="content">Main</div><nav id="nav_header">Nav</nav></body></html>"#;
        let result = inject_view_transitions(
            html,
            &["content".into(), "nav_header".into()],
        );
        assert!(
            result.contains(r#"view-transition-name: content;"#),
            "Should add transition name to content. Got: {}", result
        );
        assert!(
            result.contains(r#"view-transition-name: nav_header;"#),
            "Should add transition name to nav_header. Got: {}", result
        );
    }

    #[test]
    fn test_existing_meta_tag() {
        let html = r#"<html><head><meta name="view-transition" content="same-origin"></head><body></body></html>"#;
        let result = inject_view_transitions(html, &[]);
        let count = result.matches(r#"<meta name="view-transition""#).count();
        assert_eq!(count, 1, "Should not duplicate meta tag. Got: {}", result);
    }

    #[test]
    fn test_fragments_disabled() {
        let html = r#"<html><head></head><body><div id="content">Hello</div></body></html>"#;
        let result = inject_view_transitions(html, &[]);
        assert!(
            result.contains(r#"<meta name="view-transition""#),
            "Meta tag should be injected even without block names"
        );
        assert!(
            result.contains("globalViewTransitions"),
            "Script should be injected even without block names"
        );
        assert!(
            !result.contains("view-transition-name"),
            "No transition names when no block names provided"
        );
    }
}
