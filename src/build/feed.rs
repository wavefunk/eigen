//! Atom feed generation.
//!
//! After all pages are rendered, generates Atom 1.0 XML feeds for
//! collections configured in `[feed.*]` tables in site.toml.

use eyre::{Result, WrapErr};
use std::path::Path;

use crate::config::{FeedConfig, SiteConfig, SiteMeta};
use crate::data::DataFetcher;
use crate::frontmatter::DataQuery;
use crate::plugins::registry::PluginRegistry;

use super::sitemap::escape_xml;

/// Generate Atom feeds for all configured `[feed.*]` definitions.
///
/// Returns the number of feeds written.
pub fn generate_feeds(
    dist_dir: &Path,
    config: &SiteConfig,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
    build_time: &str,
) -> Result<usize> {
    let mut count = 0;

    for (name, feed_config) in &config.feed {
        generate_feed(
            dist_dir,
            name,
            feed_config,
            &config.site,
            fetcher,
            plugin_registry,
            build_time,
        )
        .wrap_err_with(|| format!("Failed to generate feed '{}'", name))?;
        count += 1;
    }

    Ok(count)
}

/// Build a `DataQuery` from a feed config's inline data source fields.
fn build_feed_query(config: &FeedConfig) -> DataQuery {
    DataQuery {
        file: config.file.clone(),
        source: config.source.clone(),
        path: config.query_path.clone(),
        root: config.root.clone(),
        sort: config.sort.clone(),
        limit: Some(config.limit),
        filter: None,
        method: Default::default(),
        body: None,
    }
}

/// Generate a single Atom feed and write it to disk.
fn generate_feed(
    dist_dir: &Path,
    feed_name: &str,
    feed_config: &FeedConfig,
    site: &SiteMeta,
    fetcher: &mut DataFetcher,
    plugin_registry: Option<&PluginRegistry>,
    build_time: &str,
) -> Result<()> {
    let query = build_feed_query(feed_config);

    // Fetch collection data.
    let data = fetcher
        .fetch(&query, plugin_registry)
        .wrap_err("Failed to fetch feed data")?;

    let items = match data {
        serde_json::Value::Array(arr) => arr,
        _ => Vec::new(),
    };

    let base_url = site.base_url.trim_end_matches('/');
    let feed_title = feed_config.title.as_deref().unwrap_or(&site.name);
    let feed_author = feed_config
        .author
        .as_deref()
        .or(site.schema.author.as_deref());
    let self_url = format!("{}/{}", base_url, &feed_config.path);

    // Determine feed-level <updated>: latest entry date or build_time.
    let feed_updated = items
        .first()
        .and_then(|item| item.get(&feed_config.date_field))
        .map(|v| format_atom_date(v, build_time))
        .unwrap_or_else(|| build_time.to_string());

    // Build XML.
    let mut xml = String::with_capacity(4096);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<feed xmlns=\"http://www.w3.org/2005/Atom\">\n");

    write_tag(&mut xml, "  ", "title", feed_title);
    write_attr(&mut xml, "  ", "link", &[("href", base_url)]);
    write_attr(
        &mut xml,
        "  ",
        "link",
        &[("href", &self_url), ("rel", "self")],
    );
    write_tag(&mut xml, "  ", "id", &self_url);
    write_tag(&mut xml, "  ", "updated", &feed_updated);

    if let Some(author_name) = feed_author {
        xml.push_str("  <author>\n");
        write_tag(&mut xml, "    ", "name", author_name);
        xml.push_str("  </author>\n");
    }

    // Render entries.
    for item in &items {
        if let Some(entry_xml) = render_entry(item, feed_config, base_url, build_time) {
            xml.push_str(&entry_xml);
        }
    }

    xml.push_str("</feed>\n");

    // Write to disk.
    let feed_path = dist_dir.join(&feed_config.path);
    if let Some(parent) = feed_path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("Failed to create dir for feed '{}'", feed_name))?;
    }
    std::fs::write(&feed_path, &xml)
        .wrap_err_with(|| format!("Failed to write {}", feed_path.display()))?;

    tracing::debug!(
        "  Feed '{}': {} entries -> {}",
        feed_name,
        items.len(),
        feed_config.path,
    );

    Ok(())
}

