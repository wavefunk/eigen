//! CSS merging and tree-shaking.
//!
//! Merges multiple CSS files into a single string, resolving @import chains.
//! Tree-shakes by matching selectors against all rendered HTML DOMs --
//! a selector is kept if it matches in ANY page.
//!
//! Reuses public primitives from `critical_css/extract.rs`:
//! - `strip_pseudo_for_matching` for pseudo-class/element stripping
//! - `selector_matches` for DOM matching via scraper
//! - `collect_global_deps` and `GlobalDependencies` for transitive
//!   dependency tracking (@font-face, @keyframes, custom properties)

use std::path::{Path, PathBuf};

use eyre::Result;
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;

use crate::build::critical_css::extract::{
    collect_global_deps, selector_matches, strip_pseudo_for_matching,
    GlobalDependencies,
};

/// Read and merge multiple CSS files into a single string.
///
/// Files are concatenated in the order given (first-encounter order from
/// HTML scanning). Each file's content is preceded by a source comment.
///
/// `@import` directives within each file are resolved recursively.
/// Since bundling runs before content hashing, no manifest resolution
/// is needed.
///
/// Missing files are skipped with a warning logged.
pub fn merge_css_files(
    hrefs: &[String],
    dist_dir: &Path,
) -> Result<String> {
    let mut merged = String::new();
    let mut files_merged = 0u32;

    for href in hrefs {
        let relative = href.trim_start_matches('/');
        let file_path = dist_dir.join(relative);

        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "CSS bundling: file '{}' not found, skipping: {}",
                    file_path.display(), e
                );
                continue;
            }
        };

        // Resolve @import directives.
        let resolved = resolve_imports(&content, &file_path, dist_dir, 0);

        if !merged.is_empty() {
            merged.push('\n');
        }

        // Source comment for traceability (stripped by lightningcss minifier).
        merged.push_str("/* Source: ");
        merged.push_str(href);
        merged.push_str(" */\n");
        merged.push_str(&resolved);

        files_merged += 1;
    }

    if files_merged == 0 && !hrefs.is_empty() {
        tracing::warn!("CSS bundling: all {} CSS file(s) failed to load", hrefs.len());
    }

    Ok(merged)
}

/// Matches `@import` directives in CSS (both `url()` and string forms).
static IMPORT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"@import\s+(?:url\(\s*['"]?([^'")]+)['"]?\s*\)|['"]([^'"]+)['"]);?"#
    ).expect("import regex is valid")
});

