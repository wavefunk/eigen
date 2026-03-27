//! Fragment extraction from rendered HTML.
//!
//! After rendering a full page, we extract content between the injected
//! `<!--FRAG:name:START-->` and `<!--FRAG:name:END-->` markers. The extracted
//! content is written as standalone HTML fragments for HTMX partial loading.
//!
//! The markers are stripped from the full page output.

use eyre::{Result, WrapErr};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Regex matching fragment START markers: `<!--FRAG:name:START-->`.
static FRAG_START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!--FRAG:(\w+):START-->").unwrap());

/// Regex matching all fragment markers (both START and END).
static FRAG_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!--FRAG:\w+:(?:START|END)-->").unwrap());

/// A single extracted fragment.
#[derive(Debug)]
pub struct Fragment {
    /// The block name, e.g. `"content"`.
    pub block_name: String,
    /// The HTML content between the markers.
    pub html: String,
}

/// Extract all fragments from rendered HTML.
///
/// Finds all `<!--FRAG:name:START-->...<!--FRAG:name:END-->` pairs and returns
/// the content between them.
pub fn extract_fragments(html: &str) -> Vec<Fragment> {
    // Rust's regex crate doesn't support backreferences, so we use a simple
    // state-machine approach: find each START marker, then search for the
    // matching END marker with the same name.
    let mut fragments = Vec::new();

    for cap in FRAG_START_RE.captures_iter(html) {
        let block_name = cap[1].to_string();
        let start_match = cap.get(0).unwrap();
        let content_start = start_match.end();

        let end_marker = format!("<!--FRAG:{}:END-->", block_name);
        if let Some(end_pos) = html[content_start..].find(&end_marker) {
            let content = &html[content_start..content_start + end_pos];
            // Strip any nested fragment markers from the extracted content.
            let clean = strip_fragment_markers(content);
            fragments.push(Fragment {
                block_name,
                html: clean,
            });
        }
    }

    fragments
}

/// Strip all fragment markers from the full page HTML.
///
/// The markers should not appear in the final output served to users.
pub fn strip_fragment_markers(html: &str) -> String {
    FRAG_MARKER_RE.replace_all(html, "").to_string()
}

/// Extract fragment block names from rendered HTML (before marker stripping).
///
/// Returns the block names found in `<!--FRAG:name:START-->` markers.
/// Used by view transitions to know which element IDs to add
/// `view-transition-name` to.
pub fn extract_block_names(html: &str) -> Vec<String> {
    FRAG_START_RE.captures_iter(html)
        .map(|cap| cap[1].to_string())
        .collect()
}

/// Compute the output path for a fragment file.
///
/// - Default content block: mirrors the page path.
///   `about.html` + `"content"` → `_fragments/about.html`
/// - Non-default block: uses a subdirectory.
///   `about.html` + `"sidebar"` → `_fragments/about/sidebar.html`
pub fn fragment_output_path(
    page_output_path: &Path,
    block_name: &str,
    content_block: &str,
    fragment_dir: &str,
) -> PathBuf {
    let page_str = page_output_path.to_string_lossy();
    let clean = page_str.trim_start_matches('/');

    // Derive a stem that works for both flat (`about.html`) and clean URL
    // (`about/index.html`) output paths. For clean URLs the stem is the
    // parent directory (`about`); for flat paths it's the filename minus `.html`.
    let stem = if clean.ends_with("/index.html") {
        clean.trim_end_matches("/index.html")
    } else {
        clean.strip_suffix(".html").unwrap_or(clean)
    };

    if block_name == content_block {
        PathBuf::from(fragment_dir).join(clean)
    } else {
        PathBuf::from(fragment_dir).join(format!("{}/{}.html", stem, block_name))
    }
}

/// Inject `hx-swap-oob="outerHTML"` into the first HTML element of a fragment.
///
/// Given `<header id="x" class="y">...</header>`, returns
/// `<header id="x" class="y" hx-swap-oob="outerHTML">...</header>`.
fn inject_oob_attr(html: &str) -> String {
    // Find the end of the first opening tag (the first '>' that isn't inside a comment).
    let trimmed = html.trim_start();
    if let Some(pos) = trimmed.find('>') {
        // Check for self-closing tag: insert before "/>" or before ">"
        let insert_pos = if pos > 0 && trimmed.as_bytes()[pos - 1] == b'/' {
            pos - 1
        } else {
            pos
        };
        let mut result = String::with_capacity(trimmed.len() + 30);
        result.push_str(&trimmed[..insert_pos]);
        result.push_str(" hx-swap-oob=\"outerHTML\"");
        result.push_str(&trimmed[insert_pos..]);
        result
    } else {
        html.to_string()
    }
}

