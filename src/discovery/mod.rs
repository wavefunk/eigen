use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail};
use regex::Regex;
use walkdir::WalkDir;

use crate::config::SiteConfig;
use crate::frontmatter::{self, DataQuery, Frontmatter};

/// Whether a template produces a single page or many pages from a collection.
#[derive(Debug, Clone)]
pub enum PageType {
    /// A regular template that produces exactly one output page.
    Static,
    /// A template like `[post].html` that produces one page per collection item.
    Dynamic {
        /// The parameter name extracted from the filename, e.g. `"post"` from `[post].html`.
        param_name: String,
    },
}

/// A discovered page template with all metadata needed for rendering.
#[derive(Debug, Clone)]
pub struct PageDef {
    /// Path to the template file, relative to the `templates/` directory.
    pub template_path: PathBuf,
    /// Whether this is a static or dynamic page.
    pub page_type: PageType,
    /// Output directory relative to `dist/` (the parent dir where output goes).
    pub output_dir: PathBuf,
    /// Parsed frontmatter from the template.
    pub frontmatter: Frontmatter,
    /// Template body with frontmatter stripped.
    pub template_body: String,
}

/// Walk the `templates/` directory and discover all renderable pages.
///
/// Skips files and directories whose names start with `_` (layouts, partials).
/// Returns a `Vec<PageDef>` with frontmatter parsed and page types classified.
///
/// Validates:
/// - Dynamic pages must have a `collection` in their frontmatter.
/// - All `source` references in data queries must exist in the site config.
pub fn discover_pages(project_root: &Path, config: &SiteConfig) -> Result<Vec<PageDef>> {
    let templates_dir = project_root.join("templates");
    if !templates_dir.is_dir() {
        eprintln!(
            "Warning: templates/ directory not found at {}. No pages to build.",
            templates_dir.display()
        );
        return Ok(Vec::new());
    }

    let bracket_re = Regex::new(r"^\[([A-Za-z_][A-Za-z0-9_]*)\]\.html$").unwrap();

    let mut pages = Vec::new();
    let mut static_count = 0u32;
    let mut dynamic_count = 0u32;

    for entry in WalkDir::new(&templates_dir)
        .into_iter()
        .filter_entry(|e| !is_underscore_prefixed(e.file_name()))
    {
        let entry = entry.wrap_err("Failed to read entry while walking templates/")?;

        // Only process files.
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Only process .html files.
        match path.extension().and_then(|e| e.to_str()) {
            Some("html") => {}
            _ => continue,
        }

        // Path relative to templates/.
        let rel_path = path
            .strip_prefix(&templates_dir)
            .wrap_err("Template path is not inside templates/ directory")?;

        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        // Classify: dynamic ([name].html) or static.
        let page_type = if let Some(caps) = bracket_re.captures(filename) {
            let param_name = caps[1].to_string();
            PageType::Dynamic { param_name }
        } else {
            PageType::Static
        };

        // Output directory: parent of the template path relative to templates/.
        // e.g. templates/posts/[post].html -> output_dir = "posts"
        // e.g. templates/index.html -> output_dir = ""
        let output_dir = rel_path
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();

        // Read the file content and extract frontmatter.
        let content = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("Failed to read template {}", path.display()))?;

        let display_path = rel_path.display().to_string();
        let (fm, body) = frontmatter::extract_frontmatter(&content, &display_path)?;

        // Validate: dynamic pages MUST have a collection.
        if let PageType::Dynamic { ref param_name } = page_type {
            if fm.collection.is_none() {
                bail!(
                    "Dynamic template {} (parameter \"{}\") is missing a `collection` \
                     in its frontmatter. Dynamic pages must define a collection query \
                     so Eigen knows which items to generate pages for.",
                    display_path,
                    param_name,
                );
            }
        }

        // Validate: all source references must exist in config.
        validate_source_references(&fm, &display_path, config)?;

        match &page_type {
            PageType::Static => static_count += 1,
            PageType::Dynamic { .. } => dynamic_count += 1,
        }

        pages.push(PageDef {
            template_path: rel_path.to_path_buf(),
            page_type,
            output_dir,
            frontmatter: fm,
            template_body: body.to_string(),
        });
    }

    tracing::debug!(
        "Found {} static page(s), {} dynamic template(s)",
        static_count, dynamic_count
    );

    Ok(pages)
}

/// Check whether a file/directory name starts with `_`.
fn is_underscore_prefixed(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .map(|s| s.starts_with('_'))
        .unwrap_or(false)
}