/// Write `<tag>escaped_content</tag>` with the given indent.
fn write_tag(buf: &mut String, indent: &str, tag: &str, content: &str) {
    buf.push_str(indent);
    buf.push('<');
    buf.push_str(tag);
    buf.push('>');
    buf.push_str(&escape_xml(content));
    buf.push_str("</");
    buf.push_str(tag);
    buf.push_str(">\n");
}

/// Write a self-closing tag with attributes, e.g. `<link href="..." rel="..."/>`.
fn write_attr(buf: &mut String, indent: &str, tag: &str, attrs: &[(&str, &str)]) {
    buf.push_str(indent);
    buf.push('<');
    buf.push_str(tag);
    for (key, val) in attrs {
        buf.push(' ');
        buf.push_str(key);
        buf.push_str("=\"");
        buf.push_str(&escape_xml(val));
        buf.push('"');
    }
    buf.push_str("/>\n");
}

/// Render a single `<entry>` element. Returns `None` if required
/// fields (title, slug) are missing on the item.
fn render_entry(
    item: &serde_json::Value,
    config: &FeedConfig,
    base_url: &str,
    build_time: &str,
) -> Option<String> {
    // Extract title.
    let title = match item.get(&config.title_field) {
        Some(serde_json::Value::String(s)) => s.as_str(),
        Some(v) => {
            tracing::warn!(
                "Feed entry has non-string title field '{}': {:?}, skipping.",
                config.title_field,
                v,
            );
            return None;
        }
        None => {
            tracing::warn!(
                "Feed entry missing title field '{}', skipping.",
                config.title_field,
            );
            return None;
        }
    };

    // Extract slug.
    let slug = match item.get(&config.slug_field) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => {
            tracing::warn!(
                "Feed entry missing or invalid slug field '{}', skipping.",
                config.slug_field,
            );
            return None;
        }
    };

    // Build entry URL.
    let entry_url = match &config.link_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_matches('/');
            format!("{}/{}/{}.html", base_url, prefix, slug)
        }
        None => format!("{}/{}.html", base_url, slug),
    };

    // Extract date.
    let updated = item
        .get(&config.date_field)
        .map(|v| format_atom_date(v, build_time))
        .unwrap_or_else(|| build_time.to_string());

    let mut entry = String::new();
    entry.push_str("  <entry>\n");
    write_tag(&mut entry, "    ", "title", title);
    write_attr(&mut entry, "    ", "link", &[("href", &entry_url)]);
    write_tag(&mut entry, "    ", "id", &entry_url);
    write_tag(&mut entry, "    ", "updated", &updated);

    // Optional summary.
    if let Some(ref summary_field) = config.summary_field
        && let Some(serde_json::Value::String(summary)) = item.get(summary_field)
    {
        write_tag(&mut entry, "    ", "summary", summary);
    }

    entry.push_str("  </entry>\n");

    Some(entry)
}

