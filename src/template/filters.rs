//! Step 4.3: Custom minijinja filters.
//!
//! Registers the following filters on the Environment:
//!
//! - `markdown`  — render Markdown to HTML via pulldown-cmark
//! - `date`      — parse a date string and reformat with a given pattern
//! - `slugify`   — convert a string to a URL-friendly slug
//! - `absolute`  — prepend the site's `base_url`
//! - `truncate`  — truncate at a word boundary, appending "..."
//! - `sort_by`   — sort an array of objects by a key
//! - `group_by`  — group an array of objects by a key → map of key → array
//! - `json`      — serialize a value to a JSON string

use minijinja::{Environment, Error, ErrorKind, Value};
use std::collections::BTreeMap;

use crate::config::SiteConfig;

/// Register all custom filters on the given environment.
pub fn register_filters(env: &mut Environment<'_>, config: &SiteConfig) {
    env.add_filter("markdown", filter_markdown);
    env.add_filter("date", filter_date);
    env.add_filter("slugify", filter_slugify);

    // `absolute` needs the base_url, so we capture it.
    let base_url = config.site.base_url.clone();
    env.add_filter("absolute", move |path: &str| -> String {
        let base = base_url.trim_end_matches('/');
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };
        format!("{}{}", base, path)
    });

    env.add_filter("truncate", filter_truncate);
    env.add_filter("sort_by", filter_sort_by);
    env.add_filter("group_by", filter_group_by);
    env.add_filter("json", filter_json);
}

