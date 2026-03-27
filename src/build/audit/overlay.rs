//! Audit overlay badge injection.
//!
//! Injects a small floating badge into rendered HTML pages during dev
//! builds showing the audit finding count per page. Clicking the badge
//! expands a panel listing each finding with severity and a link to the
//! full `/_audit` report.

use super::{AuditReport, Finding, Severity};
use crate::build::render::RenderedPage;
use eyre::Result;
use std::fmt::Write;
use std::path::Path;

/// Green color for the "all clear" badge.
const COLOR_SUCCESS: &str = "#16a34a";

/// Determine the badge background color from the worst severity present.
fn badge_color(findings: &[Finding]) -> &'static str {
    if findings.iter().any(|f| f.severity == Severity::Critical) {
        Severity::Critical.color()
    } else if findings.iter().any(|f| f.severity == Severity::High) {
        Severity::High.color()
    } else if findings.iter().any(|f| f.severity == Severity::Medium) {
        Severity::Medium.color()
    } else {
        COLOR_SUCCESS
    }
}

/// Generate the overlay HTML snippet (styles + script) for a set of findings.
///
/// The snippet contains a `<style>` block with all CSS prefixed by
/// `__eigen_audit_` and a `<script>` block that creates the badge element,
/// wires up click-to-expand, and persists collapsed/expanded state in
/// `localStorage`.
pub fn generate_overlay_script(findings: &[Finding]) -> String {
    let count = findings.len();
    let color = badge_color(findings);
    let label = if count == 1 {
        "1 issue".to_owned()
    } else {
        let mut s = String::with_capacity(8);
        let _ = write!(s, "{count} issues");
        s
    };

    // Build findings JSON array inline. All user-facing strings are escaped
    // to prevent breakout from the JSON or script context.
    let mut findings_json = String::from("[");
    for (i, f) in findings.iter().enumerate() {
        if i > 0 {
            findings_json.push(',');
        }
        let escaped_msg = f
            .message
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        let escaped_fix = f
            .fix
            .instruction
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        let _ = write!(
            findings_json,
            r#"{{"id":"{}","severity":"{}","severityColor":"{}","category":"{}","message":"{}","fix":"{}"}}"#,
            f.id,
            f.severity.display_name(),
            f.severity.color(),
            f.category.display_name(),
            escaped_msg,
            escaped_fix,
        );
    }
    findings_json.push(']');

    let mut out = String::with_capacity(4096);

    // --- Style block ---
    out.push_str(
        r#"<style>
.__eigen_audit_badge {
  position: fixed;
  bottom: 16px;
  right: 16px;
  z-index: 999999;
  padding: 6px 14px;
  border-radius: 20px;
  color: #fff;
  font: bold 13px/1 system-ui, sans-serif;
  cursor: pointer;
  box-shadow: 0 2px 8px rgba(0,0,0,0.25);
  user-select: none;
  transition: opacity 0.15s;
}
.__eigen_audit_badge:hover { opacity: 0.85; }
.__eigen_audit_panel {
  display: none;
  position: fixed;
  bottom: 52px;
  right: 16px;
  z-index: 999998;
  width: 380px;
  max-height: 60vh;
  overflow-y: auto;
  background: #1e1e2e;
  color: #cdd6f4;
  border-radius: 10px;
  box-shadow: 0 4px 24px rgba(0,0,0,0.4);
  font: 13px/1.5 system-ui, sans-serif;
  padding: 0;
}
.__eigen_audit_panel_open { display: block; }
.__eigen_audit_panel_header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  border-bottom: 1px solid #313244;
  font-weight: bold;
  font-size: 14px;
}
.__eigen_audit_panel_header a {
  color: #89b4fa;
  text-decoration: none;
  font-size: 12px;
}
.__eigen_audit_panel_header a:hover { text-decoration: underline; }
.__eigen_audit_panel_list {
  list-style: none;
  margin: 0;
  padding: 8px 0;
}
.__eigen_audit_panel_item {
  padding: 8px 16px;
  border-bottom: 1px solid #313244;
}
.__eigen_audit_panel_item:last-child { border-bottom: none; }
.__eigen_audit_sev_badge {
  display: inline-block;
  padding: 1px 6px;
  border-radius: 4px;
  color: #fff;
  font-size: 10px;
  font-weight: bold;
  margin-right: 6px;
  vertical-align: middle;
}
.__eigen_audit_msg {
  display: block;
  margin-top: 2px;
  color: #a6adc8;
  font-size: 12px;
}
</style>
"#,
    );

    // --- Script block ---
    //
    // The findings JSON is pre-escaped (no raw `<`, `>`, or unescaped quotes).
    // The panel is built from these trusted, escaped strings using DOM helpers
    // that construct elements and set textContent where possible. The severity
    // badge colors are hex literals from our own mapping, not user input.
    let _ = write!(
        out,
        r##"<script>
(function() {{
  var findings = {findings_json};
  var badge = document.createElement("div");
  badge.className = "__eigen_audit_badge";
  badge.style.background = "{color}";
  badge.textContent = "{label}";
  document.body.appendChild(badge);

  var panel = document.createElement("div");
  panel.className = "__eigen_audit_panel";

  var header = document.createElement("div");
  header.className = "__eigen_audit_panel_header";
  var title = document.createElement("span");
  title.textContent = "Audit Findings";
  var link = document.createElement("a");
  link.href = "/_audit";
  link.textContent = "Full Report \u2192";
  header.appendChild(title);
  header.appendChild(link);
  panel.appendChild(header);

  if (findings.length === 0) {{
    var empty = document.createElement("div");
    empty.style.cssText = "padding:16px;text-align:center;color:#a6adc8";
    empty.textContent = "No issues found on this page.";
    panel.appendChild(empty);
  }} else {{
    var list = document.createElement("ul");
    list.className = "__eigen_audit_panel_list";
    for (var i = 0; i < findings.length; i++) {{
      var f = findings[i];
      var li = document.createElement("li");
      li.className = "__eigen_audit_panel_item";
      var sev = document.createElement("span");
      sev.className = "__eigen_audit_sev_badge";
      sev.style.background = f.severityColor;
      sev.textContent = f.severity;
      li.appendChild(sev);
      var idEl = document.createElement("strong");
      idEl.textContent = f.id;
      li.appendChild(idEl);
      var msg = document.createElement("span");
      msg.className = "__eigen_audit_msg";
      msg.textContent = f.message;
      li.appendChild(msg);
      list.appendChild(li);
    }}
    panel.appendChild(list);
  }}

  document.body.appendChild(panel);

  var KEY = "__eigen_audit_panel_open";
  if (localStorage.getItem(KEY) === "true") {{
    panel.classList.add("__eigen_audit_panel_open");
  }}

  badge.addEventListener("click", function() {{
    var open = panel.classList.toggle("__eigen_audit_panel_open");
    localStorage.setItem(KEY, open ? "true" : "false");
  }});
}})();
</script>"##,
        findings_json = findings_json,
        color = color,
        label = label,
    );

    out
}

