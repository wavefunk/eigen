//! CSS/JS bundling and tree-shaking.
//!
//! Merges multiple CSS/JS files into single bundles, tree-shakes unused
//! CSS selectors, and rewrites HTML references. Runs as Phase 2.5 in the
//! build pipeline (after rendering, before content hashing).

pub mod collect;
pub mod css;
pub mod js;
pub mod rewrite;

use std::path::Path;

use eyre::{Result, WrapErr};

use crate::config::BundlingConfig;

/// Bundle CSS and JS files in dist/ and rewrite HTML references.
///
/// This is the main entry point called from the build pipeline after all
/// pages have been rendered and written to disk.
///
/// Steps:
/// 1. Scan all HTML files in dist/ for <link> and <script> tags.
/// 2. Collect and deduplicate referenced local CSS/JS files.
/// 3. For CSS: merge, tree-shake, write bundled file.
/// 4. For JS: concatenate with IIFE wrapping, write bundled file.
/// 5. Rewrite all HTML files to reference bundled files.
///
/// `minify_css` controls whether the bundled CSS output is minified
/// via lightningcss's printer. Typically set to `config.build.minify`.
///
/// Returns the list of generated file paths (relative to dist/)
/// for content hashing integration.
pub fn bundle_assets(
    dist_dir: &Path,
    config: &BundlingConfig,
    minify_css: bool,
) -> Result<Vec<String>> {
    if !config.enabled {
        return Ok(Vec::new());
    }

    // Step 1-2: Collect references from all HTML files.
    let refs = collect::collect_references(dist_dir, &config.exclude)
        .wrap_err("Failed to collect CSS/JS references from HTML files")?;

    if refs.css_hrefs.is_empty() && refs.js_srcs.is_empty() {
        tracing::debug!("Bundling: no local CSS/JS references found in HTML files");
        return Ok(Vec::new());
    }

    tracing::info!(
        "Bundling: found {} CSS file(s), {} JS file(s) across {} HTML page(s)",
        refs.css_hrefs.len(),
        refs.js_srcs.len(),
        refs.html_files.len(),
    );

    let mut generated_files: Vec<String> = Vec::new();
    let mut css_bundle_href: Option<String> = None;
    let mut js_bundle_src: Option<String> = None;

    // Step 3: CSS bundling.
    if config.css && !refs.css_hrefs.is_empty() {
        let merged = css::merge_css_files(&refs.css_hrefs, dist_dir)
            .wrap_err("Failed to merge CSS files")?;

        if merged.is_empty() {
            tracing::warn!("Bundling: merged CSS is empty (all files failed to load)");
        } else {
            let output_css = if config.tree_shake_css {
                css::tree_shake_css(&merged, &refs.html_files, minify_css)
                    .wrap_err("CSS tree-shaking failed")?
            } else if minify_css {
                // Minify without tree-shaking: parse and re-serialize.
                minify_css_string(&merged)?
            } else {
                merged
            };

            // Write bundled CSS file.
            let output_path = dist_dir.join(&config.css_output);

            // Warn if output path already exists.
            if output_path.exists() {
                tracing::warn!(
                    "Bundling: overwriting existing file at '{}'",
                    config.css_output,
                );
            }

            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!(
                        "Failed to create directory for CSS bundle: {}",
                        parent.display()
                    ))?;
            }

            std::fs::write(&output_path, &output_css)
                .wrap_err_with(|| format!(
                    "Failed to write CSS bundle to '{}'",
                    output_path.display()
                ))?;

            let href = format!("/{}", config.css_output);
            css_bundle_href = Some(href);
            generated_files.push(config.css_output.clone());

            tracing::info!(
                "Bundling: wrote CSS bundle ({} bytes) to {}",
                output_css.len(),
                config.css_output,
            );
        }
    }

    // Step 4: JS bundling.
    if config.js && !refs.js_srcs.is_empty() {
        let merged = js::merge_js_files(&refs.js_srcs, dist_dir)
            .wrap_err("Failed to merge JS files")?;

        if merged.is_empty() {
            tracing::warn!("Bundling: merged JS is empty (all files failed to load)");
        } else {
            // Write bundled JS file.
            let output_path = dist_dir.join(&config.js_output);

            if output_path.exists() {
                tracing::warn!(
                    "Bundling: overwriting existing file at '{}'",
                    config.js_output,
                );
            }

            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!(
                        "Failed to create directory for JS bundle: {}",
                        parent.display()
                    ))?;
            }

            std::fs::write(&output_path, &merged)
                .wrap_err_with(|| format!(
                    "Failed to write JS bundle to '{}'",
                    output_path.display()
                ))?;

            let src = format!("/{}", config.js_output);
            js_bundle_src = Some(src);
            generated_files.push(config.js_output.clone());

            tracing::info!(
                "Bundling: wrote JS bundle ({} bytes) to {}",
                merged.len(),
                config.js_output,
            );
        }
    }

    // Step 5: Rewrite all HTML files.
    if css_bundle_href.is_some() || js_bundle_src.is_some() {
        rewrite::rewrite_html_for_bundles(
            &refs.html_files,
            dist_dir,
            css_bundle_href.as_deref(),
            js_bundle_src.as_deref(),
            &refs.css_hrefs,
            &refs.js_srcs,
        ).wrap_err("Failed to rewrite HTML files for bundles")?;

        tracing::info!(
            "Bundling: rewrote {} HTML file(s)",
            refs.html_files.len(),
        );
    }

    Ok(generated_files)
}

