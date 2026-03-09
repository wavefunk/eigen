//! Plugin registry: manages the set of active plugins and dispatches
//! hook calls through the pipeline.

use eyre::{Result, WrapErr};
use std::path::Path;

use super::Plugin;

/// Holds all active plugins and dispatches lifecycle hooks.
#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        tracing::debug!("Registered plugin: {}", plugin.name());
        self.plugins.push(plugin);
    }

    /// Call `on_config_loaded` for all plugins.
    pub fn on_config_loaded(
        &mut self,
        plugin_configs: &std::collections::HashMap<String, toml::Value>,
        project_root: &Path,
    ) -> Result<()> {
        for plugin in &mut self.plugins {
            let config = plugin_configs.get(plugin.name());
            plugin
                .on_config_loaded(config, project_root)
                .wrap_err_with(|| {
                    format!("Plugin '{}' failed on_config_loaded", plugin.name())
                })?;
        }
        Ok(())
    }

    /// Run data through all plugins' `transform_data` hooks (chained).
    ///
    /// Each plugin receives the output of the previous one.
    pub fn transform_data(
        &self,
        mut value: serde_json::Value,
        source_name: Option<&str>,
        query_path: Option<&str>,
    ) -> Result<serde_json::Value> {
        for plugin in &self.plugins {
            value = plugin
                .transform_data(value, source_name, query_path)
                .wrap_err_with(|| {
                    format!(
                        "Plugin '{}' failed transform_data for source {:?}",
                        plugin.name(),
                        source_name,
                    )
                })?;
        }
        Ok(value)
    }

    /// Let all plugins register template extensions.
    pub fn register_template_extensions(
        &self,
        env: &mut minijinja::Environment<'_>,
    ) -> Result<()> {
        for plugin in &self.plugins {
            plugin
                .register_template_extensions(env)
                .wrap_err_with(|| {
                    format!(
                        "Plugin '{}' failed register_template_extensions",
                        plugin.name()
                    )
                })?;
        }
        Ok(())
    }

    /// Run rendered HTML through all plugins' `post_render_html` hooks (chained).
    pub fn post_render_html(
        &self,
        mut html: String,
        output_path: &str,
        dist_dir: &Path,
    ) -> Result<String> {
        for plugin in &self.plugins {
            html = plugin
                .post_render_html(html, output_path, dist_dir)
                .wrap_err_with(|| {
                    format!(
                        "Plugin '{}' failed post_render_html for '{}'",
                        plugin.name(),
                        output_path,
                    )
                })?;
        }
        Ok(html)
    }

    /// Call `post_build` for all plugins.
    pub fn post_build(
        &self,
        dist_dir: &Path,
        project_root: &Path,
    ) -> Result<()> {
        for plugin in &self.plugins {
            plugin
                .post_build(dist_dir, project_root)
                .wrap_err_with(|| {
                    format!("Plugin '{}' failed post_build", plugin.name())
                })?;
        }
        Ok(())
    }

    /// Return the names of all registered plugins.
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.name()).collect()
    }

    /// Whether any plugins are registered.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

