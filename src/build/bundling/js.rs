//! JS concatenation with IIFE wrapping.
//!
//! Each file is wrapped in an Immediately Invoked Function Expression (IIFE)
//! to prevent variable scope pollution between files.

use std::path::Path;

use eyre::Result;

/// Read and concatenate multiple JS files into a single string.
///
/// Files are concatenated in order, each wrapped in an IIFE:
/// ```js
/// // Source: /js/utils.js
/// ;(function(){
/// /* original file content */
/// })();
/// ```
///
/// The leading semicolon is defensive -- it prevents issues if a previous
/// file's content ends unexpectedly. The IIFE prevents `var` and function
/// declarations from leaking into the global scope.
///
/// Missing files are skipped with a warning logged.
pub fn merge_js_files(
    srcs: &[String],
    dist_dir: &Path,
) -> Result<String> {
    let mut merged = String::new();
    let mut files_merged = 0u32;

    for src in srcs {
        let relative = src.trim_start_matches('/');
        let file_path = dist_dir.join(relative);

        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "JS bundling: file '{}' not found, skipping: {}",
                    file_path.display(), e
                );
                continue;
            }
        };

        if !merged.is_empty() {
            merged.push('\n');
        }

        // Source comment for traceability.
        merged.push_str("// Source: ");
        merged.push_str(src);
        merged.push('\n');

        // IIFE wrapping.
        merged.push_str(";(function(){\n");
        merged.push_str(&content);
        merged.push_str("\n})();\n");

        files_merged += 1;
    }

    if files_merged == 0 && !srcs.is_empty() {
        tracing::warn!("JS bundling: all {} JS file(s) failed to load", srcs.len());
    }

    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_merge_single_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/app.js", "var x = 1;");

        let result = merge_js_files(&["/js/app.js".to_string()], dist).unwrap();
        assert!(result.contains("// Source: /js/app.js"));
        assert!(result.contains(";(function(){"));
        assert!(result.contains("var x = 1;"));
        assert!(result.contains("})();"));
    }

    #[test]
    fn test_merge_multiple_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/utils.js", "function util() {}");
        write_file(dist, "js/app.js", "util();");

        let result = merge_js_files(
            &["/js/utils.js".to_string(), "/js/app.js".to_string()],
            dist,
        ).unwrap();

        // Both files should be wrapped in IIFEs.
        assert_eq!(result.matches(";(function(){").count(), 2);
        assert_eq!(result.matches("})();").count(), 2);
        assert!(result.contains("// Source: /js/utils.js"));
        assert!(result.contains("// Source: /js/app.js"));

        // utils.js should appear before app.js.
        let utils_pos = result.find("// Source: /js/utils.js").unwrap();
        let app_pos = result.find("// Source: /js/app.js").unwrap();
        assert!(utils_pos < app_pos);
    }

    #[test]
    fn test_merge_missing_js() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/exists.js", "var y = 2;");

        let result = merge_js_files(
            &["/js/missing.js".to_string(), "/js/exists.js".to_string()],
            dist,
        ).unwrap();

        // Missing file is skipped, existing file is included.
        assert!(!result.contains("missing.js"));
        assert!(result.contains("var y = 2;"));
    }

    #[test]
    fn test_iife_wrapping() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "js/test.js", "console.log('hello');");

        let result = merge_js_files(&["/js/test.js".to_string()], dist).unwrap();

        // Verify IIFE structure.
        assert!(result.contains(";(function(){\nconsole.log('hello');\n})();"));
    }
}
