//! Phase 8, Step 8.1: Project scaffolding.
//!
//! `eigen init my-site` creates a new project directory with a minimal but
//! fully functional site that can be built and served immediately.

use eyre::{Result, WrapErr, bail};
use std::path::Path;

/// Scaffold a new Eigen project at the given directory name.
///
/// Creates the directory and all necessary files inside it. Errors if the
/// directory already exists.
pub fn init_project(name: &str) -> Result<()> {
    let project_dir = Path::new(name);

    if project_dir.exists() {
        bail!(
            "Directory '{}' already exists. Choose a different name or remove it first.",
            name,
        );
    }

    // Create the directory tree.
    let dirs = [
        "",
        "templates",
        "templates/_partials",
        "_data",
        "static/css",
    ];

    for dir in &dirs {
        let path = project_dir.join(dir);
        std::fs::create_dir_all(&path)
            .wrap_err_with(|| format!("Failed to create directory {}", path.display()))?;
    }

    // Write all scaffold files.
    write_file(project_dir, "site.toml", SITE_TOML)?;
    write_file(project_dir, "templates/_base.html", BASE_HTML)?;
    write_file(project_dir, "templates/_partials/nav.html", NAV_HTML)?;
    write_file(project_dir, "templates/index.html", INDEX_HTML)?;
    write_file(project_dir, "templates/about.html", ABOUT_HTML)?;
    write_file(project_dir, "_data/nav.yaml", NAV_YAML)?;
    write_file(project_dir, "static/css/style.css", STYLE_CSS)?;
    write_file(project_dir, ".gitignore", GITIGNORE)?;

    Ok(())
}