/// Build the default plugin registry based on which `[plugins.*]` sections
/// appear in the site config.
///
/// Only plugins whose names appear in the config are activated.  Unknown
/// plugin names produce a warning (not an error) so the system is forward-
/// compatible.
pub fn build_registry(
    plugin_configs: &std::collections::HashMap<String, toml::Value>,
    project_root: &Path,
) -> Result<PluginRegistry> {
    let mut registry = PluginRegistry::new();

    for name in plugin_configs.keys() {
        match name.as_str() {
            "strapi" => {
                registry.register(Box::new(super::strapi::StrapiPlugin::new()));
            }
            unknown => {
                tracing::warn!(
                    "Unknown plugin '{}' in [plugins.{}]. Ignoring.",
                    unknown,
                    unknown,
                );
            }
        }
    }

    // Initialize all plugins with their config.
    registry.on_config_loaded(plugin_configs, project_root)?;

    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // Test plugin helpers
    // -----------------------------------------------------------------------

    /// A test plugin that marks every array item with `plugin_touched = true`
    /// and wraps rendered HTML in a comment.
    #[derive(Debug)]
    struct TestPlugin {
        prefix: String,
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            "test"
        }

        fn transform_data(
            &self,
            mut value: serde_json::Value,
            _source: Option<&str>,
            _path: Option<&str>,
        ) -> Result<serde_json::Value> {
            if let serde_json::Value::Array(ref mut arr) = value {
                for item in arr.iter_mut() {
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert(
                            "plugin_touched".to_string(),
                            serde_json::Value::Bool(true),
                        );
                    }
                }
            }
            Ok(value)
        }

        fn post_render_html(
            &self,
            html: String,
            _path: &str,
            _dist: &Path,
        ) -> Result<String> {
            Ok(format!("<!-- {} -->\n{}", self.prefix, html))
        }
    }

    /// A plugin that uppercases all string values in JSON arrays.
    #[derive(Debug)]
    struct UppercasePlugin;

    impl Plugin for UppercasePlugin {
        fn name(&self) -> &str {
            "uppercase"
        }

        fn transform_data(
            &self,
            value: serde_json::Value,
            _source: Option<&str>,
            _path: Option<&str>,
        ) -> Result<serde_json::Value> {
            match value {
                serde_json::Value::Array(arr) => {
                    Ok(serde_json::Value::Array(
                        arr.into_iter()
                            .map(|v| match v {
                                serde_json::Value::Object(mut map) => {
                                    for val in map.values_mut() {
                                        if let serde_json::Value::String(s) = val {
                                            *s = s.to_uppercase();
                                        }
                                    }
                                    serde_json::Value::Object(map)
                                }
                                other => other,
                            })
                            .collect(),
                    ))
                }
                other => Ok(other),
            }
        }

        fn post_render_html(
            &self,
            html: String,
            _path: &str,
            _dist: &Path,
        ) -> Result<String> {
            Ok(html.to_uppercase())
        }
    }

    /// A plugin that always fails transform_data.
    #[derive(Debug)]
    struct FailingPlugin;

    impl Plugin for FailingPlugin {
        fn name(&self) -> &str {
            "failing"
        }

        fn transform_data(
            &self,
            _value: serde_json::Value,
            _source: Option<&str>,
            _path: Option<&str>,
        ) -> Result<serde_json::Value> {
            Err(eyre::eyre!("intentional failure"))
        }

        fn post_render_html(
            &self,
            _html: String,
            _path: &str,
            _dist: &Path,
        ) -> Result<String> {
            Err(eyre::eyre!("intentional html failure"))
        }

        fn post_build(
            &self,
            _dist_dir: &Path,
            _project_root: &Path,
        ) -> Result<()> {
            Err(eyre::eyre!("intentional post_build failure"))
        }
    }

    /// A plugin that only processes data from a specific source.
    #[derive(Debug)]
    struct SourceFilterPlugin {
        target_source: String,
    }

    impl Plugin for SourceFilterPlugin {
        fn name(&self) -> &str {
            "source_filter"
        }

        fn transform_data(
            &self,
            mut value: serde_json::Value,
            source_name: Option<&str>,
            _path: Option<&str>,
        ) -> Result<serde_json::Value> {
            if source_name == Some(self.target_source.as_str()) {
                if let serde_json::Value::Array(ref mut arr) = value {
                    for item in arr.iter_mut() {
                        if let Some(obj) = item.as_object_mut() {
                            obj.insert("processed_by_filter".into(), json!(true));
                        }
                    }
                }
            }
            Ok(value)
        }
    }

    // -----------------------------------------------------------------------
    // Registry basic tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_new_is_empty() {
        let registry = PluginRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.plugin_names().len(), 0);
    }

    #[test]
    fn test_registry_default_is_empty() {
        let registry = PluginRegistry::default();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_registry_register_single() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin { prefix: "A".into() }));
        assert!(!registry.is_empty());
        assert_eq!(registry.plugin_names(), vec!["test"]);
    }

    #[test]
    fn test_registry_register_multiple() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin { prefix: "A".into() }));
        registry.register(Box::new(UppercasePlugin));
        assert_eq!(registry.plugin_names(), vec!["test", "uppercase"]);
    }

    // -----------------------------------------------------------------------
    // transform_data tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_transform_data_single_plugin() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin { prefix: "A".into() }));

        let data = json!([{"id": 1}, {"id": 2}]);
        let result = registry.transform_data(data, Some("test"), None).unwrap();

        let arr = result.as_array().unwrap();
        assert!(arr[0]["plugin_touched"].as_bool().unwrap());
        assert!(arr[1]["plugin_touched"].as_bool().unwrap());
    }

    #[test]
    fn test_transform_data_chained_two_plugins() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin { prefix: "A".into() }));
        registry.register(Box::new(UppercasePlugin));

        let data = json!([{"id": 1, "name": "hello"}]);
        let result = registry.transform_data(data, Some("test"), None).unwrap();

        let arr = result.as_array().unwrap();
        // TestPlugin adds plugin_touched
        // UppercasePlugin uppercases string values
        assert_eq!(arr[0]["name"], "HELLO");
        // plugin_touched is a bool, not a string, so it stays as-is
        assert!(arr[0]["plugin_touched"].as_bool().unwrap());
    }

    #[test]
    fn test_transform_data_empty_registry_passthrough() {
        let registry = PluginRegistry::new();

        let data = json!({"key": "value"});
        let result = registry.transform_data(data.clone(), None, None).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_transform_data_with_array_passthrough() {
        let registry = PluginRegistry::new();
        let data = json!([1, 2, 3]);
        let result = registry.transform_data(data.clone(), None, None).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_transform_data_source_filtering() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(SourceFilterPlugin {
            target_source: "strapi".into(),
        }));

        // Data from the target source — should be processed.
        let data = json!([{"id": 1}]);
        let result = registry.transform_data(data, Some("strapi"), None).unwrap();
        assert!(result[0]["processed_by_filter"].as_bool().unwrap());

        // Data from a different source — should pass through.
        let data2 = json!([{"id": 2}]);
        let result2 = registry.transform_data(data2, Some("other"), None).unwrap();
        assert!(result2[0].get("processed_by_filter").is_none());

        // No source — should pass through.
        let data3 = json!([{"id": 3}]);
        let result3 = registry.transform_data(data3, None, None).unwrap();
        assert!(result3[0].get("processed_by_filter").is_none());
    }

    #[test]
    fn test_transform_data_failing_plugin_propagates_error() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(FailingPlugin));

        let data = json!([{"id": 1}]);
        let result = registry.transform_data(data, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failing"));
    }

    // -----------------------------------------------------------------------
    // post_render_html tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_post_render_html_single_plugin() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin { prefix: "PLUGIN".into() }));

        let html = "<h1>Hello</h1>".to_string();
        let result = registry
            .post_render_html(html, "index.html", Path::new("dist"))
            .unwrap();

        assert!(result.starts_with("<!-- PLUGIN -->"));
        assert!(result.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn test_post_render_html_chained_two_plugins() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin { prefix: "first".into() }));
        registry.register(Box::new(UppercasePlugin));

        let html = "<p>hello</p>".to_string();
        let result = registry
            .post_render_html(html, "test.html", Path::new("dist"))
            .unwrap();

        // TestPlugin wraps in <!-- first -->\n<p>hello</p>
        // UppercasePlugin uppercases everything
        assert!(result.contains("<!-- FIRST -->"));
        assert!(result.contains("<P>HELLO</P>"));
    }

    #[test]
    fn test_post_render_html_empty_registry_passthrough() {
        let registry = PluginRegistry::new();
        let html = "<p>Test</p>".to_string();
        let result = registry
            .post_render_html(html.clone(), "test.html", Path::new("dist"))
            .unwrap();
        assert_eq!(result, html);
    }

    #[test]
    fn test_post_render_html_failing_plugin_propagates_error() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(FailingPlugin));

        let result = registry.post_render_html(
            "<p>hi</p>".into(),
            "test.html",
            Path::new("dist"),
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failing"));
    }

    // -----------------------------------------------------------------------
    // post_build tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_post_build_empty_registry_ok() {
        let registry = PluginRegistry::new();
        let tmp = tempfile::tempdir().unwrap();
        registry.post_build(tmp.path(), tmp.path()).unwrap();
    }

    #[test]
    fn test_post_build_failing_plugin_propagates_error() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(FailingPlugin));

        let tmp = tempfile::tempdir().unwrap();
        let result = registry.post_build(tmp.path(), tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failing"));
    }

    // -----------------------------------------------------------------------
    // on_config_loaded tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_on_config_loaded_with_no_config_for_plugin() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin { prefix: "X".into() }));

        let configs = HashMap::new(); // empty — no config for "test"
        let tmp = tempfile::tempdir().unwrap();
        // Should succeed — default no-op implementation handles missing config.
        registry.on_config_loaded(&configs, tmp.path()).unwrap();
    }

    // -----------------------------------------------------------------------
    // register_template_extensions tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_register_template_extensions_empty_registry() {
        let registry = PluginRegistry::new();
        let mut env = minijinja::Environment::new();
        registry.register_template_extensions(&mut env).unwrap();
        // No error with empty registry.
    }

    // -----------------------------------------------------------------------
    // build_registry tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_registry_empty_config() {
        let configs = HashMap::new();
        let tmp = tempfile::tempdir().unwrap();
        let registry = build_registry(&configs, tmp.path()).unwrap();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_build_registry_strapi_plugin() {
        let mut configs = HashMap::new();
        configs.insert("strapi".into(), toml::Value::Table(Default::default()));

        let tmp = tempfile::tempdir().unwrap();
        let registry = build_registry(&configs, tmp.path()).unwrap();
        assert_eq!(registry.plugin_names(), vec!["strapi"]);
    }

    #[test]
    fn test_build_registry_multiple_plugins() {
        let mut configs = HashMap::new();
        configs.insert("strapi".into(), toml::Value::Table(Default::default()));

        let tmp = tempfile::tempdir().unwrap();
        let registry = build_registry(&configs, tmp.path()).unwrap();
        assert_eq!(registry.plugin_names().len(), 1);
        let names = registry.plugin_names();
        assert!(names.contains(&"strapi"));
    }

    #[test]
    fn test_build_registry_unknown_plugin_ignored() {
        let mut configs = HashMap::new();
        configs.insert("nonexistent".into(), toml::Value::Table(Default::default()));

        let tmp = tempfile::tempdir().unwrap();
        let registry = build_registry(&configs, tmp.path()).unwrap();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_build_registry_mixed_known_and_unknown() {
        let mut configs = HashMap::new();
        configs.insert("strapi".into(), toml::Value::Table(Default::default()));
        configs.insert("nonexistent".into(), toml::Value::Table(Default::default()));

        let tmp = tempfile::tempdir().unwrap();
        let registry = build_registry(&configs, tmp.path()).unwrap();
        // Only the known plugin is registered.
        assert_eq!(registry.plugin_names(), vec!["strapi"]);
    }

    #[test]
    fn test_build_registry_with_strapi_config() {
        let toml_str = r#"
            sources = ["cms"]
            media_base_url = "http://localhost:1337"
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();

        let mut configs = HashMap::new();
        configs.insert("strapi".into(), value);

        let tmp = tempfile::tempdir().unwrap();
        let registry = build_registry(&configs, tmp.path()).unwrap();
        assert_eq!(registry.plugin_names(), vec!["strapi"]);

        // Verify the plugin was configured by testing its transform behavior.
        let data = json!([{"id": 1, "attributes": {"title": "Test"}}]);
        let result = registry.transform_data(data, Some("cms"), None).unwrap();
        // Should be flattened by strapi plugin since source="cms" matches config.
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["title"], "Test");
        assert!(arr[0].get("attributes").is_none());
    }
}