/// Recursively resolve `@import` directives in CSS content.
///
/// This is equivalent to the `resolve_imports` in `critical_css/mod.rs`
/// but without manifest-based path resolution (bundling runs before
/// content hashing, so all files are at their original paths).
fn resolve_imports(
    css: &str,
    css_path: &Path,
    dist_dir: &Path,
    depth: usize,
) -> String {
    const MAX_IMPORT_DEPTH: usize = 10;

    if depth > MAX_IMPORT_DEPTH {
        tracing::warn!(
            "Circular or deeply nested @import detected in {}",
            css_path.display()
        );
        return css.to_string();
    }

    let css_dir = css_path.parent().unwrap_or(dist_dir);
    let mut result = css.to_string();

    let captures: Vec<_> = IMPORT_RE.captures_iter(css).collect();

    for cap in captures {
        let import_path_str = cap.get(1).or(cap.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");

        // Skip external imports.
        if import_path_str.starts_with("http://") || import_path_str.starts_with("https://") {
            continue;
        }

        if import_path_str.is_empty() {
            continue;
        }

        // Resolve relative to the importing file's directory.
        let import_path = if import_path_str.starts_with('/') {
            dist_dir.join(import_path_str.trim_start_matches('/'))
        } else {
            css_dir.join(import_path_str)
        };

        if import_path.exists() {
            match std::fs::read_to_string(&import_path) {
                Ok(imported_css) => {
                    let resolved = resolve_imports(
                        &imported_css, &import_path, dist_dir, depth + 1
                    );
                    if let Some(full_match) = cap.get(0) {
                        result = result.replace(full_match.as_str(), &resolved);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to read @import '{}': {e}",
                        import_path.display(),
                    );
                }
            }
        } else {
            tracing::warn!(
                "File not found for @import '{}' in {}",
                import_path_str, css_path.display()
            );
        }
    }

    result
}

/// Tree-shake CSS: remove selectors that do not match any element in any
/// rendered HTML page.
///
/// Steps:
/// 1. Parse every HTML file into a `scraper::Html` DOM.
/// 2. Parse the merged CSS with lightningcss (error recovery enabled).
/// 3. Walk all CSS rules. For each style rule, test its selectors against
///    every DOM. If a selector matches in at least one DOM, keep the rule.
/// 4. Include transitively referenced @font-face, @keyframes, and custom
///    property rules (same dependency tracking as critical_css/extract.rs).
/// 5. Serialize the surviving rules to a string.
///
/// Returns the tree-shaken CSS string.
pub fn tree_shake_css(
    merged_css: &str,
    html_files: &[PathBuf],
    minify: bool,
) -> Result<String> {
    // Step 1: Parse all HTML files into DOMs.
    let documents: Vec<scraper::Html> = html_files
        .iter()
        .filter_map(|path| {
            match std::fs::read_to_string(path) {
                Ok(html) => Some(scraper::Html::parse_document(&html)),
                Err(e) => {
                    tracing::warn!(
                        "Tree-shaking: failed to read HTML '{}': {}",
                        path.display(), e
                    );
                    None
                }
            }
        })
        .collect();

    if documents.is_empty() {
        return Ok(merged_css.to_string());
    }

    // Step 2: Parse CSS.
    let options = ParserOptions {
        error_recovery: true,
        ..ParserOptions::default()
    };
    let stylesheet = StyleSheet::parse(merged_css, options)
        .map_err(|e| eyre::eyre!("CSS parse error during tree-shaking: {e}"))?;

    // Step 3: Walk rules, matching against all DOMs.
    let mut matched_rules: Vec<String> = Vec::new();
    let mut global_deps = GlobalDependencies::default();

    let mut font_face_rules: Vec<String> = Vec::new();
    let mut keyframe_rules: Vec<String> = Vec::new();
    let mut custom_prop_rules: Vec<(String, String)> = Vec::new();

    walk_rules(
        &stylesheet.rules.0,
        &documents,
        &mut matched_rules,
        &mut global_deps,
        &mut font_face_rules,
        &mut keyframe_rules,
        &mut custom_prop_rules,
        minify,
    );

    if matched_rules.is_empty() {
        tracing::info!("Tree-shaking: no CSS selectors matched any page");
        return Ok(String::new());
    }

    // Step 4: Include transitively referenced global rules.
    let mut critical_parts: Vec<String> = Vec::new();

    // Include referenced @font-face rules.
    for rule_css in &font_face_rules {
        for family in &global_deps.font_families {
            if rule_css.contains(family.as_str()) {
                critical_parts.push(rule_css.clone());
                break;
            }
        }
    }

    // Include referenced @keyframes rules.
    for rule_css in &keyframe_rules {
        for anim_name in &global_deps.animation_names {
            if rule_css.contains(anim_name.as_str()) {
                critical_parts.push(rule_css.clone());
                break;
            }
        }
    }

    // Include referenced custom property definitions.
    for (prop_name, rule_css) in &custom_prop_rules {
        if global_deps.custom_properties.contains(prop_name) {
            critical_parts.push(rule_css.clone());
        }
    }

    // Add matched style rules.
    critical_parts.extend(matched_rules);

    Ok(critical_parts.join("\n"))
}

/// Create printer options with optional minification.
fn printer(minify: bool) -> PrinterOptions<'static> {
    PrinterOptions {
        minify,
        ..PrinterOptions::default()
    }
}