/// Validate that every `source` field referenced in any `DataQuery` within the
/// frontmatter actually exists in `config.sources`. Fails early with a helpful
/// error listing the available sources.
fn validate_source_references(
    fm: &Frontmatter,
    template_path: &str,
    config: &SiteConfig,
) -> Result<()> {
    let mut to_check: Vec<(&str, &DataQuery)> = Vec::new();

    if let Some(ref coll) = fm.collection {
        to_check.push(("collection", coll));
    }
    for (name, query) in &fm.data {
        to_check.push((name.as_str(), query));
    }

    let available: Vec<&str> = config.sources.keys().map(|s| s.as_str()).collect();

    for (query_name, query) in to_check {
        if let Some(ref source_name) = query.source {
            if !config.sources.contains_key(source_name) {
                bail!(
                    "Template {} references source \"{}\" in data query \"{}\", \
                     but it is not defined in site.toml [sources.*].\n\
                     Available sources: {}",
                    template_path,
                    source_name,
                    query_name,
                    if available.is_empty() {
                        "(none)".to_string()
                    } else {
                        available.join(", ")
                    },
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    use crate::config::{BuildConfig, SiteMeta, SourceConfig};

    /// Create a minimal SiteConfig for testing.
    fn test_config() -> SiteConfig {
        let mut sources = HashMap::new();
        sources.insert(
            "blog_api".into(),
            SourceConfig {
                url: "https://api.example.com".into(),
                headers: HashMap::new(),
            },
        );
        SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://test.com".into(),
            },
            build: BuildConfig::default(),
            assets: Default::default(),
            sources,
            plugins: HashMap::new(),
        }
    }

    /// Helper to write a file creating parent dirs as needed.
    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_discover_static_pages() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        write(root, "templates/index.html", "<h1>Home</h1>");
        write(root, "templates/about.html", "<h1>About</h1>");

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 2);

        for page in &pages {
            assert!(matches!(page.page_type, PageType::Static));
            assert!(page.frontmatter.collection.is_none());
        }
    }

    #[test]
    fn test_discover_dynamic_page() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        let dynamic_content = "---\ncollection:\n  source: blog_api\n  path: /posts\n---\n<h1>{{ post.title }}</h1>";

        write(root, "templates/posts/[post].html", dynamic_content);

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 1);

        let page = &pages[0];
        match &page.page_type {
            PageType::Dynamic { param_name } => {
                assert_eq!(param_name, "post");
            }
            _ => panic!("Expected dynamic page"),
        }
        assert!(page.frontmatter.collection.is_some());
        assert_eq!(page.output_dir, PathBuf::from("posts"));
    }

    #[test]
    fn test_skip_underscore_prefixed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        write(root, "templates/index.html", "<h1>Home</h1>");
        write(root, "templates/_base.html", "<!DOCTYPE html>");
        write(root, "templates/_partials/nav.html", "<nav>nav</nav>");

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].template_path, PathBuf::from("index.html"));
    }

    #[test]
    fn test_nested_static_pages() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        write(root, "templates/index.html", "<h1>Home</h1>");
        write(root, "templates/docs/getting-started.html", "<h1>Docs</h1>");
        write(root, "templates/docs/api/reference.html", "<h1>API</h1>");

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 3);

        let docs_page = pages
            .iter()
            .find(|p| p.template_path == PathBuf::from("docs/getting-started.html"))
            .unwrap();
        assert_eq!(docs_page.output_dir, PathBuf::from("docs"));

        let api_page = pages
            .iter()
            .find(|p| p.template_path == PathBuf::from("docs/api/reference.html"))
            .unwrap();
        assert_eq!(api_page.output_dir, PathBuf::from("docs/api"));
    }

    #[test]
    fn test_dynamic_page_missing_collection_errors() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        // Dynamic filename but no collection in frontmatter.
        write(root, "templates/[post].html", "<h1>Bad dynamic</h1>");

        let result = discover_pages(root, &config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("collection"));
        assert!(err.contains("[post].html"));
    }

    #[test]
    fn test_invalid_source_reference_errors() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        let content = "---\ndata:\n  items:\n    source: nonexistent_api\n    path: /items\n---\n<h1>Items</h1>";

        write(root, "templates/items.html", content);

        let result = discover_pages(root, &config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent_api"));
        assert!(err.contains("blog_api")); // should list available sources
    }

    #[test]
    fn test_valid_source_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        let content = "---\ndata:\n  posts:\n    source: blog_api\n    path: /posts\n---\n<h1>Posts</h1>";

        write(root, "templates/posts.html", content);

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn test_frontmatter_attached_to_page() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        let content = "---\ndata:\n  nav:\n    file: \"nav.yaml\"\n---\n<h1>{{ nav }}</h1>";

        write(root, "templates/index.html", content);

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].frontmatter.data.len(), 1);
        assert!(pages[0].frontmatter.data.contains_key("nav"));
        assert_eq!(pages[0].template_body, "<h1>{{ nav }}</h1>");
    }

    #[test]
    fn test_no_templates_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        // No templates/ directory at all.
        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 0);
    }

    #[test]
    fn test_non_html_files_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        write(root, "templates/index.html", "<h1>Home</h1>");
        write(root, "templates/readme.md", "# Readme");
        write(root, "templates/data.json", "{}");

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn test_output_paths() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        write(root, "templates/index.html", "home");
        write(root, "templates/about.html", "about");
        write(root, "templates/posts/index.html", "posts index");

        let pages = discover_pages(root, &config).unwrap();
        assert_eq!(pages.len(), 3);

        let index = pages
            .iter()
            .find(|p| p.template_path == PathBuf::from("index.html"))
            .unwrap();
        assert_eq!(index.output_dir, PathBuf::from(""));

        let about = pages
            .iter()
            .find(|p| p.template_path == PathBuf::from("about.html"))
            .unwrap();
        assert_eq!(about.output_dir, PathBuf::from(""));

        let posts_index = pages
            .iter()
            .find(|p| p.template_path == PathBuf::from("posts/index.html"))
            .unwrap();
        assert_eq!(posts_index.output_dir, PathBuf::from("posts"));
    }

    #[test]
    fn test_collection_source_validation() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = test_config();

        let content = "---\ncollection:\n  source: nonexistent_source\n  path: /items\n---\n<h1>{{ item.title }}</h1>";

        write(root, "templates/[item].html", content);

        let result = discover_pages(root, &config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent_source"));
        assert!(err.contains("collection"));
    }
}
