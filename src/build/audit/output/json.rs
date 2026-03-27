use super::super::AuditReport;
use eyre::Result;
use std::path::Path;

pub fn render_json(report: &AuditReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

pub fn write_json(report: &AuditReport, dist_path: &Path) -> Result<()> {
    let json = render_json(report)?;
    std::fs::write(dist_path.join("_audit.json"), json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::audit::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_json_output_valid() {
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
        let json = render_json(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["summary"]["total"], 1);
        assert_eq!(parsed["site_findings"][0]["id"], "seo/sitemap");
        assert_eq!(parsed["site_findings"][0]["severity"], "high");
    }

    #[test]
    fn test_write_json_file() {
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
        write_json(&report, tmp.path()).unwrap();
        assert!(tmp.path().join("_audit.json").exists());
        let content = std::fs::read_to_string(tmp.path().join("_audit.json")).unwrap();
        let _: serde_json::Value = serde_json::from_str(&content).unwrap();
    }
}
