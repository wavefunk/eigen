//! Step 7.4: Live reload client injection.
//!
//! During dev builds, injects a small `<script>` tag before `</body>` that
//! connects to the `/_reload` SSE endpoint and reloads the page on events.
//! This is never active during `eigen build` — only during `eigen dev`.

/// The live-reload script injected into every full HTML page during dev mode.
const RELOAD_SCRIPT: &str = r#"<script>
  const es = new EventSource("/_reload");
  es.addEventListener("reload", () => window.location.reload());
  es.onerror = () => setTimeout(() => window.location.reload(), 1000);
</script>"#;

/// Inject the live-reload script into rendered HTML.
///
/// Inserts the script immediately before `</body>`. If `</body>` is not
/// found, the script is appended at the end.
pub fn inject_reload_script(html: &str) -> String {
    // Case-insensitive search for </body>.
    let lower = html.to_lowercase();
    if let Some(pos) = lower.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + RELOAD_SCRIPT.len() + 1);
        result.push_str(&html[..pos]);
        result.push('\n');
        result.push_str(RELOAD_SCRIPT);
        result.push('\n');
        result.push_str(&html[pos..]);
        result
    } else {
        // No </body> tag — append at end.
        format!("{}\n{}", html, RELOAD_SCRIPT)
    }
}

/// Inject a status banner (e.g. "DRAFT" or "SCHEDULED: 2026-04-01")
/// into rendered HTML during dev mode.
///
/// The banner is a fixed-position div at the bottom of the viewport.
/// If `label` is empty, returns the HTML unchanged.
pub fn inject_status_banner(html: &str, label: &str) -> String {
    if label.is_empty() {
        return html.to_string();
    }

    let banner = format!(
        r#"<div id="eigen-draft-banner" style="position:fixed;bottom:0;left:0;right:0;background:#b91c1c;color:#fff;text-align:center;padding:6px 12px;font:14px/1.4 system-ui;z-index:99999;">{}</div>"#,
        label
    );

    let lower = html.to_lowercase();
    if let Some(pos) = lower.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + banner.len() + 1);
        result.push_str(&html[..pos]);
        result.push('\n');
        result.push_str(&banner);
        result.push('\n');
        result.push_str(&html[pos..]);
        result
    } else {
        format!("{}\n{}", html, banner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_before_body_close() {
        let html = "<html><body><h1>Hi</h1></body></html>";
        let result = inject_reload_script(html);
        assert!(result.contains("EventSource"));
        assert!(result.contains("/_reload"));
        // Script should appear before </body>.
        let script_pos = result.find("EventSource").unwrap();
        let body_close_pos = result.find("</body>").unwrap();
        assert!(script_pos < body_close_pos);
    }

    #[test]
    fn test_inject_case_insensitive() {
        let html = "<html><body><p>test</p></BODY></html>";
        let result = inject_reload_script(html);
        assert!(result.contains("EventSource"));
    }

    #[test]
    fn test_inject_no_body_tag() {
        let html = "<h1>Fragment</h1>";
        let result = inject_reload_script(html);
        assert!(result.contains("EventSource"));
        assert!(result.ends_with("</script>"));
    }

    #[test]
    fn test_inject_preserves_content() {
        let html = "<html><body><h1>Hello</h1></body></html>";
        let result = inject_reload_script(html);
        assert!(result.contains("<h1>Hello</h1>"));
        assert!(result.contains("</body></html>"));
    }

    #[test]
    fn test_inject_status_banner_draft() {
        let html = "<html><body><h1>Hi</h1></body></html>";
        let result = inject_status_banner(html, "DRAFT");
        assert!(result.contains("DRAFT"));
        assert!(result.contains("eigen-draft-banner"));
        let banner_pos = result.find("eigen-draft-banner").unwrap();
        let body_close_pos = result.find("</body>").unwrap();
        assert!(banner_pos < body_close_pos);
    }

    #[test]
    fn test_inject_status_banner_scheduled() {
        let html = "<html><body><h1>Hi</h1></body></html>";
        let result = inject_status_banner(html, "SCHEDULED: 2026-04-01");
        assert!(result.contains("SCHEDULED: 2026-04-01"));
        assert!(result.contains("eigen-draft-banner"));
    }

    #[test]
    fn test_inject_status_banner_empty_label() {
        let html = "<html><body><h1>Hi</h1></body></html>";
        let result = inject_status_banner(html, "");
        assert_eq!(result, html, "Empty label should not inject a banner");
    }

    #[test]
    fn test_inject_status_banner_no_body() {
        let html = "<h1>Fragment</h1>";
        let result = inject_status_banner(html, "DRAFT");
        assert!(result.contains("DRAFT"));
    }
}
