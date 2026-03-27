//! 404 page generation.
//!
//! When `build.not_found = true` in `site.toml`:
//! - If the project has a `templates/404.html` template it is rendered like any
//!   other static page (handled by the normal rendering pipeline).
//! - If no such template exists, a built-in default `404.html` is written to
//!   `dist/404.html` so the site always has a usable not-found page.

use eyre::{Result, WrapErr};
use std::path::Path;

/// The built-in 404 page emitted when no custom template exists.
const DEFAULT_404_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>404 – Page Not Found</title>
  <style>
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: system-ui, -apple-system, sans-serif;
      background: #f8f8f8;
      color: #222;
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      padding: 2rem;
    }
    .card {
      background: #fff;
      border-radius: 12px;
      box-shadow: 0 4px 24px rgba(0,0,0,.08);
      padding: 3rem 2.5rem;
      max-width: 480px;
      width: 100%;
      text-align: center;
    }
    .code {
      font-size: 6rem;
      font-weight: 800;
      line-height: 1;
      color: #e0e0e0;
      letter-spacing: -4px;
    }
    h1 {
      font-size: 1.5rem;
      margin: 1rem 0 .5rem;
    }
    p {
      color: #666;
      line-height: 1.6;
    }
    a {
      display: inline-block;
      margin-top: 1.5rem;
      padding: .6rem 1.4rem;
      background: #222;
      color: #fff;
      border-radius: 6px;
      text-decoration: none;
      font-size: .9rem;
    }
    a:hover { background: #444; }
  </style>
</head>
<body>
  <div class="card">
    <div class="code">404</div>
    <h1>Page Not Found</h1>
    <p>The page you were looking for doesn&rsquo;t exist or has been moved.</p>
    <a href="/">Go Home</a>
  </div>
</body>
</html>
"#;

/// Write the default 404 page to `dist/404.html` if no custom template exists.
///
/// If a `templates/404.html` exists the normal rendering pipeline already
/// handled it, so this is a no-op.
pub fn write_default_if_missing(project_root: &Path, dist_dir: &Path) -> Result<()> {
    let custom_template = project_root.join("templates").join("404.html");
    if custom_template.exists() {
        return Ok(());
    }

    let out = dist_dir.join("404.html");
    std::fs::write(&out, DEFAULT_404_HTML)
        .wrap_err_with(|| format!("Failed to write default 404 page to {}", out.display()))?;

    tracing::info!("Generating default 404.html... ✓");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup(tmp: &TempDir) -> std::path::PathBuf {
        let dist = tmp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();
        dist
    }

    #[test]
    fn test_writes_default_when_no_template() {
        let tmp = TempDir::new().unwrap();
        let dist = setup(&tmp);

        write_default_if_missing(tmp.path(), &dist).unwrap();

        let out = dist.join("404.html");
        assert!(out.exists(), "dist/404.html should be created");

        let html = fs::read_to_string(&out).unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("404"));
        assert!(html.contains("Page Not Found"));
        assert!(html.contains(r#"href="/""#), "Should contain a link back to home");
    }

    #[test]
    fn test_skips_default_when_custom_template_exists() {
        let tmp = TempDir::new().unwrap();
        let dist = setup(&tmp);

        // Create a custom template (content doesn't matter here).
        let tmpl_dir = tmp.path().join("templates");
        fs::create_dir_all(&tmpl_dir).unwrap();
        fs::write(tmpl_dir.join("404.html"), "<h1>Custom 404</h1>").unwrap();

        write_default_if_missing(tmp.path(), &dist).unwrap();

        assert!(
            !dist.join("404.html").exists(),
            "Should NOT write default when custom template is present"
        );
    }

    #[test]
    fn test_default_html_is_valid_structure() {
        assert!(DEFAULT_404_HTML.contains("<!DOCTYPE html>"));
        assert!(DEFAULT_404_HTML.contains("</html>"));
        assert!(DEFAULT_404_HTML.contains("<style>"));
        assert!(DEFAULT_404_HTML.contains("404"));
    }
}
