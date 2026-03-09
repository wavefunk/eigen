//! Step 4.1: Minijinja environment setup.
//!
//! Creates a `minijinja::Environment` with:
//! - Template loader that reads from `templates/` directory
//! - All page templates registered with frontmatter stripped
//! - Layout files (`_base.html`, `_*.html`) and partials (`_partials/*`) loaded
//! - Strict undefined behavior (error on undefined var)
//! - Custom filters and functions registered

use eyre::{Result, WrapErr};
use minijinja::Environment;
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

use crate::config::SiteConfig;
use crate::discovery::PageDef;
use crate::plugins::registry::PluginRegistry;

use super::filters;
use super::functions;
use super::preprocessing;

/// Set up and return a fully configured `minijinja::Environment`.
///
/// - Registers all discovered page templates (frontmatter-stripped bodies).
/// - Loads all `_`-prefixed layout/partial templates from `templates/`.
/// - Applies fragment marker preprocessing if fragments are enabled.
/// - Registers custom filters and functions.
/// - Lets plugins register their own template extensions.
pub fn setup_environment(
    project_root: &Path,
    config: &SiteConfig,
    pages: &[PageDef],
    plugin_registry: Option<&PluginRegistry>,
) -> Result<Environment<'static>> {
    let mut env = Environment::new();

    // Strict mode: error on undefined variables.
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);

    // Disable auto-escaping. In a static site generator, all template content
    // is author-controlled (not user input), so HTML escaping is unnecessary
    // and causes issues with URLs containing `/` being escaped to `&#x2f;`.
    // Authors can use the `|e` filter explicitly if needed.
    env.set_auto_escape_callback(|_name: &str| {
        minijinja::AutoEscape::None
    });

    let templates_dir = project_root.join("templates");

    // Build a map of template name → processed body.
    // We collect all templates into owned Strings, then use set_loader to serve them.
    let mut template_map: HashMap<String, String> = HashMap::new();

    // 1. Load all _-prefixed files (layouts, partials) — these were skipped by
    //    discovery but are needed by the template engine for {% extends %} and
    //    {% include %}.
    let underscored = collect_underscore_templates(&templates_dir)?;
    for (name, body) in underscored {
        let processed = if config.build.fragments {
            preprocessing::inject_fragment_markers(&body)
        } else {
            body
        };
        template_map.insert(name, processed);
    }

    // 2. Register discovered page templates (frontmatter already stripped).
    for page in pages {
        let name = page.template_path.to_string_lossy().to_string();
        let body = if config.build.fragments {
            preprocessing::inject_fragment_markers(&page.template_body)
        } else {
            page.template_body.clone()
        };
        template_map.insert(name, body);
    }

    // 3. Set up the template loader using the collected map.
    env.set_loader(move |name: &str| -> Result<Option<String>, minijinja::Error> {
        Ok(template_map.get(name).cloned())
    });

    // 4. Register custom filters.
    filters::register_filters(&mut env, config);

    // 5. Register custom functions.
    functions::register_functions(&mut env, config);

    // 6. Let plugins register their own filters/functions/globals.
    if let Some(registry) = plugin_registry {
        registry.register_template_extensions(&mut env)?;
    }

    Ok(env)
}

