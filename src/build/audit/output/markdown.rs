use super::super::{AuditReport, Severity};
use eyre::Result;
use std::fmt::Write;
use std::path::Path;

/// Render the audit report as a Markdown string.
pub fn render_markdown(report: &AuditReport) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# Eigen Audit Report\n");
    let _ = writeln!(md, "## Summary\n");

    for sev in &[
        Severity::Critical,
        Severity::High,
        Severity::Medium,
        Severity::Low,
    ] {
        let count = report.summary.by_severity.get(sev).copied().unwrap_or(0);
        if count > 0 {
            let _ = writeln!(md, "- **{}:** {}", sev.display_name(), count);
        }
    }

    if report.summary.total == 0 {
        let _ = writeln!(md, "\nAll checks passed!");
        return md;
    }

    if !report.site_findings.is_empty() {
        let _ = writeln!(md, "\n## Site-Wide Issues\n");
        for f in &report.site_findings {
            let _ = writeln!(md, "### [{}] {}", f.severity.display_name(), f.id);
            let _ = writeln!(md, "{}", f.message);
            let _ = writeln!(md, "**Fix:** In `{}`, {}\n", f.fix.file, f.fix.instruction);
        }
    }

    if !report.page_findings.is_empty() {
        let _ = writeln!(md, "\n## Page Issues\n");
        for (page, findings) in &report.page_findings {
            let _ = writeln!(md, "### {}\n", page);
            for f in findings {
                let _ = writeln!(md, "#### [{}] {}", f.severity.display_name(), f.id);
                let _ = writeln!(md, "{}", f.message);
                let _ = writeln!(md, "**Fix:** In `{}`, {}\n", f.fix.file, f.fix.instruction);
            }
        }
    }

    md
}

/// Write the audit report as `_audit.md` in the dist directory.
pub fn write_markdown(report: &AuditReport, dist_path: &Path) -> Result<()> {
    let md = render_markdown(report);
    std::fs::write(dist_path.join("_audit.md"), md)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::audit::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_markdown_structure() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 2,
                by_severity: [(Severity::High, 1), (Severity::Low, 1)]
                    .into_iter()
                    .collect(),
                by_category: [(Category::Seo, 2)].into_iter().collect(),
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
            page_findings: [(
                "index.html".into(),
                vec![Finding {
                    id: "seo/feed",
                    category: Category::Seo,
                    severity: Severity::Low,
                    scope: Scope::Page,
                    message: "No feed".into(),
                    fix: Fix {
                        file: "site.toml".into(),
                        instruction: "Add feed".into(),
                    },
                }],
            )]
            .into_iter()
            .collect(),
        };
        let md = render_markdown(&report);
        assert!(md.contains("# Eigen Audit Report"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("- **HIGH:** 1"));
        assert!(md.contains("- **LOW:** 1"));
        assert!(md.contains("## Site-Wide Issues"));
        assert!(md.contains("[HIGH] seo/sitemap"));
        assert!(md.contains("No sitemap"));
        assert!(md.contains("### index.html"));
        assert!(md.contains("[LOW] seo/feed"));
        assert!(md.contains("**Fix:**"));
        assert!(md.contains("In `site.toml`, Add sitemap"));
    }

    #[test]
    fn test_markdown_empty_report() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 0,
                by_severity: BTreeMap::new(),
                by_category: BTreeMap::new(),
            },
            site_findings: vec![],
            page_findings: BTreeMap::new(),
        };
        let md = render_markdown(&report);
        assert!(md.contains("# Eigen Audit Report"));
        assert!(md.contains("All checks passed!"));
        // Should not contain issue sections
        assert!(!md.contains("## Site-Wide Issues"));
        assert!(!md.contains("## Page Issues"));
    }

    #[test]
    fn test_markdown_severity_ordering() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 3,
                by_severity: [
                    (Severity::Low, 1),
                    (Severity::Critical, 1),
                    (Severity::Medium, 1),
                ]
                .into_iter()
                .collect(),
                by_category: [(Category::Seo, 3)].into_iter().collect(),
            },
            site_findings: vec![],
            page_findings: BTreeMap::new(),
        };
        let md = render_markdown(&report);
        let crit_pos = md.find("**CRITICAL:**").expect("CRITICAL not found");
        let med_pos = md.find("**MEDIUM:**").expect("MEDIUM not found");
        let low_pos = md.find("**LOW:**").expect("LOW not found");
        assert!(crit_pos < med_pos, "CRITICAL should appear before MEDIUM");
        assert!(med_pos < low_pos, "MEDIUM should appear before LOW");
    }

    #[test]
    fn test_write_markdown_file() {
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
        write_markdown(&report, tmp.path()).unwrap();
        let path = tmp.path().join("_audit.md");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Eigen Audit Report"));
    }

    #[test]
    fn test_markdown_no_site_section_when_empty() {
        let report = AuditReport {
            summary: AuditSummary {
                total: 1,
                by_severity: [(Severity::Low, 1)].into_iter().collect(),
                by_category: [(Category::Seo, 1)].into_iter().collect(),
            },
            site_findings: vec![],
            page_findings: [(
                "/about.html".into(),
                vec![Finding {
                    id: "seo/title",
                    category: Category::Seo,
                    severity: Severity::Low,
                    scope: Scope::Page,
                    message: "Missing title".into(),
                    fix: Fix {
                        file: "templates/about.html".into(),
                        instruction: "Add a <title> tag".into(),
                    },
                }],
            )]
            .into_iter()
            .collect(),
        };
        let md = render_markdown(&report);
        assert!(!md.contains("## Site-Wide Issues"));
        assert!(md.contains("## Page Issues"));
        assert!(md.contains("### /about.html"));
        assert!(md.contains("In `templates/about.html`, Add a <title> tag"));
    }
}