/// Minify a CSS string using lightningcss without tree-shaking.
fn minify_css_string(css: &str) -> Result<String> {
    use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};

    let options = ParserOptions {
        error_recovery: true,
        ..ParserOptions::default()
    };
    let stylesheet = StyleSheet::parse(css, options)
        .map_err(|e| eyre::eyre!("CSS parse error during minification: {e}"))?;

    let result = stylesheet.to_css(PrinterOptions {
        minify: true,
        ..PrinterOptions::default()
    }).map_err(|e| eyre::eyre!("CSS serialization error: {e}"))?;

    Ok(result.code)
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
    fn test_bundle_assets_disabled() {
        let tmp = TempDir::new().unwrap();
        let config = BundlingConfig::default(); // enabled: false
        let result = bundle_assets(tmp.path(), &config, false).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_bundle_assets_no_refs() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "index.html", "<html><body>No CSS or JS</body></html>");

        let config = BundlingConfig { enabled: true, ..Default::default() };
        let result = bundle_assets(dist, &config, false).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_bundle_assets_css_only() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/style.css", ".hero { color: red; }");
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hi</div></body></html>"#);

        let config = BundlingConfig {
            enabled: true,
            js: false,
            ..Default::default()
        };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result, vec!["css/bundle.css"]);
        assert!(dist.join("css/bundle.css").exists());

        // HTML should reference the bundle.
        let html = fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(html.contains("/css/bundle.css"));
    }

    #[test]
    fn test_bundle_assets_js_only() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "js/app.js", "var x = 1;");
        write_file(dist, "index.html", r#"<html><body><script src="/js/app.js"></script></body></html>"#);

        let config = BundlingConfig {
            enabled: true,
            css: false,
            ..Default::default()
        };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result, vec!["js/bundle.js"]);
        assert!(dist.join("js/bundle.js").exists());

        // HTML should reference the bundle.
        let html = fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(html.contains("/js/bundle.js"));
    }

    #[test]
    fn test_bundle_assets_end_to_end() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        write_file(dist, "css/reset.css", "* { margin: 0; }");
        write_file(dist, "css/style.css", ".hero { color: red; } .unused { display: none; }");
        write_file(dist, "js/utils.js", "function util() {}");
        write_file(dist, "js/app.js", "util();");
        write_file(dist, "index.html", r#"<html><head>
            <link rel="stylesheet" href="/css/reset.css">
            <link rel="stylesheet" href="/css/style.css">
        </head><body>
            <div class="hero">Hello</div>
            <script src="/js/utils.js"></script>
            <script src="/js/app.js"></script>
        </body></html>"#);

        let config = BundlingConfig { enabled: true, ..Default::default() };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains(&"css/bundle.css".to_string()));
        assert!(result.contains(&"js/bundle.js".to_string()));

        // Verify CSS bundle: tree-shaking should keep .hero and * but not .unused.
        let css_bundle = fs::read_to_string(dist.join("css/bundle.css")).unwrap();
        assert!(css_bundle.contains(".hero"));
        assert!(!css_bundle.contains(".unused"));

        // Verify JS bundle: both files wrapped in IIFEs.
        let js_bundle = fs::read_to_string(dist.join("js/bundle.js")).unwrap();
        assert!(js_bundle.contains("function util()"));
        assert!(js_bundle.contains("util();"));
        assert_eq!(js_bundle.matches(";(function(){").count(), 2);

        // Verify HTML rewriting.
        let html = fs::read_to_string(dist.join("index.html")).unwrap();
        assert!(html.contains("/css/bundle.css"));
        assert!(html.contains("/js/bundle.js"));
        assert!(!html.contains("/css/reset.css"));
        assert!(!html.contains("/css/style.css"));
        assert!(!html.contains("/js/utils.js"));
        assert!(!html.contains("/js/app.js"));
    }

    #[test]
    fn test_bundle_assets_output_path_conflict() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();

        // Pre-existing file at bundle output path.
        write_file(dist, "css/bundle.css", "/* old content */");
        write_file(dist, "css/style.css", ".hero { color: red; }");
        write_file(dist, "index.html", r#"<html><head><link rel="stylesheet" href="/css/style.css"></head><body><div class="hero">Hi</div></body></html>"#);

        let config = BundlingConfig {
            enabled: true,
            js: false,
            ..Default::default()
        };
        let result = bundle_assets(dist, &config, false).unwrap();

        assert_eq!(result, vec!["css/bundle.css"]);

        // Old content should be overwritten.
        let bundle = fs::read_to_string(dist.join("css/bundle.css")).unwrap();
        assert!(!bundle.contains("old content"));
        assert!(bundle.contains(".hero"));
    }
}