/// Recursively walk CSS rules, matching style rules against ALL DOMs.
///
/// This is the bundling variant of the walk_rules in critical_css/extract.rs.
/// The key difference: selectors are tested against multiple documents (the
/// union of all pages), not a single document.
#[allow(clippy::too_many_arguments)]
fn walk_rules(
    rules: &[CssRule],
    documents: &[scraper::Html],
    matched_rules: &mut Vec<String>,
    global_deps: &mut GlobalDependencies,
    font_face_rules: &mut Vec<String>,
    keyframe_rules: &mut Vec<String>,
    custom_prop_rules: &mut Vec<(String, String)>,
    minify: bool,
) {
    for rule in rules {
        match rule {
            CssRule::Style(style_rule) => {
                handle_style_rule(
                    style_rule, documents, matched_rules,
                    global_deps, custom_prop_rules, minify,
                );
            }
            CssRule::Media(media_rule) => {
                let mut media_matched: Vec<String> = Vec::new();
                walk_rules(
                    &media_rule.rules.0, documents, &mut media_matched,
                    global_deps, font_face_rules, keyframe_rules,
                    custom_prop_rules, minify,
                );
                if !media_matched.is_empty() {
                    let query = media_rule.query
                        .to_css_string(printer(minify))
                        .unwrap_or_default();
                    let inner = media_matched.join("\n");
                    matched_rules.push(format!("@media {query} {{\n{inner}\n}}"));
                }
            }
            CssRule::Supports(supports_rule) => {
                let mut supports_matched: Vec<String> = Vec::new();
                walk_rules(
                    &supports_rule.rules.0, documents, &mut supports_matched,
                    global_deps, font_face_rules, keyframe_rules,
                    custom_prop_rules, minify,
                );
                if !supports_matched.is_empty() {
                    let condition = supports_rule.condition
                        .to_css_string(printer(minify))
                        .unwrap_or_default();
                    let inner = supports_matched.join("\n");
                    matched_rules.push(format!("@supports {condition} {{\n{inner}\n}}"));
                }
            }
            CssRule::LayerBlock(layer_rule) => {
                let mut layer_matched: Vec<String> = Vec::new();
                walk_rules(
                    &layer_rule.rules.0, documents, &mut layer_matched,
                    global_deps, font_face_rules, keyframe_rules,
                    custom_prop_rules, minify,
                );
                if !layer_matched.is_empty() {
                    let name = layer_rule.name.as_ref()
                        .and_then(|n| n.to_css_string(printer(minify)).ok())
                        .unwrap_or_default();
                    let inner = layer_matched.join("\n");
                    if name.is_empty() {
                        matched_rules.push(format!("@layer {{\n{inner}\n}}"));
                    } else {
                        matched_rules.push(format!("@layer {name} {{\n{inner}\n}}"));
                    }
                }
            }
            CssRule::LayerStatement(_) => {
                if let Ok(css) = rule.to_css_string(printer(minify)) {
                    matched_rules.push(css);
                }
            }
            CssRule::FontFace(_) => {
                if let Ok(css) = rule.to_css_string(printer(minify)) {
                    font_face_rules.push(css);
                }
            }
            CssRule::Keyframes(_) => {
                if let Ok(css) = rule.to_css_string(printer(minify)) {
                    keyframe_rules.push(css);
                }
            }
            CssRule::Import(_) | CssRule::Namespace(_) => {
                if let Ok(css) = rule.to_css_string(printer(minify)) {
                    matched_rules.push(css);
                }
            }
            _ => {
                // Unknown at-rules: include by default.
                if let Ok(css) = rule.to_css_string(printer(minify)) {
                    matched_rules.push(css);
                }
            }
        }
    }
}

