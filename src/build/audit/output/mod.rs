pub mod html;
pub mod json;
pub mod markdown;

use super::AuditReport;
use eyre::Result;
use std::path::Path;

/// Write all audit output files to dist/.
pub fn write_all(report: &AuditReport, dist_path: &Path) -> Result<()> {
    html::write_html(report, dist_path)?;
    json::write_json(report, dist_path)?;
    markdown::write_markdown(report, dist_path)?;
    Ok(())
}