/// Walk the `templates/` directory and collect all `_`-prefixed files
/// (layouts and partials).
///
/// Returns a vec of `(template_name, body)` tuples. The template name is the
/// path relative to `templates/`, e.g. `"_base.html"`, `"_partials/nav.html"`.
fn collect_underscore_templates(templates_dir: &Path) -> Result<Vec<(String, String)>> {
    let mut results = Vec::new();

    if !templates_dir.is_dir() {
        return Ok(results);
    }

    for entry in WalkDir::new(templates_dir).into_iter() {
        let entry = entry.wrap_err("Failed to read entry while collecting layout/partial templates")?;

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Only .html files.
        match path.extension().and_then(|e| e.to_str()) {
            Some("html") => {}
            _ => continue,
        }

        let rel_path = path
            .strip_prefix(templates_dir)
            .wrap_err("Template path is not inside templates/ directory")?;

        // We want files that are _-prefixed themselves OR inside a _-prefixed dir.
        // The discovery phase already skipped these. We identify them by checking
        // if any component of the relative path starts with `_`.
        let is_underscore = rel_path
            .components()
            .any(|c| {
                c.as_os_str()
                    .to_str()
                    .map(|s| s.starts_with('_'))
                    .unwrap_or(false)
            });

        if !is_underscore {
            continue;
        }

        let body = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("Failed to read template {}", path.display()))?;

        // For layout/partial files, we don't strip frontmatter — they shouldn't
        // have any. Just use the raw content.
        let name = rel_path.to_string_lossy().to_string();
        // Normalize path separators to forward slashes for template names.
        let name = name.replace('\\', "/");

        results.push((name, body));
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BuildConfig, SiteMeta};
    use crate::discovery::{PageDef, PageType};
    use crate::frontmatter::Frontmatter;
    use std::path::PathBuf;
    use std::fs;
    use tempfile::TempDir;

    fn test_config() -> SiteConfig {
        SiteConfig {
            site: SiteMeta {
                name: "Test".into(),
                base_url: "https://test.com".into(),
            },
            build: BuildConfig {
                fragments: false,
                ..Default::default()
            },
            assets: Default::default(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
        }
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_collect_underscore_templates() {
        let tmp = TempDir::new().unwrap();
        let tpl = tmp.path().join("templates");
        fs::create_dir_all(&tpl).unwrap();

        write(tmp.path(), "templates/_base.html", "<!DOCTYPE html>{% block body %}{% endblock %}");
        write(tmp.path(), "templates/_partials/nav.html", "<nav>nav</nav>");
        write(tmp.path(), "templates/index.html", "<h1>Home</h1>");

        let results = collect_underscore_templates(&tpl).unwrap();
        assert_eq!(results.len(), 2);

        let names: Vec<&str> = results.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"_base.html"));
        assert!(names.contains(&"_partials/nav.html"));
    }

    #[test]
    fn test_setup_environment_basic() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write(root, "templates/_base.html",
              "<!DOCTYPE html><html><body>{% block content %}{% endblock %}</body></html>");
        write(root, "templates/_partials/nav.html", "<nav>nav</nav>");

        let config = test_config();

        let pages = vec![PageDef {
            template_path: PathBuf::from("index.html"),
            page_type: PageType::Static,
            output_dir: PathBuf::from(""),
            frontmatter: Frontmatter::default(),
            template_body: r#"{% extends "_base.html" %}{% block content %}<h1>Hi</h1>{% endblock %}"#.into(),
        }];

        let env = setup_environment(root, &config, &pages, None).unwrap();

        // Verify template is registered and can render.
        let tmpl = env.get_template("index.html").unwrap();
        let ctx = minijinja::context! {};
        let result = tmpl.render(ctx).unwrap();
        assert!(result.contains("<h1>Hi</h1>"));
        assert!(result.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn test_strict_undefined_behavior() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // We need a templates dir to exist (even if empty for underscore collection).
        fs::create_dir_all(root.join("templates")).unwrap();

        let config = test_config();

        let pages = vec![PageDef {
            template_path: PathBuf::from("test.html"),
            page_type: PageType::Static,
            output_dir: PathBuf::from(""),
            frontmatter: Frontmatter::default(),
            template_body: "<h1>{{ undefined_var }}</h1>".into(),
        }];

        let env = setup_environment(root, &config, &pages, None).unwrap();
        let tmpl = env.get_template("test.html").unwrap();
        let result = tmpl.render(minijinja::context! {});
        assert!(result.is_err());
    }

    #[test]
    fn test_setup_environment_with_plugin_registry() {
        use crate::plugins::Plugin;
        use crate::plugins::registry::PluginRegistry;

        #[derive(Debug)]
        struct TestPlugin;

        impl Plugin for TestPlugin {
            fn name(&self) -> &str { "test_ext" }

            fn register_template_extensions(
                &self,
                env: &mut Environment<'_>,
            ) -> eyre::Result<()> {
                env.add_function("plugin_hello", || -> String {
                    "hello from plugin".to_string()
                });
                Ok(())
            }
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("templates")).unwrap();

        let config = test_config();

        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin));

        let pages = vec![PageDef {
            template_path: PathBuf::from("test.html"),
            page_type: PageType::Static,
            output_dir: PathBuf::from(""),
            frontmatter: Frontmatter::default(),
            template_body: "{{ plugin_hello() }}".into(),
        }];

        let env = setup_environment(root, &config, &pages, Some(&registry)).unwrap();
        let tmpl = env.get_template("test.html").unwrap();
        let result = tmpl.render(minijinja::context! {}).unwrap();
        assert_eq!(result, "hello from plugin");
    }

    #[test]
    fn test_setup_environment_without_plugin_registry() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("templates")).unwrap();

        let config = test_config();

        let pages = vec![PageDef {
            template_path: PathBuf::from("test.html"),
            page_type: PageType::Static,
            output_dir: PathBuf::from(""),
            frontmatter: Frontmatter::default(),
            template_body: "<h1>No plugins</h1>".into(),
        }];

        // Passing None for plugin_registry should work fine.
        let env = setup_environment(root, &config, &pages, None).unwrap();
        let tmpl = env.get_template("test.html").unwrap();
        let result = tmpl.render(minijinja::context! {}).unwrap();
        assert_eq!(result, "<h1>No plugins</h1>");
    }
}
