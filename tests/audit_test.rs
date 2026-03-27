//! Integration tests for the post-build audit feature.
//!
//! These tests build a minimal site from scratch, then run the audit
//! against the rendered output to verify findings, ignore lists, and
//! output file generation.

use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Helper to write a file, creating parent dirs as needed.
fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// Create a minimal site project in `root` with the given `site_toml` content
/// and a bare `templates/index.html` (no SEO tags).
fn setup_minimal_site(root: &Path, site_toml: &str) {
    write(root, "site.toml", site_toml);
    write(
        root,
        "templates/index.html",
        "<html><head></head><body><p>hello</p></body></html>",
    );
}

// ============================================================================
// 1. Audit finds issues on a minimal site
// ============================================================================

#[test]
fn test_audit_finds_issues_on_minimal_site() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let site_toml = r#"
[site]
name = "Audit Test"
base_url = "https://example.com"

[build]
minify = false
"#;

    setup_minimal_site(root, site_toml);

    // Full build: creates dist/ with rendered pages and sitemap.
    eigen::build::build(root, true).unwrap();

    let config = eigen::config::load_config(root).unwrap();
    let dist = root.join("dist");

    // The build renders index.html into dist/index.html.
    let rendered_pages = vec![eigen::build::render::RenderedPage {
        url_path: "/index.html".into(),
        is_index: true,
        is_dynamic: false,
        template_path: Some("index.html".into()),
    }];

    let report = eigen::build::audit::run_audit(&config, &dist, &rendered_pages).unwrap();

    // A bare HTML page with no SEO tags and no robots config should produce findings.
    assert!(
        report.summary.total > 0,
        "expected audit findings on a bare site, got total=0"
    );

    // Site-level: no robots.txt configured.
    let site_ids: Vec<&str> = report.site_findings.iter().map(|f| f.id).collect();
    assert!(
        site_ids.contains(&"seo/robots-txt"),
        "expected seo/robots-txt in site findings, got: {site_ids:?}"
    );
}

// ============================================================================
// 2. Audit respects ignore list
// ============================================================================

#[test]
fn test_audit_respects_ignore_list() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let site_toml = r#"
[site]
name = "Ignore Test"
base_url = "https://example.com"

[build]
minify = false

[audit]
ignore = ["seo/robots-txt", "seo/feed"]
"#;

    setup_minimal_site(root, site_toml);
    eigen::build::build(root, true).unwrap();

    let config = eigen::config::load_config(root).unwrap();
    let dist = root.join("dist");

    let rendered_pages = vec![eigen::build::render::RenderedPage {
        url_path: "/index.html".into(),
        is_index: true,
        is_dynamic: false,
        template_path: Some("index.html".into()),
    }];

    let report = eigen::build::audit::run_audit(&config, &dist, &rendered_pages).unwrap();

    let site_ids: Vec<&str> = report.site_findings.iter().map(|f| f.id).collect();
    assert!(
        !site_ids.contains(&"seo/robots-txt"),
        "seo/robots-txt should be filtered by ignore list, got: {site_ids:?}"
    );
    assert!(
        !site_ids.contains(&"seo/feed"),
        "seo/feed should be filtered by ignore list, got: {site_ids:?}"
    );
}

// ============================================================================
// 3. Audit output files are written
// ============================================================================

#[test]
fn test_audit_output_files_written() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let site_toml = r#"
[site]
name = "Output Test"
base_url = "https://example.com"

[build]
minify = false
"#;

    setup_minimal_site(root, site_toml);
    eigen::build::build(root, true).unwrap();

    let config = eigen::config::load_config(root).unwrap();
    let dist = root.join("dist");

    let rendered_pages = vec![eigen::build::render::RenderedPage {
        url_path: "/index.html".into(),
        is_index: true,
        is_dynamic: false,
        template_path: Some("index.html".into()),
    }];

    let report = eigen::build::audit::run_audit(&config, &dist, &rendered_pages).unwrap();
    eigen::build::audit::output::write_all(&report, &dist).unwrap();

    // All three output files should exist.
    assert!(
        dist.join("_audit.html").exists(),
        "_audit.html should be written"
    );
    assert!(
        dist.join("_audit.json").exists(),
        "_audit.json should be written"
    );
    assert!(
        dist.join("_audit.md").exists(),
        "_audit.md should be written"
    );

    // The JSON file should parse as valid JSON.
    let json_content = fs::read_to_string(dist.join("_audit.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_content)
        .expect("_audit.json should be valid JSON");

    // Sanity check: the JSON should have a "summary" key.
    assert!(
        parsed.get("summary").is_some(),
        "JSON report should contain a 'summary' field"
    );
}
