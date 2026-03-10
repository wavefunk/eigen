//! HTML minification using `minify-html`.
//!
//! Provides a thin wrapper that minifies rendered HTML (including inline
//! CSS in `<style>` tags / `style` attributes, and JavaScript in `<script>`
//! tags).  Used as the final transformation step before writing to disk.

use minify_html::{Cfg, minify};

/// Build a [`Cfg`] suitable for production HTML output.
///
/// Enables CSS and JS minification, strips comments, and minifies the
/// doctype.  Template-syntax preservation is off because all templates
/// have already been rendered by this point.
fn build_cfg() -> Cfg {
    let mut cfg = Cfg::new();
    cfg.minify_css = true;
    cfg.minify_js = true;
    cfg.minify_doctype = true;
    cfg.keep_comments = false;
    cfg.keep_ssi_comments = false;
    // These are safe optimizations accepted by all browsers.
    cfg.allow_noncompliant_unquoted_attribute_values = false;
    cfg.allow_optimal_entities = false;
    cfg.allow_removing_spaces_between_attributes = false;
    cfg
}

/// Minify an HTML string.
///
/// Returns the minified HTML as a `String`.  If the input is not valid
/// UTF-8 after minification (should never happen), falls back to returning
/// the original input unchanged.
pub fn minify_html(html: &str) -> String {
    let cfg = build_cfg();
    let minified = minify(html.as_bytes(), &cfg);
    String::from_utf8(minified).unwrap_or_else(|_| html.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minify_basic_html() {
        let input = r#"<!DOCTYPE html>
<html>
  <head>
    <title>Test</title>
  </head>
  <body>
    <p>  Hello,   world!  </p>
  </body>
</html>"#;
        let result = minify_html(input);
        // Whitespace inside <p> should be collapsed.
        assert!(result.contains("Hello,"));
        assert!(result.contains("world!"));
        // Should be smaller than input.
        assert!(result.len() < input.len());
    }

    #[test]
    fn test_minify_strips_comments() {
        let input = "<div><!-- this is a comment --><p>Hello</p></div>";
        let result = minify_html(input);
        assert!(!result.contains("<!-- this is a comment -->"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn test_minify_inline_css() {
        let input = r#"<style>
  body {
    color:   red;
    margin:  0;
  }
</style>"#;
        let result = minify_html(input);
        // CSS should be minified — no multi-line whitespace.
        assert!(result.contains("color:red"));
        assert!(!result.contains("   "));
    }

    #[test]
    fn test_minify_inline_js() {
        let input = r#"<script>
  function hello() {
    console.log("hello");
  }
</script>"#;
        let result = minify_html(input);
        // JS should be minified — smaller than input.
        assert!(result.len() < input.len());
        // Should still contain the essential code.
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_minify_preserves_content() {
        let input = "<p>Important text</p>";
        let result = minify_html(input);
        assert!(result.contains("Important text"));
    }

    #[test]
    fn test_minify_empty_input() {
        let result = minify_html("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_minify_picture_element_preserved() {
        // Ensure our <picture> elements from image optimization survive minification.
        let input = r#"<picture>
  <source srcset="/assets/hero-480w.avif 480w, /assets/hero-768w.avif 768w" type="image/avif">
  <source srcset="/assets/hero-480w.webp 480w, /assets/hero-768w.webp 768w" type="image/webp">
  <img src="/assets/hero.jpg" alt="Hero" class="main" loading="lazy">
</picture>"#;
        let result = minify_html(input);
        // All srcset values and attributes should survive.
        assert!(result.contains("srcset="));
        assert!(result.contains("480w"));
        assert!(result.contains("768w"));
        assert!(result.contains(r#"alt="Hero""#) || result.contains("alt=Hero"));
        assert!(result.contains("loading="));
    }

    #[test]
    fn test_minify_multiline_whitespace() {
        let input = "<div>\n\n\n    <p>   spaced   </p>\n\n</div>";
        let result = minify_html(input);
        assert!(result.len() < input.len());
        assert!(result.contains("spaced"));
    }
}