/// Handle a single style rule: check if its selectors match ANY DOM,
/// and if so, serialize it and collect its global dependencies.
fn handle_style_rule(
    style_rule: &lightningcss::rules::style::StyleRule,
    documents: &[scraper::Html],
    matched_rules: &mut Vec<String>,
    global_deps: &mut GlobalDependencies,
    custom_prop_rules: &mut Vec<(String, String)>,
    minify: bool,
) {
    let selector_list = style_rule.selectors
        .to_css_string(printer(minify))
        .unwrap_or_default();

    let css_text = match style_rule.to_css_string(printer(minify)) {
        Ok(css) => css,
        Err(_) => return,
    };

    // Collect custom property definitions from this rule regardless of
    // whether it matches (needed for var() dependency resolution).
    static CUSTOM_PROP_DEF_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(--[a-zA-Z0-9_-]+)\s*:").expect("custom prop regex is valid")
    });

    for cap in CUSTOM_PROP_DEF_RE.captures_iter(&css_text) {
        custom_prop_rules.push((cap[1].to_string(), css_text.clone()));
    }

    // Check each selector in the comma-separated list against ALL documents.
    let mut any_match = false;
    for selector in selector_list.split(',') {
        let selector = selector.trim();
        if selector.is_empty() {
            continue;
        }

        match strip_pseudo_for_matching(selector) {
            None => {
                // Pseudo-only selector (e.g. ::selection) -> include unconditionally.
                any_match = true;
                break;
            }
            Some(stripped) => {
                // Match against ANY document.
                for doc in documents {
                    if selector_matches(&stripped, doc) {
                        any_match = true;
                        break;
                    }
                }
                if any_match {
                    break;
                }
            }
        }
    }

    if any_match {
        let deps = collect_global_deps(&css_text);
        global_deps.font_families.extend(deps.font_families);
        global_deps.animation_names.extend(deps.animation_names);
        global_deps.custom_properties.extend(deps.custom_properties);

        matched_rules.push(css_text);
    }
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

    // --- merge_css_files tests ---

    #[test]
    fn test_merge_single_file() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/style.css", "body { color: red; }");

        let result = merge_css_files(&["/css/style.css".to_string()], dist).unwrap();
        assert!(result.contains("/* Source: /css/style.css */"));
        assert!(result.contains("body { color: red; }"));
    }

    #[test]
    fn test_merge_multiple_files() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/reset.css", "* { margin: 0; }");
        write_file(dist, "css/style.css", "body { color: red; }");

        let result = merge_css_files(
            &["/css/reset.css".to_string(), "/css/style.css".to_string()],
            dist,
        ).unwrap();

        assert!(result.contains("/* Source: /css/reset.css */"));
        assert!(result.contains("/* Source: /css/style.css */"));

        // reset.css should appear before style.css.
        let reset_pos = result.find("* { margin: 0; }").unwrap();
        let style_pos = result.find("body { color: red; }").unwrap();
        assert!(reset_pos < style_pos);
    }

    #[test]
    fn test_merge_resolves_import() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/base.css", ".base { margin: 0; }");
        write_file(dist, "css/style.css", "@import \"base.css\";\n.main { color: red; }");

        let result = merge_css_files(&["/css/style.css".to_string()], dist).unwrap();

        // @import should be resolved (base.css content inlined).
        assert!(result.contains(".base { margin: 0; }"));
        assert!(result.contains(".main { color: red; }"));
        assert!(!result.contains("@import"));
    }

    #[test]
    fn test_merge_missing_file() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "css/exists.css", ".ok { color: green; }");

        let result = merge_css_files(
            &["/css/missing.css".to_string(), "/css/exists.css".to_string()],
            dist,
        ).unwrap();

        // Missing file skipped, existing file included.
        assert!(!result.contains("missing"));
        assert!(result.contains(".ok { color: green; }"));
    }

    // --- tree_shake_css tests ---

    #[test]
    fn test_tree_shake_removes_unused() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><div class="used">Hello</div></body></html>"#);

        let css = ".used { color: red; } .unused { color: blue; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains(".used"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_tree_shake_keeps_used() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><p class="hero">Hi</p></body></html>"#);

        let css = ".hero { color: red; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains(".hero"));
        assert!(result.contains("color"));
    }

    #[test]
    fn test_tree_shake_keeps_pseudo() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><a class="link">Click</a></body></html>"#);

        let css = ".link { color: blue; } .link:hover { color: red; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        // Both the base rule and :hover rule should be kept.
        assert!(result.contains(".link"));
        assert!(result.contains("blue") || result.contains("red"));
    }

    #[test]
    fn test_tree_shake_media_query() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><div class="card">Card</div></body></html>"#);

        let css = r#"
            @media (max-width: 768px) {
                .card { width: 100%; }
                .sidebar { display: none; }
            }
        "#;
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains("@media"));
        assert!(result.contains(".card"));
        assert!(!result.contains(".sidebar"));
    }

    #[test]
    fn test_tree_shake_font_face() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><h1 class="heading">Title</h1></body></html>"#);

        let css = r#"
            @font-face {
                font-family: "Inter";
                src: url("/fonts/inter.woff2") format("woff2");
            }
            .heading { font-family: "Inter", sans-serif; }
            .unused { color: green; }
        "#;
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains("@font-face"));
        assert!(result.contains("Inter"));
        assert!(result.contains(".heading"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_tree_shake_keyframes() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", r#"<html><body><div class="spinner">Loading</div></body></html>"#);

        let css = r#"
            @keyframes spin {
                from { transform: rotate(0deg); }
                to { transform: rotate(360deg); }
            }
            .spinner { animation-name: spin; }
            .unused { color: green; }
        "#;
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.contains("@keyframes"));
        assert!(result.contains("spin"));
        assert!(result.contains(".spinner"));
        assert!(!result.contains(".unused"));
    }

    #[test]
    fn test_tree_shake_empty_result() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "page.html", "<html><body><div>No matching classes</div></body></html>");

        let css = ".missing-a { color: red; } .missing-b { color: blue; }";
        let html_files = vec![dist.join("page.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn test_tree_shake_multiple_docs() {
        let tmp = TempDir::new().unwrap();
        let dist = tmp.path();
        write_file(dist, "a.html", r#"<html><body><div class="a-only">A</div></body></html>"#);
        write_file(dist, "b.html", r#"<html><body><div class="b-only">B</div></body></html>"#);

        let css = ".a-only { color: red; } .b-only { color: blue; } .neither { color: green; }";
        let html_files = vec![dist.join("a.html"), dist.join("b.html")];
        let result = tree_shake_css(css, &html_files, false).unwrap();

        // Selectors used in either doc should be kept.
        assert!(result.contains(".a-only"));
        assert!(result.contains(".b-only"));
        // Selector used in neither should be removed.
        assert!(!result.contains(".neither"));
    }
}