/// Inject the overlay snippet into rendered HTML before `</body>`.
///
/// Uses case-insensitive `rfind` to locate `</body>` and inserts the
/// overlay immediately before it. If no `</body>` is found the snippet
/// is appended at the end.
pub fn inject_overlay(html: &str, findings: &[Finding]) -> String {
    let snippet = generate_overlay_script(findings);
    let lower = html.to_lowercase();
    if let Some(pos) = lower.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + snippet.len() + 1);
        result.push_str(&html[..pos]);
        result.push('\n');
        result.push_str(&snippet);
        result.push('\n');
        result.push_str(&html[pos..]);
        result
    } else {
        let mut result = String::with_capacity(html.len() + snippet.len() + 1);
        result.push_str(html);
        result.push('\n');
        result.push_str(&snippet);
        result
    }
}

/// Inject audit overlay badges into all rendered page HTML files.
///
/// For each rendered page, looks up page-specific findings from the report,
/// reads the HTML file from `dist_path`, injects the overlay, and writes
/// the modified HTML back.
pub fn inject_badges(
    report: &AuditReport,
    dist_path: &Path,
    rendered_pages: &[RenderedPage],
) -> Result<()> {
    for page in rendered_pages {
        let findings: &[Finding] = report
            .page_findings
            .get(&page.url_path)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let html_path = dist_path.join(page.url_path.trim_start_matches('/'));
        let html = match std::fs::read_to_string(&html_path) {
            Ok(h) => h,
            Err(_) => continue,
        };

        let modified = inject_overlay(&html, findings);
        std::fs::write(&html_path, modified)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::audit::*;

    #[test]
    fn test_overlay_script_contains_badge() {
        let findings = vec![Finding {
            id: "seo/meta-title",
            category: Category::Seo,
            severity: Severity::High,
            scope: Scope::Page,
            message: "Missing title".into(),
            fix: Fix {
                file: "t.html".into(),
                instruction: "Add title".into(),
            },
        }];
        let script = generate_overlay_script(&findings);
        assert!(script.contains("__eigen_audit_"));
        assert!(script.contains("/_audit"));
    }

    #[test]
    fn test_inject_overlay_before_body() {
        let html = "<html><body><p>hi</p></body></html>";
        let result = inject_overlay(html, &[]);
        assert!(result.contains("__eigen_audit_"));
        let badge_pos = result.find("__eigen_audit_").unwrap();
        let body_pos = result.find("</body>").unwrap();
        assert!(badge_pos < body_pos);
    }

    #[test]
    fn test_inject_preserves_content() {
        let html = "<html><body><p>hello world</p></body></html>";
        let result = inject_overlay(html, &[]);
        assert!(result.contains("<p>hello world</p>"));
    }

    #[test]
    fn test_zero_findings_green() {
        let script = generate_overlay_script(&[]);
        assert!(
            script.contains("#16a34a") || script.contains("green") || script.contains("0 issues")
        );
    }

    #[test]
    fn test_badge_color_critical() {
        let findings = vec![Finding {
            id: "perf/large-image",
            category: Category::Performance,
            severity: Severity::Critical,
            scope: Scope::Page,
            message: "Image too large".into(),
            fix: Fix {
                file: "img.png".into(),
                instruction: "Compress".into(),
            },
        }];
        let script = generate_overlay_script(&findings);
        assert!(script.contains("#dc2626"));
    }

    #[test]
    fn test_badge_color_medium_only() {
        let findings = vec![Finding {
            id: "a11y/alt-text",
            category: Category::Accessibility,
            severity: Severity::Medium,
            scope: Scope::Page,
            message: "Missing alt".into(),
            fix: Fix {
                file: "page.html".into(),
                instruction: "Add alt".into(),
            },
        }];
        let script = generate_overlay_script(&findings);
        assert!(script.contains("#ca8a04"));
    }

    #[test]
    fn test_inject_badges_writes_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dist = tmp.path();
        std::fs::write(
            dist.join("index.html"),
            "<html><body><p>hi</p></body></html>",
        )
        .unwrap();

        let mut page_findings = std::collections::BTreeMap::new();
        page_findings.insert(
            "/index.html".to_string(),
            vec![Finding {
                id: "seo/meta-title",
                category: Category::Seo,
                severity: Severity::High,
                scope: Scope::Page,
                message: "Missing title".into(),
                fix: Fix {
                    file: "t.html".into(),
                    instruction: "Add title".into(),
                },
            }],
        );

        let report = AuditReport {
            summary: AuditSummary {
                total: 1,
                by_severity: std::collections::BTreeMap::new(),
                by_category: std::collections::BTreeMap::new(),
            },
            site_findings: vec![],
            page_findings,
        };

        let pages = vec![RenderedPage {
            url_path: "/index.html".into(),
            is_index: true,
            is_dynamic: false,
            template_path: None,
        }];

        inject_badges(&report, dist, &pages).unwrap();

        let modified = std::fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(modified.contains("__eigen_audit_"));
        assert!(modified.contains("1 issue"));
        assert!(modified.contains("<p>hi</p>"));
    }

    #[test]
    fn test_inject_overlay_no_body_tag() {
        let html = "<h1>Fragment</h1>";
        let result = inject_overlay(html, &[]);
        assert!(result.contains("__eigen_audit_"));
        assert!(result.contains("<h1>Fragment</h1>"));
    }

    #[test]
    fn test_message_escaping() {
        let findings = vec![Finding {
            id: "seo/test",
            category: Category::Seo,
            severity: Severity::Low,
            scope: Scope::Page,
            message: r#"Contains "quotes" and <tags>"#.into(),
            fix: Fix {
                file: "f.html".into(),
                instruction: "Fix it".into(),
            },
        }];
        let script = generate_overlay_script(&findings);
        // Should not contain raw < or > inside JSON
        assert!(!script.contains(r#""message":"Contains "quotes""#));
        assert!(script.contains(r#"\u003ctags\u003e"#));
    }

    #[test]
    fn test_single_issue_label() {
        let findings = vec![Finding {
            id: "seo/test",
            category: Category::Seo,
            severity: Severity::Low,
            scope: Scope::Page,
            message: "Test".into(),
            fix: Fix {
                file: "f.html".into(),
                instruction: "Fix".into(),
            },
        }];
        let script = generate_overlay_script(&findings);
        assert!(script.contains("1 issue"));
        assert!(!script.contains("1 issues"));
    }
}
