//! Robots.txt generation.
//!
//! When `[build.robots] enabled = true`, either copies `static/robots.txt`
//! to `dist/robots.txt` (if the file exists) or generates a sensible default.

use eyre::{Result, WrapErr};
use std::path::Path;

const DEFAULT_ROBOTS_TXT: &str = "\
User-agent: *\n\
Allow: /\n\
";

/// Write `dist/robots.txt`.
///
/// If `{project_root}/static/robots.txt` exists it is copied as-is.
/// Otherwise the built-in default is written.
pub fn write(project_root: &Path, dist_dir: &Path) -> Result<()> {
    let custom = project_root.join("static").join("robots.txt");
    let dest = dist_dir.join("robots.txt");

    if custom.exists() {
        std::fs::copy(&custom, &dest)
            .wrap_err_with(|| format!("Failed to copy robots.txt to {}", dest.display()))?;
        tracing::info!("Copying robots.txt... ✓");
    } else {
        std::fs::write(&dest, DEFAULT_ROBOTS_TXT)
            .wrap_err_with(|| format!("Failed to write robots.txt to {}", dest.display()))?;
        tracing::info!("Generating default robots.txt... ✓");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup(custom_robots: Option<&str>) -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("dist")).unwrap();
        if let Some(content) = custom_robots {
            fs::create_dir_all(tmp.path().join("static")).unwrap();
            fs::write(tmp.path().join("static/robots.txt"), content).unwrap();
        }
        tmp
    }

    #[test]
    fn test_writes_default_when_no_custom() {
        let tmp = setup(None);
        write(tmp.path(), &tmp.path().join("dist")).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert!(content.contains("User-agent: *"));
        assert!(content.contains("Allow: /"));
    }

    #[test]
    fn test_copies_custom_when_exists() {
        let custom = "User-agent: *\nDisallow: /secret/\n";
        let tmp = setup(Some(custom));
        write(tmp.path(), &tmp.path().join("dist")).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert_eq!(content, custom);
    }

    #[test]
    fn test_custom_overrides_default() {
        let custom = "User-agent: Googlebot\nDisallow: /\n";
        let tmp = setup(Some(custom));
        write(tmp.path(), &tmp.path().join("dist")).unwrap();
        let content = fs::read_to_string(tmp.path().join("dist/robots.txt")).unwrap();
        assert!(!content.contains("Allow: /"));
        assert!(content.contains("Googlebot"));
    }
}