/// Parse a date value and format it as RFC 3339 for Atom.
///
/// Accepts:
/// - String: ISO 8601 / RFC 3339 formats
/// - Falls back to `build_time` if unparseable or not a string.
fn format_atom_date(value: &serde_json::Value, build_time: &str) -> String {
    let s = match value {
        serde_json::Value::String(s) => s.as_str(),
        _ => return build_time.to_string(),
    };

    // Try full RFC 3339 / DateTime parse first.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    }

    // Try date-only (YYYY-MM-DD) -> midnight UTC.
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid");
        return chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    }

    // Fallback.
    build_time.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn test_feed_config() -> FeedConfig {
        FeedConfig {
            file: Some("posts.json".into()),
            source: None,
            query_path: None,
            root: None,
            sort: None,
            title: None,
            path: "feed.xml".into(),
            author: None,
            limit: 50,
            title_field: "title".into(),
            date_field: "date".into(),
            summary_field: None,
            slug_field: "slug".into(),
            link_prefix: None,
        }
    }

    fn test_site_meta() -> SiteMeta {
        use crate::config::{SiteSchemaConfig, SiteSeoConfig};
        SiteMeta {
            name: "Test Site".into(),
            base_url: "https://example.com".into(),
            seo: SiteSeoConfig::default(),
            schema: SiteSchemaConfig::default(),
        }
    }

    // --- build_feed_query ---

    #[test]
    fn test_build_feed_query_from_file() {
        let mut config = test_feed_config();
        config.file = Some("posts.json".into());
        config.sort = Some("-date".into());
        config.limit = 20;

        let query = build_feed_query(&config);
        assert_eq!(query.file.as_deref(), Some("posts.json"));
        assert!(query.source.is_none());
        assert!(query.path.is_none());
        assert_eq!(query.sort.as_deref(), Some("-date"));
        assert_eq!(query.limit, Some(20));
        assert!(query.filter.is_none());
    }

    #[test]
    fn test_build_feed_query_from_source() {
        let mut config = test_feed_config();
        config.file = None;
        config.source = Some("cms".into());
        config.query_path = Some("/posts".into());
        config.root = Some("data.posts".into());

        let query = build_feed_query(&config);
        assert!(query.file.is_none());
        assert_eq!(query.source.as_deref(), Some("cms"));
        assert_eq!(query.path.as_deref(), Some("/posts"));
        assert_eq!(query.root.as_deref(), Some("data.posts"));
    }

    // --- format_atom_date ---

    #[test]
    fn test_format_atom_date_rfc3339() {
        let val = json!("2026-03-17T12:00:00Z");
        let result = format_atom_date(&val, "2026-01-01T00:00:00Z");
        assert_eq!(result, "2026-03-17T12:00:00Z");
    }

    #[test]
    fn test_format_atom_date_date_only() {
        let val = json!("2026-03-17");
        let result = format_atom_date(&val, "2026-01-01T00:00:00Z");
        assert_eq!(result, "2026-03-17T00:00:00Z");
    }

    #[test]
    fn test_format_atom_date_with_offset() {
        let val = json!("2026-03-17T12:00:00+05:30");
        let result = format_atom_date(&val, "2026-01-01T00:00:00Z");
        // Should be valid RFC 3339 output.
        assert!(result.contains("2026-03-17"));
    }

    #[test]
    fn test_format_atom_date_fallback_non_string() {
        let val = json!(12345);
        let result = format_atom_date(&val, "2026-01-01T00:00:00Z");
        assert_eq!(result, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn test_format_atom_date_fallback_bad_string() {
        let val = json!("not a date");
        let result = format_atom_date(&val, "2026-01-01T00:00:00Z");
        assert_eq!(result, "2026-01-01T00:00:00Z");
    }

    // --- render_entry ---

    #[test]
    fn test_render_entry_basic() {
        let config = test_feed_config();
        let item = json!({
            "title": "Hello World",
            "slug": "hello-world",
            "date": "2026-03-17T12:00:00Z"
        });

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        assert!(entry.is_some());
        let xml = entry.unwrap();
        assert!(xml.contains("<title>Hello World</title>"));
        assert!(xml.contains("https://example.com/hello-world.html"));
        assert!(xml.contains("<updated>2026-03-17T12:00:00Z</updated>"));
    }

    #[test]
    fn test_render_entry_with_prefix() {
        let mut config = test_feed_config();
        config.link_prefix = Some("blog".into());
        let item = json!({"title": "Post", "slug": "my-post", "date": "2026-03-17"});

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        let xml = entry.unwrap();
        assert!(xml.contains("https://example.com/blog/my-post.html"));
    }

    #[test]
    fn test_render_entry_with_summary() {
        let mut config = test_feed_config();
        config.summary_field = Some("excerpt".into());
        let item = json!({
            "title": "Post",
            "slug": "post",
            "date": "2026-03-17",
            "excerpt": "A short summary."
        });

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        let xml = entry.unwrap();
        assert!(xml.contains("<summary>A short summary.</summary>"));
    }

    #[test]
    fn test_render_entry_missing_title() {
        let config = test_feed_config();
        let item = json!({"slug": "no-title", "date": "2026-03-17"});

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        assert!(entry.is_none());
    }

    #[test]
    fn test_render_entry_missing_slug() {
        let config = test_feed_config();
        let item = json!({"title": "No Slug", "date": "2026-03-17"});

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        assert!(entry.is_none());
    }

    #[test]
    fn test_render_entry_xml_escaping() {
        let config = test_feed_config();
        let item = json!({
            "title": "Tom & Jerry <Show>",
            "slug": "tom-jerry",
            "date": "2026-03-17"
        });

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        let xml = entry.unwrap();
        assert!(xml.contains("Tom &amp; Jerry &lt;Show&gt;"));
    }

    #[test]
    fn test_render_entry_custom_fields() {
        let mut config = test_feed_config();
        config.title_field = "name".into();
        config.date_field = "publishedAt".into();
        config.slug_field = "id".into();
        let item = json!({
            "name": "Release v1.0",
            "id": "v1-0",
            "publishedAt": "2026-03-17T10:00:00Z"
        });

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        let xml = entry.unwrap();
        assert!(xml.contains("<title>Release v1.0</title>"));
        assert!(xml.contains("https://example.com/v1-0.html"));
        assert!(xml.contains("<updated>2026-03-17T10:00:00Z</updated>"));
    }

    #[test]
    fn test_render_entry_numeric_slug() {
        let config = test_feed_config();
        let item = json!({"title": "Post", "slug": 42, "date": "2026-03-17"});

        let entry =
            render_entry(&item, &config, "https://example.com", "2026-01-01T00:00:00Z");
        let xml = entry.unwrap();
        assert!(xml.contains("https://example.com/42.html"));
    }

    // --- generate_feed (integration with filesystem) ---

    #[test]
    fn test_generate_feed_basic() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dist = root.join("dist");
        fs::create_dir_all(&dist).unwrap();

        // Write test data file.
        let data_dir = root.join("_data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(
            data_dir.join("posts.json"),
            r#"[
                {"title": "First Post", "slug": "first-post", "date": "2026-03-17T12:00:00Z"},
                {"title": "Second Post", "slug": "second-post", "date": "2026-03-16T12:00:00Z"}
            ]"#,
        )
        .unwrap();

        let mut fetcher = DataFetcher::new(&std::collections::HashMap::new(), root);
        let site = test_site_meta();
        let config = test_feed_config();

        generate_feed(
            &dist,
            "blog",
            &config,
            &site,
            &mut fetcher,
            None,
            "2026-03-18T00:00:00Z",
        )
        .unwrap();

        let xml = fs::read_to_string(dist.join("feed.xml")).unwrap();
        assert!(xml.contains("<?xml version=\"1.0\""));
        assert!(xml.contains("<feed xmlns=\"http://www.w3.org/2005/Atom\">"));
        assert!(xml.contains("<title>Test Site</title>"));
        assert!(xml.contains("https://example.com/feed.xml"));
        assert!(xml.contains("<entry>"));
        assert!(xml.contains("First Post"));
        assert!(xml.contains("Second Post"));
        assert!(xml.contains("</feed>"));
    }

    #[test]
    fn test_generate_feed_empty_collection() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dist = root.join("dist");
        fs::create_dir_all(&dist).unwrap();

        let data_dir = root.join("_data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("posts.json"), "[]").unwrap();

        let mut fetcher = DataFetcher::new(&std::collections::HashMap::new(), root);
        let site = test_site_meta();
        let config = test_feed_config();

        generate_feed(
            &dist,
            "blog",
            &config,
            &site,
            &mut fetcher,
            None,
            "2026-03-18T00:00:00Z",
        )
        .unwrap();

        let xml = fs::read_to_string(dist.join("feed.xml")).unwrap();
        assert!(xml.contains("<feed"));
        assert!(xml.contains("</feed>"));
        assert!(!xml.contains("<entry>"));
    }

    #[test]
    fn test_generate_feed_limit() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dist = root.join("dist");
        fs::create_dir_all(&dist).unwrap();

        let data_dir = root.join("_data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(
            data_dir.join("posts.json"),
            r#"[
                {"title": "A", "slug": "a", "date": "2026-03-17"},
                {"title": "B", "slug": "b", "date": "2026-03-16"},
                {"title": "C", "slug": "c", "date": "2026-03-15"}
            ]"#,
        )
        .unwrap();

        let mut fetcher = DataFetcher::new(&std::collections::HashMap::new(), root);
        let site = test_site_meta();
        let mut config = test_feed_config();
        config.limit = 2;

        generate_feed(
            &dist,
            "blog",
            &config,
            &site,
            &mut fetcher,
            None,
            "2026-03-18T00:00:00Z",
        )
        .unwrap();

        let xml = fs::read_to_string(dist.join("feed.xml")).unwrap();
        // Only 2 entries because limit is enforced via DataQuery.limit.
        let entry_count = xml.matches("<entry>").count();
        assert_eq!(entry_count, 2);
    }

    #[test]
    fn test_generate_feed_nested_path() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dist = root.join("dist");
        fs::create_dir_all(&dist).unwrap();

        let data_dir = root.join("_data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(
            data_dir.join("posts.json"),
            r#"[{"title": "P", "slug": "p", "date": "2026-03-17"}]"#,
        )
        .unwrap();

        let mut fetcher = DataFetcher::new(&std::collections::HashMap::new(), root);
        let site = test_site_meta();
        let mut config = test_feed_config();
        config.path = "blog/feed.xml".into();

        generate_feed(
            &dist,
            "blog",
            &config,
            &site,
            &mut fetcher,
            None,
            "2026-03-18T00:00:00Z",
        )
        .unwrap();

        assert!(dist.join("blog/feed.xml").exists());
    }

    #[test]
    fn test_generate_feed_with_author() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dist = root.join("dist");
        fs::create_dir_all(&dist).unwrap();

        let data_dir = root.join("_data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("posts.json"), "[]").unwrap();

        let mut fetcher = DataFetcher::new(&std::collections::HashMap::new(), root);
        let site = test_site_meta();
        let mut config = test_feed_config();
        config.author = Some("Jane Doe".into());

        generate_feed(
            &dist,
            "blog",
            &config,
            &site,
            &mut fetcher,
            None,
            "2026-03-18T00:00:00Z",
        )
        .unwrap();

        let xml = fs::read_to_string(dist.join("feed.xml")).unwrap();
        assert!(xml.contains("<author>"));
        assert!(xml.contains("<name>Jane Doe</name>"));
    }

    #[test]
    fn test_generate_feed_author_from_schema() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let dist = root.join("dist");
        fs::create_dir_all(&dist).unwrap();

        let data_dir = root.join("_data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("posts.json"), "[]").unwrap();

        let mut fetcher = DataFetcher::new(&std::collections::HashMap::new(), root);
        let mut site = test_site_meta();
        site.schema.author = Some("Schema Author".into());
        let config = test_feed_config(); // no author set

        generate_feed(
            &dist,
            "blog",
            &config,
            &site,
            &mut fetcher,
            None,
            "2026-03-18T00:00:00Z",
        )
        .unwrap();

        let xml = fs::read_to_string(dist.join("feed.xml")).unwrap();
        assert!(xml.contains("<name>Schema Author</name>"));
    }
}