/// Write fragment files to disk.
///
/// `page_output_path` is relative to `dist/`, e.g. `about.html` or `posts/my-post.html`.
///
/// When `oob_blocks` is non-empty, fragments whose block name appears in
/// `oob_blocks` are appended to the content block fragment with
/// `hx-swap-oob="outerHTML"` injected into their root element.  This enables
/// HTMX out-of-band swaps so that elements outside the main swap target (e.g.
/// the navigation header) are updated on every client-side navigation.
pub fn write_fragments(
    dist_dir: &Path,
    page_output_path: &Path,
    fragments: &[Fragment],
    content_block: &str,
    fragment_dir: &str,
    oob_blocks: &[String],
) -> Result<()> {
    // Collect OOB fragment HTML (with attribute injected).
    let oob_html: String = if !oob_blocks.is_empty() {
        fragments
            .iter()
            .filter(|f| oob_blocks.contains(&f.block_name))
            .map(|f| inject_oob_attr(&f.html))
            .collect::<Vec<_>>()
            .join("")
    } else {
        String::new()
    };

    for fragment in fragments {
        // Skip OOB-only fragments — they are merged into the content block.
        if oob_blocks.contains(&fragment.block_name) {
            continue;
        }

        let mut html = fragment.html.clone();

        // Append OOB blocks to the content fragment.
        if fragment.block_name == content_block && !oob_html.is_empty() {
            html.push_str(&oob_html);
        }

        let frag_path = fragment_output_path(page_output_path, &fragment.block_name, content_block, fragment_dir);
        let full_path = dist_dir.join(&frag_path);

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .wrap_err_with(|| format!("Failed to create fragment dir {}", parent.display()))?;
        }

        std::fs::write(&full_path, &html)
            .wrap_err_with(|| format!("Failed to write fragment {}", full_path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_extract_single_fragment() {
        let html = "<html><body><!--FRAG:content:START--><h1>Hi</h1><!--FRAG:content:END--></body></html>";
        let frags = extract_fragments(html);
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].block_name, "content");
        assert_eq!(frags[0].html, "<h1>Hi</h1>");
    }

    #[test]
    fn test_extract_multiple_fragments() {
        let html = concat!(
            "<!--FRAG:title:START-->My Title<!--FRAG:title:END-->",
            "<!--FRAG:content:START--><p>Body</p><!--FRAG:content:END-->",
        );
        let frags = extract_fragments(html);
        assert_eq!(frags.len(), 2);
        assert_eq!(frags[0].block_name, "title");
        assert_eq!(frags[0].html, "My Title");
        assert_eq!(frags[1].block_name, "content");
        assert_eq!(frags[1].html, "<p>Body</p>");
    }

    #[test]
    fn test_extract_no_fragments() {
        let html = "<html><body>No markers here</body></html>";
        let frags = extract_fragments(html);
        assert!(frags.is_empty());
    }

    #[test]
    fn test_extract_multiline_fragment() {
        let html = "<!--FRAG:content:START-->\n<h1>Title</h1>\n<p>Body</p>\n<!--FRAG:content:END-->";
        let frags = extract_fragments(html);
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].html, "\n<h1>Title</h1>\n<p>Body</p>\n");
    }

    #[test]
    fn test_strip_markers() {
        let html = "BEFORE<!--FRAG:content:START-->MID<!--FRAG:content:END-->AFTER";
        let stripped = strip_fragment_markers(html);
        assert_eq!(stripped, "BEFOREMIDAFTER");
    }

    #[test]
    fn test_strip_markers_multiple() {
        let html = "<!--FRAG:a:START-->X<!--FRAG:a:END-->-<!--FRAG:b:START-->Y<!--FRAG:b:END-->";
        let stripped = strip_fragment_markers(html);
        assert_eq!(stripped, "X-Y");
    }

    #[test]
    fn test_strip_markers_none() {
        let html = "<html><body>No markers</body></html>";
        let stripped = strip_fragment_markers(html);
        assert_eq!(stripped, html);
    }

    #[test]
    fn test_fragment_output_path_content_block() {
        let path = fragment_output_path(
            Path::new("about.html"),
            "content",
            "content",
            "_fragments",
        );
        assert_eq!(path, PathBuf::from("_fragments/about.html"));
    }

    #[test]
    fn test_fragment_output_path_nested_page() {
        let path = fragment_output_path(
            Path::new("posts/my-post.html"),
            "content",
            "content",
            "_fragments",
        );
        assert_eq!(path, PathBuf::from("_fragments/posts/my-post.html"));
    }

    #[test]
    fn test_fragment_output_path_non_content_block() {
        let path = fragment_output_path(
            Path::new("about.html"),
            "sidebar",
            "content",
            "_fragments",
        );
        assert_eq!(path, PathBuf::from("_fragments/about/sidebar.html"));
    }

    #[test]
    fn test_fragment_output_path_custom_fragment_dir() {
        let path = fragment_output_path(
            Path::new("index.html"),
            "content",
            "content",
            "_frags",
        );
        assert_eq!(path, PathBuf::from("_frags/index.html"));
    }

    #[test]
    fn test_write_fragments() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(dist.join("_fragments")).unwrap();

        let frags = vec![
            Fragment {
                block_name: "content".into(),
                html: "<h1>Hi</h1>".into(),
            },
            Fragment {
                block_name: "sidebar".into(),
                html: "<aside>Side</aside>".into(),
            },
        ];

        write_fragments(
            &dist,
            Path::new("about.html"),
            &frags,
            "content",
            "_fragments",
            &[],
        ).unwrap();

        assert!(dist.join("_fragments/about.html").exists());
        assert!(dist.join("_fragments/about/sidebar.html").exists());

        let content = fs::read_to_string(dist.join("_fragments/about.html")).unwrap();
        assert_eq!(content, "<h1>Hi</h1>");

        let sidebar = fs::read_to_string(dist.join("_fragments/about/sidebar.html")).unwrap();
        assert_eq!(sidebar, "<aside>Side</aside>");
    }

    #[test]
    fn test_inject_oob_attr() {
        assert_eq!(
            inject_oob_attr(r#"<header id="nav" class="foo">content</header>"#),
            r#"<header id="nav" class="foo" hx-swap-oob="outerHTML">content</header>"#,
        );
    }

    #[test]
    fn test_inject_oob_attr_self_closing() {
        assert_eq!(
            inject_oob_attr(r#"<br/>"#),
            r#"<br hx-swap-oob="outerHTML"/>"#,
        );
    }

    #[test]
    fn test_write_fragments_with_oob() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path().join("dist");
        fs::create_dir_all(dist.join("_fragments")).unwrap();

        let frags = vec![
            Fragment {
                block_name: "content".into(),
                html: "<h1>Hi</h1>".into(),
            },
            Fragment {
                block_name: "nav_header".into(),
                html: r#"<header id="nav">Nav</header>"#.into(),
            },
        ];

        write_fragments(
            &dist,
            Path::new("about.html"),
            &frags,
            "content",
            "_fragments",
            &["nav_header".to_string()],
        ).unwrap();

        // Content fragment should include OOB block appended.
        let content = fs::read_to_string(dist.join("_fragments/about.html")).unwrap();
        assert_eq!(content, r#"<h1>Hi</h1><header id="nav" hx-swap-oob="outerHTML">Nav</header>"#);

        // OOB block should NOT be written as a separate fragment file.
        assert!(!dist.join("_fragments/about/nav_header.html").exists());
    }

    #[test]
    fn test_extract_block_names() {
        let html = "<!--FRAG:content:START--><h1>Hi</h1><!--FRAG:content:END--><!--FRAG:nav:START-->Nav<!--FRAG:nav:END-->";
        let names = extract_block_names(html);
        assert_eq!(names, vec!["content", "nav"]);
    }

    #[test]
    fn test_extract_block_names_empty() {
        let html = "<html><body>No markers</body></html>";
        let names = extract_block_names(html);
        assert!(names.is_empty());
    }
}
