//! Build-time audit: SEO, performance, accessibility, and best-practice checks.
//!
//! Runs as the final build phase. Inspects `SiteConfig` and rendered HTML
//! in `dist/` to produce an `AuditReport` with actionable findings.

pub mod checks;
pub mod output;
pub mod overlay;

use crate::build::render::RenderedPage;
use crate::config::SiteConfig;
use eyre::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Severity of an audit finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

/// Category of an audit check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Seo,
    Performance,
    Accessibility,
    BestPractices,
}

impl Category {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Seo => "SEO",
            Self::Performance => "Performance",
            Self::Accessibility => "Accessibility",
            Self::BestPractices => "Best Practices",
        }
    }
}

impl Severity {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
        }
    }
}

impl Severity {
    /// CSS color for this severity level.
    pub fn color(&self) -> &'static str {
        match self {
            Self::Critical => "#dc2626",
            Self::High => "#ea580c",
            Self::Medium => "#ca8a04",
            Self::Low => "#2563eb",
        }
    }
}

/// Scope of a check: site-wide or per-page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Site,
    Page,
}

/// Actionable fix information for a finding.
#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    /// File to modify (e.g., "site.toml" or "templates/about.html").
    pub file: String,
    /// Human-readable instruction for the fix.
    pub instruction: String,
}

/// A single audit finding.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Stable check ID (e.g., "seo/meta-description").
    pub id: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub scope: Scope,
    /// Human-readable issue description.
    pub message: String,
    /// Actionable fix.
    pub fix: Fix,
}

/// Summary counts for the audit report.
#[derive(Debug, Clone, Serialize)]
pub struct AuditSummary {
    pub total: usize,
    pub by_severity: BTreeMap<Severity, usize>,
    pub by_category: BTreeMap<Category, usize>,
}

/// The complete audit report.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub summary: AuditSummary,
    pub site_findings: Vec<Finding>,
    pub page_findings: BTreeMap<String, Vec<Finding>>,
}

impl AuditReport {
    /// Build summary counts from the findings.
    fn compute_summary(
        site_findings: &[Finding],
        page_findings: &BTreeMap<String, Vec<Finding>>,
    ) -> AuditSummary {
        let mut by_severity: BTreeMap<Severity, usize> = BTreeMap::new();
        let mut by_category: BTreeMap<Category, usize> = BTreeMap::new();
        let mut total = 0;

        let all_findings = site_findings
            .iter()
            .chain(page_findings.values().flatten());

        for f in all_findings {
            total += 1;
            *by_severity.entry(f.severity).or_default() += 1;
            *by_category.entry(f.category).or_default() += 1;
        }

        AuditSummary {
            total,
            by_severity,
            by_category,
        }
    }
}

/// Run the full audit against the built site.
///
/// Inspects config for site-level checks, then reads each rendered page's
/// HTML from `dist/` for page-level checks. Filters out any check IDs
/// listed in `config.audit.ignore`.
pub fn run_audit(
    config: &SiteConfig,
    dist_path: &Path,
    rendered_pages: &[RenderedPage],
) -> Result<AuditReport> {
    let ignore_ids: Vec<&str> = config
        .audit
        .as_ref()
        .map(|a| a.ignore.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    // Run site-level checks.
    let mut site_findings = checks::run_site_checks(config, dist_path);
    site_findings.retain(|f| !ignore_ids.contains(&f.id));

    // Run page-level checks.
    let mut page_findings: BTreeMap<String, Vec<Finding>> = BTreeMap::new();
    for page in rendered_pages {
        let html_path = dist_path.join(page.url_path.trim_start_matches('/'));
        let html = match std::fs::read_to_string(&html_path) {
            Ok(h) => h,
            Err(_) => continue,
        };

        let template_path = page
            .template_path
            .as_deref()
            .unwrap_or("(unknown)");

        let mut findings =
            checks::run_page_checks(&html, &page.url_path, template_path, config);
        findings.retain(|f| !ignore_ids.contains(&f.id));

        if !findings.is_empty() {
            page_findings.insert(page.url_path.clone(), findings);
        }
    }

    let summary = AuditReport::compute_summary(&site_findings, &page_findings);

    Ok(AuditReport {
        summary,
        site_findings,
        page_findings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteConfig;

    fn test_config() -> SiteConfig {
        let toml_str = r#"
            [site]
            name = "Test"
            base_url = "https://example.com"
        "#;
        toml::from_str(toml_str).unwrap()
    }

    #[test]
    fn test_run_audit_empty_pages() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config();
        let report = run_audit(&config, tmp.path(), &[]).unwrap();
        assert_eq!(report.page_findings.len(), 0);
    }

    #[test]
    fn test_run_audit_filters_ignored_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let dist = tmp.path();

        std::fs::write(dist.join("index.html"), "<html><body>hi</body></html>").unwrap();

        let pages = vec![RenderedPage {
            url_path: "/index.html".into(),
            is_index: true,
            is_dynamic: false,
            template_path: None,
        }];

        // Without ignore — run audit to see what findings exist.
        let config = test_config();
        let report = run_audit(&config, dist, &pages).unwrap();

        // With ignore list covering all site findings.
        let all_site_ids: Vec<String> = report.site_findings
            .iter()
            .map(|f| f.id.to_string())
            .collect();

        if !all_site_ids.is_empty() {
            let toml_str = format!(
                r#"
                [site]
                name = "Test"
                base_url = "https://example.com"
                [audit]
                ignore = {:?}
                "#,
                all_site_ids,
            );
            let config2: SiteConfig = toml::from_str(&toml_str).unwrap();
            let filtered = run_audit(&config2, dist, &pages).unwrap();
            assert!(filtered.site_findings.is_empty());
        }
    }
}
