use super::super::{AuditReport, Category, Finding, Severity};
use eyre::Result;
use std::fmt::Write;
use std::path::Path;

fn write_severity_badge(out: &mut String, severity: &Severity) {
    let _ = write!(
        out,
        r#"<span class="__eigen_audit_badge" style="background:{color}">{name}</span>"#,
        color = severity.color(),
        name = severity.display_name(),
    );
}

fn write_finding(out: &mut String, finding: &Finding) {
    let _ = write!(
        out,
        r#"<div class="__eigen_audit_finding" data-severity="{sev}" data-category="{cat}">"#,
        sev = finding.severity.display_name().to_ascii_lowercase(),
        cat = finding.category.display_name().to_ascii_lowercase(),
    );
    write_severity_badge(out, &finding.severity);
    let _ = write!(
        out,
        r#" <code class="__eigen_audit_id">{id}</code>
<p class="__eigen_audit_message">{msg}</p>
<div class="__eigen_audit_fix"><strong>Fix:</strong> <code>{file}</code> &mdash; {instruction}</div>
</div>"#,
        id = finding.id,
        msg = html_escape(&finding.message),
        file = html_escape(&finding.fix.file),
        instruction = html_escape(&finding.fix.instruction),
    );
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

const CSS: &str = r#"
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:system-ui,-apple-system,sans-serif;line-height:1.6;padding:2rem;background:#ffffff;color:#1a1a2e}
@media(prefers-color-scheme:dark){
  body{background:#1a1a2e;color:#e0e0e0}
  .__eigen_audit_finding{background:#2a2a4a;border-color:#3a3a5a}
  .__eigen_audit_fix{background:#2a2a4a}
  code{background:#3a3a5a}
}
.__eigen_audit_container{max-width:960px;margin:0 auto}
.__eigen_audit_container h1{font-size:1.75rem;margin-bottom:1rem}
.__eigen_audit_summary{display:flex;gap:.75rem;flex-wrap:wrap;margin-bottom:2rem}
.__eigen_audit_summary_item{padding:.5rem 1rem;border-radius:.375rem;color:#fff;font-weight:600;font-size:.9rem}
.__eigen_audit_badge{display:inline-block;padding:.125rem .5rem;border-radius:.25rem;color:#fff;font-size:.75rem;font-weight:700;vertical-align:middle;margin-right:.5rem}
.__eigen_audit_category{margin-bottom:2rem}
.__eigen_audit_category h2{font-size:1.35rem;margin-bottom:.75rem;border-bottom:2px solid currentColor;padding-bottom:.25rem}
.__eigen_audit_page_group h3{font-size:1rem;margin:.75rem 0 .5rem;color:#6b7280}
@media(prefers-color-scheme:dark){.__eigen_audit_page_group h3{color:#9ca3af}}
.__eigen_audit_finding{border:1px solid #e5e7eb;border-radius:.5rem;padding:1rem;margin-bottom:.75rem}
.__eigen_audit_id{font-size:.85rem;font-weight:600}
.__eigen_audit_message{margin:.5rem 0}
.__eigen_audit_fix{font-size:.85rem;padding:.5rem;border-radius:.25rem;background:#f9fafb;margin-top:.5rem}
.__eigen_audit_filters{display:flex;gap:.5rem;flex-wrap:wrap;margin-bottom:1.5rem}
.__eigen_audit_filters button{padding:.375rem .75rem;border:1px solid #d1d5db;border-radius:.25rem;cursor:pointer;font-size:.8rem;background:#fff;color:#374151}
.__eigen_audit_filters button.active{background:#2563eb;color:#fff;border-color:#2563eb}
@media(prefers-color-scheme:dark){
  .__eigen_audit_filters button{background:#2a2a4a;color:#e0e0e0;border-color:#3a3a5a}
  .__eigen_audit_filters button.active{background:#2563eb;color:#fff;border-color:#2563eb}
}
.__eigen_audit_pass{text-align:center;padding:3rem;font-size:1.25rem;color:#16a34a;font-weight:600}
"#;

const JS: &str = r#"
(function(){
  var activeFilters = { severity: null, category: null };

  function applyFilters() {
    var findings = document.querySelectorAll('.__eigen_audit_finding');
    findings.forEach(function(el) {
      var show = true;
      if (activeFilters.severity && el.dataset.severity !== activeFilters.severity) show = false;
      if (activeFilters.category && el.dataset.category !== activeFilters.category) show = false;
      el.style.display = show ? '' : 'none';
    });
    // Hide empty category sections and page groups
    document.querySelectorAll('.__eigen_audit_category').forEach(function(sec) {
      var visible = sec.querySelectorAll('.__eigen_audit_finding:not([style*="display: none"])');
      sec.style.display = visible.length ? '' : 'none';
    });
    document.querySelectorAll('.__eigen_audit_page_group').forEach(function(pg) {
      var visible = pg.querySelectorAll('.__eigen_audit_finding:not([style*="display: none"])');
      pg.style.display = visible.length ? '' : 'none';
    });
  }

  document.querySelectorAll('.__eigen_audit_filters button').forEach(function(btn) {
    btn.addEventListener('click', function() {
      var kind = btn.dataset.filterKind;
      var value = btn.dataset.filterValue;
      if (activeFilters[kind] === value) {
        activeFilters[kind] = null;
        btn.classList.remove('active');
      } else {
        document.querySelectorAll('[data-filter-kind="' + kind + '"]').forEach(function(b) { b.classList.remove('active'); });
        activeFilters[kind] = value;
        btn.classList.add('active');
      }
      applyFilters();
    });
  });
})();
"#;

/// All categories in display order.
const ALL_CATEGORIES: [Category; 4] = [
    Category::Seo,
    Category::Performance,
    Category::Accessibility,
    Category::BestPractices,
];

/// All severities in display order.
const ALL_SEVERITIES: [Severity; 4] = [
    Severity::Critical,
    Severity::High,
    Severity::Medium,
    Severity::Low,
];

/// Render the audit report to an HTML string.
pub fn render_html(report: &AuditReport) -> String {
    let mut html = String::with_capacity(8192);

    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str("  <title>Eigen Audit Report</title>\n");
    html.push_str("  <style>");
    html.push_str(CSS);
    html.push_str("</style>\n");
    html.push_str("</head>\n<body>\n");
    html.push_str("<div class=\"__eigen_audit_container\">\n");
    html.push_str("<h1>Eigen Audit Report</h1>\n");

    if report.summary.total == 0 {
        html.push_str("<div class=\"__eigen_audit_pass\">All checks passed</div>\n");
    } else {
        // Summary bar
        write_summary_bar(&mut html, report);

        // Filter buttons
        write_filter_buttons(&mut html, report);

        // Findings grouped by category
        for cat in &ALL_CATEGORIES {
            write_category_section(&mut html, report, *cat);
        }
    }

    html.push_str("</div>\n");
    html.push_str("<script>");
    html.push_str(JS);
    html.push_str("</script>\n");
    html.push_str("</body>\n</html>\n");

    html
}

fn write_summary_bar(out: &mut String, report: &AuditReport) {
    out.push_str("<div class=\"__eigen_audit_summary\">\n");
    for sev in &ALL_SEVERITIES {
        let count = report.summary.by_severity.get(sev).copied().unwrap_or(0);
        if count > 0 {
            let _ = write!(
                out,
                r#"<div class="__eigen_audit_summary_item" style="background:{color}">{count} {name}</div>"#,
                color = sev.color(),
                name = sev.display_name(),
            );
            out.push('\n');
        }
    }
    out.push_str("</div>\n");
}

fn write_filter_buttons(out: &mut String, report: &AuditReport) {
    out.push_str("<div class=\"__eigen_audit_filters\">\n");

    for sev in &ALL_SEVERITIES {
        if report.summary.by_severity.contains_key(sev) {
            let _ = write!(
                out,
                r#"<button data-filter-kind="severity" data-filter-value="{val}">{name}</button>"#,
                val = sev.display_name().to_ascii_lowercase(),
                name = sev.display_name(),
            );
            out.push('\n');
        }
    }

    for cat in &ALL_CATEGORIES {
        if report.summary.by_category.contains_key(cat) {
            let _ = write!(
                out,
                r#"<button data-filter-kind="category" data-filter-value="{val}">{name}</button>"#,
                val = cat.display_name().to_ascii_lowercase(),
                name = cat.display_name(),
            );
            out.push('\n');
        }
    }

    out.push_str("</div>\n");
}

fn write_category_section(out: &mut String, report: &AuditReport, category: Category) {
    // Collect site-wide findings for this category.
    let site: Vec<&Finding> = report
        .site_findings
        .iter()
        .filter(|f| f.category == category)
        .collect();

    // Collect page findings for this category, grouped by page.
    let mut pages: Vec<(&str, Vec<&Finding>)> = Vec::new();
    for (path, findings) in &report.page_findings {
        let matched: Vec<&Finding> = findings.iter().filter(|f| f.category == category).collect();
        if !matched.is_empty() {
            pages.push((path.as_str(), matched));
        }
    }

    if site.is_empty() && pages.is_empty() {
        return;
    }

    let _ = write!(
        out,
        "<div class=\"__eigen_audit_category\" data-category=\"{val}\">\n<h2>{name}</h2>\n",
        val = category.display_name().to_ascii_lowercase(),
        name = category.display_name(),
    );

    // Site-wide findings
    if !site.is_empty() {
        out.push_str("<div class=\"__eigen_audit_page_group\">\n<h3>Site-wide</h3>\n");
        for finding in &site {
            write_finding(out, finding);
            out.push('\n');
        }
        out.push_str("</div>\n");
    }

    // Page-specific findings
    for (path, findings) in &pages {
        let _ = write!(
            out,
            "<div class=\"__eigen_audit_page_group\">\n<h3>{path}</h3>\n",
        );
        for finding in findings {
            write_finding(out, finding);
            out.push('\n');
        }
        out.push_str("</div>\n");
    }

    out.push_str("</div>\n");
}

/// Write the HTML audit report to `dist/_audit.html`.
pub fn write_html(report: &AuditReport, dist_path: &Path) -> Result<()> {
    let html = render_html(report);
    std::fs::write(dist_path.join("_audit.html"), html)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::audit::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_html_contains_report() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 1,
                by_severity: [(Severity::High, 1)].into_iter().collect(),
                by_category: [(Category::Seo, 1)].into_iter().collect(),
            },
            site_findings: vec![Finding {
                id: "seo/sitemap",
                category: Category::Seo,
                severity: Severity::High,
                scope: Scope::Site,
                message: "No sitemap".into(),
                fix: Fix {
                    file: "site.toml".into(),
                    instruction: "Add sitemap".into(),
                },
            }],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Eigen Audit Report"));
        assert!(html.contains("seo/sitemap"));
        assert!(html.contains("No sitemap"));
    }

    #[test]
    fn test_html_empty_report() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 0,
                by_severity: BTreeMap::new(),
                by_category: BTreeMap::new(),
            },
            site_findings: vec![],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        assert!(html.contains("All checks passed"));
    }

    #[test]
    fn test_write_html_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let report = AuditReport {
            summary: AuditSummary {
                total: 0,
                by_severity: BTreeMap::new(),
                by_category: BTreeMap::new(),
            },
            site_findings: vec![],
            page_findings: BTreeMap::new(),
        };
        write_html(&report, tmp.path()).unwrap();
        assert!(tmp.path().join("_audit.html").exists());
    }

    #[test]
    fn test_html_severity_colors() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 4,
                by_severity: [
                    (Severity::Critical, 1),
                    (Severity::High, 1),
                    (Severity::Medium, 1),
                    (Severity::Low, 1),
                ]
                .into_iter()
                .collect(),
                by_category: [(Category::Seo, 4)].into_iter().collect(),
            },
            site_findings: vec![
                Finding {
                    id: "test/critical",
                    category: Category::Seo,
                    severity: Severity::Critical,
                    scope: Scope::Site,
                    message: "Critical issue".into(),
                    fix: Fix {
                        file: "a.toml".into(),
                        instruction: "fix it".into(),
                    },
                },
                Finding {
                    id: "test/high",
                    category: Category::Seo,
                    severity: Severity::High,
                    scope: Scope::Site,
                    message: "High issue".into(),
                    fix: Fix {
                        file: "a.toml".into(),
                        instruction: "fix it".into(),
                    },
                },
                Finding {
                    id: "test/medium",
                    category: Category::Seo,
                    severity: Severity::Medium,
                    scope: Scope::Site,
                    message: "Medium issue".into(),
                    fix: Fix {
                        file: "a.toml".into(),
                        instruction: "fix it".into(),
                    },
                },
                Finding {
                    id: "test/low",
                    category: Category::Seo,
                    severity: Severity::Low,
                    scope: Scope::Site,
                    message: "Low issue".into(),
                    fix: Fix {
                        file: "a.toml".into(),
                        instruction: "fix it".into(),
                    },
                },
            ],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        assert!(html.contains("#dc2626")); // critical red
        assert!(html.contains("#ea580c")); // high orange
        assert!(html.contains("#ca8a04")); // medium yellow
        assert!(html.contains("#2563eb")); // low blue
    }

    #[test]
    fn test_html_page_findings_grouped() {
        let mut page_findings = BTreeMap::new();
        page_findings.insert(
            "/about.html".to_string(),
            vec![Finding {
                id: "seo/meta-description",
                category: Category::Seo,
                severity: Severity::Medium,
                scope: Scope::Page,
                message: "Missing meta description".into(),
                fix: Fix {
                    file: "templates/about.html".into(),
                    instruction: "Add meta description tag".into(),
                },
            }],
        );

        let report = AuditReport {
            summary: AuditSummary {
                total: 1,
                by_severity: [(Severity::Medium, 1)].into_iter().collect(),
                by_category: [(Category::Seo, 1)].into_iter().collect(),
            },
            site_findings: vec![],
            page_findings,
        };
        let html = render_html(&report);
        assert!(html.contains("/about.html"));
        assert!(html.contains("seo/meta-description"));
        assert!(html.contains("Missing meta description"));
        assert!(html.contains("templates/about.html"));
    }

    #[test]
    fn test_html_escapes_special_chars() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 1,
                by_severity: [(Severity::Low, 1)].into_iter().collect(),
                by_category: [(Category::BestPractices, 1)].into_iter().collect(),
            },
            site_findings: vec![Finding {
                id: "bp/test",
                category: Category::BestPractices,
                severity: Severity::Low,
                scope: Scope::Site,
                message: "Use <meta> & \"quotes\"".into(),
                fix: Fix {
                    file: "test.html".into(),
                    instruction: "Add <tag>".into(),
                },
            }],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        assert!(html.contains("&lt;meta&gt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&quot;quotes&quot;"));
    }

    #[test]
    fn test_html_has_filter_buttons() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 1,
                by_severity: [(Severity::High, 1)].into_iter().collect(),
                by_category: [(Category::Performance, 1)].into_iter().collect(),
            },
            site_findings: vec![Finding {
                id: "perf/test",
                category: Category::Performance,
                severity: Severity::High,
                scope: Scope::Site,
                message: "Slow".into(),
                fix: Fix {
                    file: "a.html".into(),
                    instruction: "Speed up".into(),
                },
            }],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        assert!(html.contains("data-filter-kind=\"severity\""));
        assert!(html.contains("data-filter-kind=\"category\""));
        assert!(html.contains("data-filter-value=\"high\""));
        assert!(html.contains("data-filter-value=\"performance\""));
    }

    #[test]
    fn test_html_has_dark_mode_support() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 0,
                by_severity: BTreeMap::new(),
                by_category: BTreeMap::new(),
            },
            site_findings: vec![],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        assert!(html.contains("prefers-color-scheme:dark"));
        assert!(html.contains("#1a1a2e")); // dark mode bg
    }

    #[test]
    fn test_html_structure() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 0,
                by_severity: BTreeMap::new(),
                by_category: BTreeMap::new(),
            },
            site_findings: vec![],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("<head>"));
        assert!(html.contains("</head>"));
        assert!(html.contains("<body>"));
        assert!(html.contains("</body>"));
        assert!(html.contains("</html>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("<script>"));
    }

    #[test]
    fn test_html_css_class_prefix() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 1,
                by_severity: [(Severity::Low, 1)].into_iter().collect(),
                by_category: [(Category::Seo, 1)].into_iter().collect(),
            },
            site_findings: vec![Finding {
                id: "seo/test",
                category: Category::Seo,
                severity: Severity::Low,
                scope: Scope::Site,
                message: "Test".into(),
                fix: Fix {
                    file: "a.toml".into(),
                    instruction: "fix".into(),
                },
            }],
            page_findings: BTreeMap::new(),
        };
        let html = render_html(&report);
        // All custom classes should use the __eigen_audit_ prefix.
        // Check that key class names appear with the prefix.
        assert!(html.contains("__eigen_audit_container"));
        assert!(html.contains("__eigen_audit_summary"));
        assert!(html.contains("__eigen_audit_finding"));
        assert!(html.contains("__eigen_audit_badge"));
        assert!(html.contains("__eigen_audit_fix"));
    }
}
