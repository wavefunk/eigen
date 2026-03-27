//! Check registry: collects and runs all audit checks.

pub mod accessibility;
pub mod best_practices;
pub mod performance;
pub mod seo;

use super::Finding;
use crate::config::SiteConfig;
use std::path::Path;

/// Run all site-level checks.
pub fn run_site_checks(config: &SiteConfig, dist_path: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(seo::site_checks(config, dist_path));
    findings.extend(performance::site_checks(config));
    findings.extend(accessibility::site_checks());
    findings.extend(best_practices::site_checks(config, dist_path));
    findings
}

/// Run all page-level checks on a single page.
pub fn run_page_checks(
    html: &str,
    page_path: &str,
    template_path: &str,
    config: &SiteConfig,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(seo::page_checks(html, page_path, template_path));
    findings.extend(performance::page_checks(html, page_path, template_path, config));
    findings.extend(accessibility::page_checks(html, page_path, template_path));
    findings.extend(best_practices::page_checks(html, page_path, template_path));
    findings
}
