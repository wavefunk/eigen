//! Step 5.1: Output directory setup.
//!
//! - Delete `dist/` entirely
//! - Recreate `dist/`
//! - Create `dist/_fragments/` if fragments are enabled
//! - Copy `static/` → `dist/` recursively

use eyre::{Result, WrapErr};
use std::path::Path;
use walkdir::WalkDir;

/// Prepare the output directory: clean, recreate, and optionally create
/// the fragments subdirectory.
pub fn setup_output_dir(project_root: &Path, fragments_enabled: bool, fragment_dir: &str) -> Result<()> {
    let dist = project_root.join("dist");

    // Clean: remove the entire dist/ directory if it exists.
    if dist.exists() {
        std::fs::remove_dir_all(&dist)
            .wrap_err_with(|| format!("Failed to remove {}", dist.display()))?;
    }

    // Recreate dist/.
    std::fs::create_dir_all(&dist)
        .wrap_err("Failed to create dist/ directory")?;

    // Create fragments directory if enabled.
    if fragments_enabled {
        let frag_dir = dist.join(fragment_dir);
        std::fs::create_dir_all(&frag_dir)
            .wrap_err_with(|| format!("Failed to create {}", frag_dir.display()))?;
    }

    Ok(())
}

/// Copy the `static/` directory contents into `dist/` recursively.
///
/// Preserves directory structure: `static/css/style.css` → `dist/css/style.css`.
/// If `static/` does not exist, this is a no-op.
///
/// When content hashing is enabled, files are renamed with content-based
/// hashes and the mapping is returned in the `AssetManifest`.
pub fn copy_static_assets(
    project_root: &Path,
    content_hash_config: &crate::config::ContentHashConfig,
) -> Result<crate::build::content_hash::AssetManifest> {
    let static_dir = project_root.join("static");
    let dist_dir = project_root.join("dist");

    if !static_dir.is_dir() {
        return Ok(crate::build::content_hash::AssetManifest::new());
    }

    for entry in WalkDir::new(&static_dir).into_iter() {
        let entry = entry.wrap_err("Failed to read entry while copying static assets")?;
        let src_path = entry.path();
        let rel_path = src_path
            .strip_prefix(&static_dir)
            .wrap_err("Static asset path is not inside static/ directory")?;
        let dst_path = dist_dir.join(rel_path);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dst_path)
                .wrap_err_with(|| format!("Failed to create directory {}", dst_path.display()))?;
        } else {
            // Skip robots.txt — handled by the robots module.
            if rel_path == std::path::Path::new("robots.txt") {
                continue;
            }
            // Ensure parent directory exists.
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!("Failed to create parent dir for {}", dst_path.display()))?;
            }
            std::fs::copy(src_path, &dst_path)
                .wrap_err_with(|| {
                    format!("Failed to copy {} → {}", src_path.display(), dst_path.display())
                })?;
        }
    }

    // Phase 1: Build content hash manifest if enabled.
    if content_hash_config.enabled {
        crate::build::content_hash::build_manifest(&dist_dir, &static_dir, content_hash_config)
    } else {
        Ok(crate::build::content_hash::AssetManifest::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_setup_output_dir_creates_dist() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        setup_output_dir(root, false, "_fragments").unwrap();
        assert!(root.join("dist").is_dir());
        assert!(!root.join("dist/_fragments").exists());
    }

    #[test]
    fn test_setup_output_dir_with_fragments() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        setup_output_dir(root, true, "_fragments").unwrap();
        assert!(root.join("dist").is_dir());
        assert!(root.join("dist/_fragments").is_dir());
    }

    #[test]
    fn test_setup_output_dir_cleans_existing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create some pre-existing content.
        write(root, "dist/old_file.html", "old");
        write(root, "dist/subdir/old.html", "old");

        setup_output_dir(root, false, "_fragments").unwrap();
        assert!(root.join("dist").is_dir());
        assert!(!root.join("dist/old_file.html").exists());
        assert!(!root.join("dist/subdir").exists());
    }

    #[test]
    fn test_setup_output_dir_custom_fragment_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        setup_output_dir(root, true, "_frags").unwrap();
        assert!(root.join("dist/_frags").is_dir());
    }

    #[test]
    fn test_copy_static_assets() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(root, "static/css/style.css", "body { color: red; }");
        write(root, "static/js/app.js", "console.log('hi');");
        write(root, "static/favicon.ico", "icon-data");

        // Create dist/.
        fs::create_dir_all(root.join("dist")).unwrap();

        copy_static_assets(root, &crate::config::ContentHashConfig::default()).unwrap();

        assert!(root.join("dist/css/style.css").exists());
        assert!(root.join("dist/js/app.js").exists());
        assert!(root.join("dist/favicon.ico").exists());

        let css = fs::read_to_string(root.join("dist/css/style.css")).unwrap();
        assert_eq!(css, "body { color: red; }");
    }

    #[test]
    fn test_copy_static_assets_no_static_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("dist")).unwrap();

        // Should be a no-op, not an error.
        copy_static_assets(root, &crate::config::ContentHashConfig::default()).unwrap();
    }

    #[test]
    fn test_copy_static_preserves_structure() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(root, "static/a/b/c/deep.txt", "deep");
        fs::create_dir_all(root.join("dist")).unwrap();

        copy_static_assets(root, &crate::config::ContentHashConfig::default()).unwrap();

        assert!(root.join("dist/a/b/c/deep.txt").exists());
        let content = fs::read_to_string(root.join("dist/a/b/c/deep.txt")).unwrap();
        assert_eq!(content, "deep");
    }
}