/// Write a file relative to the project directory.
fn write_file(project_dir: &Path, rel_path: &str, content: &str) -> Result<()> {
    let path = project_dir.join(rel_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("Failed to create directory {}", parent.display()))?;
    }
    std::fs::write(&path, content)
        .wrap_err_with(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scaffold file contents
// ---------------------------------------------------------------------------

const SITE_TOML: &str = r#"[site]
name = "My Eigen Site"
base_url = "http://localhost:3000"

[build]
fragments = true
fragment_dir = "_fragments"
content_block = "content"
"#;

const BASE_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{% block title %}{{ site.name }}{% endblock %}</title>
    <link rel="stylesheet" href="{{ asset('css/style.css') }}">
    <script src="https://unpkg.com/htmx.org@2.0.4"></script>
</head>
<body>
    {% include "_partials/nav.html" %}
    <main id="content">
        {% block content %}{% endblock %}
    </main>
    <footer>
        <p>&copy; {{ current_year() }} {{ site.name }}</p>
    </footer>
</body>
</html>
"#;

const NAV_HTML: &str = r#"<nav>
    <ul>
        {% for item in nav %}
        <li><a {{ link_to(item.url) }}>{{ item.label }}</a></li>
        {% endfor %}
    </ul>
</nav>
"#;

const INDEX_HTML: &str = r#"---
data:
  nav:
    file: "nav.yaml"
---
{% extends "_base.html" %}

{% block title %}Home — {{ site.name }}{% endblock %}

{% block content %}
<h1>Welcome to {{ site.name }}</h1>
<p>Your new site is ready. Start editing templates in the <code>templates/</code> directory.</p>
<p>Run <code>eigen dev</code> to start the development server with live reload.</p>
{% endblock %}
"#;

const ABOUT_HTML: &str = r#"---
data:
  nav:
    file: "nav.yaml"
---
{% extends "_base.html" %}

{% block title %}About — {{ site.name }}{% endblock %}

{% block content %}
<h1>About</h1>
<p>This is a site built with <strong>Eigen</strong>, a static site generator with HTMX support.</p>
{% endblock %}
"#;

const NAV_YAML: &str = r#"- label: Home
  url: /index.html
- label: About
  url: /about.html
"#;

const STYLE_CSS: &str = r#"/* Minimal reset */
*, *::before, *::after {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
}

body {
    font-family: system-ui, -apple-system, sans-serif;
    line-height: 1.6;
    max-width: 800px;
    margin: 0 auto;
    padding: 2rem;
    color: #333;
    background: #fff;
}

nav ul {
    display: flex;
    list-style: none;
    gap: 1rem;
    padding: 1rem 0;
    border-bottom: 1px solid #eee;
    margin-bottom: 2rem;
}

nav a {
    text-decoration: none;
    color: #0066cc;
}

nav a:hover {
    text-decoration: underline;
}

h1, h2, h3 {
    margin-top: 1.5rem;
    margin-bottom: 0.5rem;
}

p {
    margin-bottom: 1rem;
}

code {
    background: #f4f4f4;
    padding: 0.15rem 0.4rem;
    border-radius: 3px;
    font-size: 0.9em;
}

footer {
    margin-top: 3rem;
    padding-top: 1rem;
    border-top: 1px solid #eee;
    color: #666;
    font-size: 0.9rem;
}
"#;

const GITIGNORE: &str = r#"/dist/
/.eigen_cache/
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_init_creates_project() {
        let tmp = TempDir::new().unwrap();
        let project_name = tmp.path().join("my-site");
        let name = project_name.to_string_lossy().to_string();

        init_project(&name).unwrap();

        // Check all expected files exist.
        assert!(project_name.join("site.toml").exists());
        assert!(project_name.join("templates/_base.html").exists());
        assert!(project_name.join("templates/_partials/nav.html").exists());
        assert!(project_name.join("templates/index.html").exists());
        assert!(project_name.join("templates/about.html").exists());
        assert!(project_name.join("_data/nav.yaml").exists());
        assert!(project_name.join("static/css/style.css").exists());
        assert!(project_name.join(".gitignore").exists());
    }

    #[test]
    fn test_init_site_toml_is_valid() {
        let tmp = TempDir::new().unwrap();
        let project_name = tmp.path().join("test-site");
        let name = project_name.to_string_lossy().to_string();

        init_project(&name).unwrap();

        let toml_content = fs::read_to_string(project_name.join("site.toml")).unwrap();
        let config: toml::Value = toml::from_str(&toml_content).unwrap();
        assert!(config.get("site").is_some());
        assert!(config["site"].get("name").is_some());
        assert!(config["site"].get("base_url").is_some());
    }

    #[test]
    fn test_init_fails_if_exists() {
        let tmp = TempDir::new().unwrap();
        let project_name = tmp.path().join("existing");
        fs::create_dir_all(&project_name).unwrap();
        let name = project_name.to_string_lossy().to_string();

        let result = init_project(&name);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn test_init_gitignore_content() {
        let tmp = TempDir::new().unwrap();
        let project_name = tmp.path().join("gi-site");
        let name = project_name.to_string_lossy().to_string();

        init_project(&name).unwrap();

        let gi = fs::read_to_string(project_name.join(".gitignore")).unwrap();
        assert!(gi.contains("dist"));
    }

    #[test]
    fn test_init_scaffold_is_buildable() {
        let tmp = TempDir::new().unwrap();
        let project_name = tmp.path().join("buildable-site");
        let name = project_name.to_string_lossy().to_string();

        init_project(&name).unwrap();

        // The scaffolded site should be buildable.
        let result = crate::build::build(&project_name, true);
        assert!(result.is_ok(), "Scaffolded site failed to build: {:#}", result.unwrap_err());

        // Check output files exist.
        assert!(project_name.join("dist/index.html").exists());
        assert!(project_name.join("dist/about.html").exists());
        assert!(project_name.join("dist/css/style.css").exists());
        assert!(project_name.join("dist/sitemap.xml").exists());

        // Check fragment generation.
        assert!(project_name.join("dist/_fragments/index.html").exists());
        assert!(project_name.join("dist/_fragments/about.html").exists());
    }
}