/// Render a Markdown string to HTML.
///
/// Usage: `{{ content | markdown }}`
fn filter_markdown(value: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(value, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

/// Parse a date string and reformat it.
///
/// Usage: `{{ post.date | date("%B %d, %Y") }}`
///
/// Tries several common input formats (ISO 8601, RFC 2822, etc.).
fn filter_date(value: &str, format: &str) -> Result<String, Error> {
    use chrono::NaiveDate;

    // Try parsing as a full datetime first, then date-only.
    let formats = [
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M:%S%:z",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d",
        "%d/%m/%Y",
        "%m/%d/%Y",
    ];

    for fmt in &formats {
        // Try NaiveDateTime first.
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(value, fmt) {
            return Ok(dt.format(format).to_string());
        }
        // Try NaiveDate.
        if let Ok(d) = NaiveDate::parse_from_str(value, fmt) {
            return Ok(d.format(format).to_string());
        }
    }

    // Try chrono's DateTime<FixedOffset> for RFC 3339/2822.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(dt.format(format).to_string());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(value) {
        return Ok(dt.format(format).to_string());
    }

    Err(Error::new(
        ErrorKind::InvalidOperation,
        format!("Cannot parse '{}' as a date", value),
    ))
}

/// Convert a string to a URL-friendly slug.
///
/// Usage: `{{ title | slugify }}`
fn filter_slugify(value: &str) -> String {
    slug::slugify(value)
}

/// Truncate a string at a word boundary, appending "..." if truncated.
///
/// Usage: `{{ text | truncate(100) }}`
///
/// The `length` parameter is the maximum number of characters to keep (before
/// the "..." suffix). If cutting at `length` lands inside a word, we back up
/// to the previous word boundary (space). If there's no space at all, we just
/// cut at `length`.
fn filter_truncate(value: &str, length: usize) -> String {
    if value.len() <= length {
        return value.to_string();
    }

    let end = length.min(value.len());

    // If we're exactly at a word boundary (next char is space, or end is a
    // space), just take everything up to `end`.
    let at_boundary = value.as_bytes().get(end).map(|&b| b == b' ').unwrap_or(true);
    let break_pos = if at_boundary {
        end
    } else {
        // We're inside a word — find the last space before `end`.
        value[..end].rfind(' ').unwrap_or(end)
    };

    let trimmed = value[..break_pos].trim_end();
    format!("{}...", trimmed)
}

/// Sort an array of objects by a given key.
///
/// Usage: `{{ items | sort_by("name") }}` or `{{ items | sort_by("-date") }}`
fn filter_sort_by(value: Value, key: &str) -> Result<Value, Error> {
    let mut items: Vec<Value> = value
        .try_iter()
        .map_err(|e| Error::new(ErrorKind::InvalidOperation, format!("sort_by requires a sequence: {}", e)))?
        .collect();

    let (field, descending) = if let Some(stripped) = key.strip_prefix('-') {
        (stripped, true)
    } else {
        (key, false)
    };

    items.sort_by(|a, b| {
        let va = a.get_attr(field).ok();
        let vb = b.get_attr(field).ok();
        let cmp = compare_minijinja_values(va.as_ref(), vb.as_ref());
        if descending { cmp.reverse() } else { cmp }
    });

    Ok(Value::from(items))
}

/// Group an array of objects by a key, returning a map of key → array.
///
/// Usage: `{% set groups = items | group_by("category") %}` then iterate
/// with `{% for k in groups | list %}` and access `groups[k]`.
fn filter_group_by(value: Value, key: &str) -> Result<Value, Error> {
    let items: Vec<Value> = value
        .try_iter()
        .map_err(|e| Error::new(ErrorKind::InvalidOperation, format!("group_by requires a sequence: {}", e)))?
        .collect();

    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();

    for item in items {
        let group_key: String = item
            .get_attr(key)
            .ok()
            .map(|v| {
                if let Some(s) = v.as_str() {
                    s.to_string()
                } else {
                    v.to_string()
                }
            })
            .unwrap_or_default();
        groups.entry(group_key).or_default().push(item);
    }

    // Build a map using from_iter with string keys.
    Ok(Value::from_iter(
        groups
            .into_iter()
            .map(|(k, v)| (k, Value::from(v)))
    ))
}

/// Serialize a value to a JSON string.
///
/// Usage: `{{ data | json }}` or in `<script>var data = {{ data | json }};</script>`
fn filter_json(value: Value) -> Result<String, Error> {
    let json_value: serde_json::Value = serde_json::to_value(&value).map_err(|e| {
        Error::new(ErrorKind::InvalidOperation, format!("Cannot serialize to JSON: {}", e))
    })?;
    serde_json::to_string_pretty(&json_value).map_err(|e| {
        Error::new(ErrorKind::InvalidOperation, format!("JSON serialization error: {}", e))
    })
}

/// Compare two optional minijinja Values for sorting.
fn compare_minijinja_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => {
            // Try string comparison as the common case.
            let sa = va.to_string();
            let sb = vb.to_string();

            // Try numeric comparison first.
            if let (Ok(na), Ok(nb)) = (sa.parse::<f64>(), sb.parse::<f64>()) {
                return na.partial_cmp(&nb).unwrap_or(Ordering::Equal);
            }

            sa.cmp(&sb)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minijinja::context;

    // Helper to create a minimal env for testing filters.
    fn test_env() -> Environment<'static> {
        let config = SiteConfig {
            site: crate::config::SiteMeta {
                name: "Test".into(),
                base_url: "https://example.com".into(),
            },
            build: crate::config::BuildConfig::default(),
            assets: Default::default(),
            sources: std::collections::HashMap::new(),
            plugins: std::collections::HashMap::new(),
        };

        let mut env = Environment::new();
        register_filters(&mut env, &config);
        env
    }

    // --- markdown filter ---

    #[test]
    fn test_filter_markdown_basic() {
        let result = filter_markdown("# Hello\n\nWorld");
        assert!(result.contains("<h1>Hello</h1>"));
        assert!(result.contains("<p>World</p>"));
    }

    #[test]
    fn test_filter_markdown_inline() {
        let result = filter_markdown("**bold** and *italic*");
        assert!(result.contains("<strong>bold</strong>"));
        assert!(result.contains("<em>italic</em>"));
    }

    #[test]
    fn test_filter_markdown_in_template() {
        let mut env = test_env();
        env.add_template("test", "{{ content | markdown }}").unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! { content => "# Title" }).unwrap();
        assert!(result.contains("<h1>Title</h1>"));
    }

    // --- date filter ---

    #[test]
    fn test_filter_date_iso() {
        let result = filter_date("2024-01-15", "%B %d, %Y").unwrap();
        assert_eq!(result, "January 15, 2024");
    }

    #[test]
    fn test_filter_date_datetime() {
        let result = filter_date("2024-01-15 14:30:00", "%Y/%m/%d").unwrap();
        assert_eq!(result, "2024/01/15");
    }

    #[test]
    fn test_filter_date_invalid() {
        let result = filter_date("not-a-date", "%Y-%m-%d");
        assert!(result.is_err());
    }

    // --- slugify filter ---

    #[test]
    fn test_filter_slugify_basic() {
        assert_eq!(filter_slugify("Hello World"), "hello-world");
    }

    #[test]
    fn test_filter_slugify_special_chars() {
        assert_eq!(filter_slugify("Hello, World! #1"), "hello-world-1");
    }

    #[test]
    fn test_filter_slugify_unicode() {
        let result = filter_slugify("Über Cool");
        assert!(result.contains("cool"));
    }

    // --- truncate filter ---

    #[test]
    fn test_filter_truncate_no_truncation() {
        assert_eq!(filter_truncate("Short", 100), "Short");
    }

    #[test]
    fn test_filter_truncate_at_word_boundary() {
        assert_eq!(
            filter_truncate("Hello beautiful world", 15),
            "Hello beautiful..."
        );
    }

    #[test]
    fn test_filter_truncate_no_space() {
        assert_eq!(
            filter_truncate("Superlongwordwithoutspaces", 10),
            "Superlongw..."
        );
    }

    // --- absolute filter ---

    #[test]
    fn test_filter_absolute_in_template() {
        let mut env = test_env();
        env.add_template("test", "{{ path | absolute }}").unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! { path => "/about.html" }).unwrap();
        assert_eq!(result, "https://example.com/about.html");
    }

    #[test]
    fn test_filter_absolute_no_leading_slash() {
        let mut env = test_env();
        env.add_template("test", "{{ path | absolute }}").unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! { path => "about.html" }).unwrap();
        assert_eq!(result, "https://example.com/about.html");
    }

    // --- sort_by filter ---

    #[test]
    fn test_filter_sort_by_in_template() {
        let mut env = test_env();
        env.add_template("test", "{% for i in items | sort_by('name') %}{{ i.name }} {% endfor %}").unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {
            items => vec![
                context!{ name => "Charlie" },
                context!{ name => "Alice" },
                context!{ name => "Bob" },
            ]
        }).unwrap();
        assert_eq!(result.trim(), "Alice Bob Charlie");
    }

    #[test]
    fn test_filter_sort_by_descending() {
        let mut env = test_env();
        env.add_template("test", "{% for i in items | sort_by('-name') %}{{ i.name }} {% endfor %}").unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {
            items => vec![
                context!{ name => "Alice" },
                context!{ name => "Charlie" },
                context!{ name => "Bob" },
            ]
        }).unwrap();
        assert_eq!(result.trim(), "Charlie Bob Alice");
    }

    // --- group_by filter ---

    #[test]
    fn test_filter_group_by_in_template() {
        let mut env = test_env();
        env.add_template("test",
            "{% set groups = items | group_by('cat') %}{% for k in groups | list %}{{ k }}:{{ groups[k] | length }} {% endfor %}"
        ).unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {
            items => vec![
                context!{ name => "A", cat => "x" },
                context!{ name => "B", cat => "y" },
                context!{ name => "C", cat => "x" },
            ]
        }).unwrap();
        // BTreeMap orders alphabetically: x, y
        assert!(result.contains("x:2"));
        assert!(result.contains("y:1"));
    }

    // --- json filter ---

    #[test]
    fn test_filter_json_in_template() {
        let mut env = test_env();
        env.add_template("test", "{{ data | json }}").unwrap();
        let tmpl = env.get_template("test").unwrap();
        let result = tmpl.render(context! {
            data => context!{ key => "value", num => 42 }
        }).unwrap();
        // Should be valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["key"], "value");
        assert_eq!(parsed["num"], 42);
    }
}
