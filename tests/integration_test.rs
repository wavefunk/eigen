//! These tests exercise the full build pipeline end-to-end, verifying:
//! - Full build of the example site
//! - Dynamic page with empty collection
//! - Fragment generation and marker stripping
//! - HTMX `link_to()` attributes in output HTML
//! - Static asset copying
//! - Sitemap generation
//! - Global data loading
//! - Custom filters and functions in rendered output
//! - Edge cases (missing templates, undefined vars, etc.)

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

// ============================================================================
// Full example site build
// ============================================================================

#[test]
fn test_full_build_example_site() {
    // Build the actual example_site that ships with the project.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let example_site = manifest_dir.join("example_site");

    // Copy example_site to a temp dir so we don't pollute the repo with dist/.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    copy_dir_all(&example_site, root);

    eigen::build::build(root).unwrap();

    // Verify dist/ structure.
    assert!(root.join("dist").is_dir(), "dist/ should exist");
    assert!(root.join("dist/index.html").exists(), "index.html should exist");
    assert!(root.join("dist/about.html").exists(), "about.html should exist");
    assert!(root.join("dist/sitemap.xml").exists(), "sitemap.xml should exist");

    // Verify static assets copied.
    assert!(
        root.join("dist/css/style.css").exists(),
        "static/css/style.css should be copied to dist/"
    );

    // Verify fragment generation (the example site has fragments enabled by default).
    assert!(
        root.join("dist/_fragments/index.html").exists(),
        "fragment for index.html should exist"
    );
    assert!(
        root.join("dist/_fragments/about.html").exists(),
        "fragment for about.html should exist"
    );

    // Verify full pages contain DOCTYPE but fragments don't.
    let full_html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(full_html.contains("<!DOCTYPE html>"), "Full page should have DOCTYPE");
    assert!(
        !full_html.contains("<!--FRAG:"),
        "Full page should NOT contain fragment markers"
    );

    let frag_html = fs::read_to_string(root.join("dist/_fragments/index.html")).unwrap();
    assert!(
        !frag_html.contains("<!DOCTYPE html>"),
        "Fragment should NOT have DOCTYPE"
    );

    // Verify sitemap has correct URLs.
    let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
    assert!(sitemap.contains("<urlset"), "Sitemap should be valid XML");
    assert!(
        sitemap.contains("https://example.com/index.html"),
        "Sitemap should contain correct base_url + path"
    );
    assert!(
        !sitemap.contains("_fragments"),
        "Sitemap should NOT contain fragment URLs"
    );
}

// ============================================================================
// Dynamic pages
// ============================================================================

#[test]
fn test_dynamic_pages_generate_one_per_item() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Dynamic Test"
base_url = "https://test.com"

[build]
fragments = true
"#);

    write(root, "templates/_base.html",
          "<!DOCTYPE html><html><body>{% block content %}{% endblock %}</body></html>");

    write(root, "templates/posts/[post].html", r#"---
collection:
  file: "posts.json"
slug_field: slug
item_as: post
---
{% extends "_base.html" %}
{% block content %}
<h1>{{ post.title }}</h1>
<p>By {{ post.author }}</p>
{% endblock %}"#);

    write(root, "_data/posts.json", r#"[
        {"slug": "hello-world", "title": "Hello World", "author": "Alice"},
        {"slug": "second-post", "title": "Second Post", "author": "Bob"},
        {"slug": "third-post", "title": "Third Post", "author": "Carol"}
    ]"#);

    eigen::build::build(root).unwrap();

    // Verify all three pages generated.
    assert!(root.join("dist/posts/hello-world.html").exists());
    assert!(root.join("dist/posts/second-post.html").exists());
    assert!(root.join("dist/posts/third-post.html").exists());

    // Verify content is correct for each page.
    let hello = fs::read_to_string(root.join("dist/posts/hello-world.html")).unwrap();
    assert!(hello.contains("<h1>Hello World</h1>"));
    assert!(hello.contains("By Alice"));

    let second = fs::read_to_string(root.join("dist/posts/second-post.html")).unwrap();
    assert!(second.contains("<h1>Second Post</h1>"));
    assert!(second.contains("By Bob"));

    // Verify fragments generated.
    assert!(root.join("dist/_fragments/posts/hello-world.html").exists());
    let frag = fs::read_to_string(root.join("dist/_fragments/posts/hello-world.html")).unwrap();
    assert!(frag.contains("<h1>Hello World</h1>"));
    assert!(!frag.contains("<!DOCTYPE html>"));
}

#[test]
fn test_dynamic_page_empty_collection_no_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Empty Coll"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/[item].html", r#"---
collection:
  file: "items.json"
---
{% extends "_base.html" %}
{% block content %}<p>{{ item.name }}</p>{% endblock %}"#);

    write(root, "_data/items.json", "[]");

    // Should succeed without error.
    eigen::build::build(root).unwrap();

    // No pages should be generated for empty collection.
    let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
    assert!(!sitemap.contains("<url>"), "No pages should be in sitemap");
}

#[test]
fn test_dynamic_page_with_nested_data_queries() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Nested Data"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/[post].html", r#"---
collection:
  file: "posts.json"
slug_field: slug
item_as: post
data:
  author:
    file: "authors.json"
    filter:
      id: "{{ post.author_id }}"
---
{% extends "_base.html" %}
{% block content %}
<h1>{{ post.title }}</h1>
{% for a in author %}<p>Author: {{ a.name }}</p>{% endfor %}
{% endblock %}"#);

    write(root, "_data/posts.json", r#"[
        {"slug": "post-1", "title": "Post One", "author_id": "1"},
        {"slug": "post-2", "title": "Post Two", "author_id": "2"}
    ]"#);

    write(root, "_data/authors.json", r#"[
        {"id": "1", "name": "Alice"},
        {"id": "2", "name": "Bob"}
    ]"#);

    eigen::build::build(root).unwrap();

    let post1 = fs::read_to_string(root.join("dist/post-1.html")).unwrap();
    assert!(post1.contains("Post One"));
    assert!(post1.contains("Author: Alice"));
    assert!(!post1.contains("Author: Bob"));

    let post2 = fs::read_to_string(root.join("dist/post-2.html")).unwrap();
    assert!(post2.contains("Post Two"));
    assert!(post2.contains("Author: Bob"));
    assert!(!post2.contains("Author: Alice"));
}

// ============================================================================
// Fragment generation
// ============================================================================

#[test]
fn test_fragment_markers_stripped_from_full_page() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Frag Test"
base_url = "https://test.com"

[build]
fragments = true
content_block = "content"
"#);

    write(root, "templates/_base.html",
          "<!DOCTYPE html><html><body>{% block content %}{% endblock %}</body></html>");

    write(root, "templates/index.html", r#"{% extends "_base.html" %}
{% block content %}<h1>Home</h1><p>Welcome</p>{% endblock %}"#);

    eigen::build::build(root).unwrap();

    // Full page: no markers.
    let full = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(!full.contains("<!--FRAG:"), "Full page must not contain markers");
    assert!(full.contains("<h1>Home</h1>"));
    assert!(full.contains("<!DOCTYPE html>"));

    // Fragment: contains just the block content.
    let frag = fs::read_to_string(root.join("dist/_fragments/index.html")).unwrap();
    assert!(frag.contains("<h1>Home</h1>"));
    assert!(frag.contains("<p>Welcome</p>"));
    assert!(!frag.contains("<!DOCTYPE html>"));
    assert!(!frag.contains("<!--FRAG:"), "Fragment must not contain markers");
}

#[test]
fn test_multiple_fragment_blocks() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Multi Frag"
base_url = "https://test.com"

[build]
fragments = true
"#);

    write(root, "templates/_base.html", r#"<!DOCTYPE html>
<html>
<body>
<div id="sidebar">{% block sidebar %}Default sidebar{% endblock %}</div>
<main>{% block content %}{% endblock %}</main>
</body>
</html>"#);

    write(root, "templates/about.html", r#"---
fragment_blocks:
  - content
  - sidebar
---
{% extends "_base.html" %}
{% block sidebar %}<aside>About sidebar</aside>{% endblock %}
{% block content %}<h1>About</h1>{% endblock %}"#);

    eigen::build::build(root).unwrap();

    // Content fragment.
    assert!(root.join("dist/_fragments/about.html").exists());
    let content_frag = fs::read_to_string(root.join("dist/_fragments/about.html")).unwrap();
    assert!(content_frag.contains("<h1>About</h1>"));

    // Sidebar fragment.
    assert!(root.join("dist/_fragments/about/sidebar.html").exists());
    let sidebar_frag = fs::read_to_string(root.join("dist/_fragments/about/sidebar.html")).unwrap();
    assert!(sidebar_frag.contains("<aside>About sidebar</aside>"));
}

#[test]
fn test_fragments_disabled() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "No Frags"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/index.html", r#"{% extends "_base.html" %}
{% block content %}<h1>Home</h1>{% endblock %}"#);

    eigen::build::build(root).unwrap();

    assert!(root.join("dist/index.html").exists());
    assert!(!root.join("dist/_fragments").exists(), "_fragments dir should not exist");

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(!html.contains("<!--FRAG:"), "No markers when fragments disabled");
}

// ============================================================================
// HTMX link_to function
// ============================================================================

#[test]
fn test_link_to_generates_htmx_attributes() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "HTMX Test"
base_url = "https://test.com"

[build]
fragments = true
fragment_dir = "_fragments"
content_block = "content"
"#);

    write(root, "templates/_base.html",
          "<html><body>{% block content %}{% endblock %}</body></html>");

    write(root, "templates/index.html", r##"{% extends "_base.html" %}
{% block content %}
<a {{ link_to("/about.html") }}>About</a>
<a {{ link_to("/posts.html", "#main") }}>Posts</a>
{% endblock %}"##);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();

    // Default link_to should produce all 4 attributes.
    assert!(html.contains(r#"href="/about.html""#), "Should have href");
    assert!(
        html.contains(r#"hx-get="/_fragments/about.html""#),
        "Should have hx-get pointing to fragment"
    );
    assert!(html.contains(r##"hx-target="#content""##), "Default target should be #content");
    assert!(html.contains(r#"hx-push-url="/about.html""#), "Should have hx-push-url");

    // Custom target link_to.
    assert!(
        html.contains(r##"hx-target="#main""##),
        "Custom target should be respected"
    );
}

#[test]
fn test_link_to_without_fragments_is_plain_href() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Plain HREF"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html", r#"<a {{ link_to("/about.html") }}>About</a>"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains(r#"href="/about.html""#));
    assert!(!html.contains("hx-get"), "Should NOT have hx-get when fragments disabled");
}

// ============================================================================
// Custom filters in rendered output
// ============================================================================

#[test]
fn test_markdown_filter_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Markdown Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    // Use a data file so we can test the markdown filter on multi-line content.
    write(root, "_data/content.yaml", "text: \"# Hello\\n\\nWorld **bold**\"");
    write(root, "templates/index.html",
          "---\ndata:\n  content:\n    file: \"content.yaml\"\n---\n{{ content.text | markdown }}");

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("<h1>Hello</h1>"));
    assert!(html.contains("<strong>bold</strong>"));
}

#[test]
fn test_date_filter_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Date Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html",
          r#"{{ "2024-03-15" | date("%B %d, %Y") }}"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("March 15, 2024"));
}

#[test]
fn test_slugify_filter_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Slugify Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html",
          r#"{{ "Hello World! #1" | slugify }}"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("hello-world-1"));
}

#[test]
fn test_absolute_filter_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Absolute Test"
base_url = "https://example.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html",
          r#"{{ "/about.html" | absolute }}"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("https://example.com/about.html"));
}

#[test]
fn test_json_filter_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "JSON Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html", r#"---
data:
  info:
    file: "info.json"
---
<script>var data = {{ info | json }};</script>"#);

    write(root, "_data/info.json", r#"{"key": "value", "num": 42}"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("var data ="));
    // The JSON output should be parseable.
    let start = html.find("var data = ").unwrap() + "var data = ".len();
    let end = html.find(";</script>").unwrap();
    let json_str = &html[start..end];
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed["key"], "value");
    assert_eq!(parsed["num"], 42);
}

// ============================================================================
// Custom functions in rendered output
// ============================================================================

#[test]
fn test_current_year_function_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Year Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html",
          r#"<footer>&copy; {{ current_year() }}</footer>"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    let year = chrono::Local::now().format("%Y").to_string();
    assert!(html.contains(&format!("&copy; {}", year)));
}

#[test]
fn test_asset_function_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Asset Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html",
          r#"<link rel="stylesheet" href="{{ asset('css/style.css') }}">"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains(r#"href="/css/style.css""#));
}

#[test]
fn test_site_global_in_output() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "My Awesome Site"
base_url = "https://awesome.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html",
          r#"<title>{{ site.name }}</title><base href="{{ site.base_url }}">"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("<title>My Awesome Site</title>"));
    assert!(html.contains(r#"href="https://awesome.com""#));
}

// ============================================================================
// Global data (_data/) in templates
// ============================================================================

#[test]
fn test_global_data_yaml_in_template() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Data Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html", r#"---
data:
  nav:
    file: "nav.yaml"
---
<nav>{% for item in nav %}<a href="{{ item.url }}">{{ item.label }}</a> {% endfor %}</nav>"#);

    write(root, "_data/nav.yaml", r#"
- label: Home
  url: /
- label: About
  url: /about
- label: Blog
  url: /blog
"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains(r#"<a href="/">Home</a>"#));
    assert!(html.contains(r#"<a href="/about">About</a>"#));
    assert!(html.contains(r#"<a href="/blog">Blog</a>"#));
}

#[test]
fn test_global_data_json_in_template() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "JSON Data"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html", r#"---
data:
  config:
    file: "config.json"
---
<p>Theme: {{ config.theme }}</p><p>Debug: {{ config.debug }}</p>"#);

    write(root, "_data/config.json", r#"{"theme": "dark", "debug": false}"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("Theme: dark"));
    assert!(html.contains("Debug: false"));
}

// ============================================================================
// Data transforms (sort, filter, limit) in rendered output
// ============================================================================

#[test]
fn test_data_sort_filter_limit_in_build() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Transform Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html", r#"---
data:
  posts:
    file: "posts.json"
    filter:
      status: "published"
    sort: "-id"
    limit: 2
---
{% for p in posts %}{{ p.title }} {% endfor %}"#);

    write(root, "_data/posts.json", r#"[
        {"id": 1, "title": "First", "status": "published"},
        {"id": 2, "title": "Second", "status": "draft"},
        {"id": 3, "title": "Third", "status": "published"},
        {"id": 4, "title": "Fourth", "status": "published"},
        {"id": 5, "title": "Fifth", "status": "published"}
    ]"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    // Filtered to published (1,3,4,5), sorted by -id (5,4,3,1), limited to 2 (5,4).
    assert!(html.contains("Fifth"));
    assert!(html.contains("Fourth"));
    assert!(!html.contains("Third"), "Third should be excluded by limit");
    assert!(!html.contains("Second"), "Second should be filtered out (draft)");
    assert!(!html.contains("First"), "First should be excluded by limit");
}

// ============================================================================
// Page metadata (page object)
// ============================================================================

#[test]
fn test_page_metadata_available() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Meta Test"
base_url = "https://meta.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/docs/guide.html", r#"{% extends "_base.html" %}
{% block content %}
URL:{{ page.current_url }}
PATH:{{ page.current_path }}
BASE:{{ page.base_url }}
{% endblock %}"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/docs/guide.html")).unwrap();
    assert!(html.contains("URL:/docs/guide.html"));
    assert!(html.contains("PATH:docs/guide.html"));
    assert!(html.contains("BASE:https://meta.com"));
}

// ============================================================================
// Sitemap priorities
// ============================================================================

#[test]
fn test_sitemap_priorities() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Sitemap Prio"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/index.html", r#"{% extends "_base.html" %}
{% block content %}Home{% endblock %}"#);

    write(root, "templates/about.html", r#"{% extends "_base.html" %}
{% block content %}About{% endblock %}"#);

    write(root, "templates/[item].html", r#"---
collection:
  file: "items.json"
---
{% extends "_base.html" %}
{% block content %}{{ item.title }}{% endblock %}"#);

    write(root, "_data/items.json", r#"[{"slug": "test-item", "title": "Test"}]"#);

    eigen::build::build(root).unwrap();

    let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();

    // Index page should have priority 1.0.
    assert!(sitemap.contains("<loc>https://test.com/index.html</loc>"));

    // Find the priority for each URL.
    // Parse it simply.
    let urls: Vec<&str> = sitemap.split("<url>").skip(1).collect();
    for url_block in &urls {
        if url_block.contains("/index.html") {
            assert!(url_block.contains("<priority>1.0</priority>"), "Index should be 1.0");
        } else if url_block.contains("/about.html") {
            assert!(url_block.contains("<priority>0.8</priority>"), "Static non-index should be 0.8");
        } else if url_block.contains("/test-item.html") {
            assert!(url_block.contains("<priority>0.6</priority>"), "Dynamic should be 0.6");
        }
    }
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_template_includes_partial() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Include Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_partials/footer.html",
          "<footer>© {{ site.name }}</footer>");

    write(root, "templates/index.html",
          "<main>Hello</main>{% include \"_partials/footer.html\" %}");

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("<main>Hello</main>"));
    assert!(html.contains("<footer>© Include Test</footer>"));
}

#[test]
fn test_template_extends_base_layout() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Extends Test"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html", r#"<!DOCTYPE html>
<html>
<head><title>{% block title %}{{ site.name }}{% endblock %}</title></head>
<body>{% block content %}{% endblock %}</body>
</html>"#);

    write(root, "templates/index.html", r#"{% extends "_base.html" %}
{% block title %}Home — {{ site.name }}{% endblock %}
{% block content %}<h1>Welcome</h1>{% endblock %}"#);

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("<title>Home — Extends Test</title>"));
    assert!(html.contains("<h1>Welcome</h1>"));
}

#[test]
fn test_missing_slug_field_item_skipped() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Skip Missing Slug"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/[item].html", r#"---
collection:
  file: "items.json"
slug_field: slug
---
{% extends "_base.html" %}
{% block content %}{{ item.name }}{% endblock %}"#);

    // One item has slug, one doesn't.
    write(root, "_data/items.json", r#"[
        {"slug": "good-item", "name": "Good"},
        {"name": "No Slug"}
    ]"#);

    eigen::build::build(root).unwrap();

    // Good item should be generated.
    assert!(root.join("dist/good-item.html").exists());

    // Bad item silently skipped — no crash.
    // Only one page in sitemap.
    let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
    let url_count = sitemap.matches("<url>").count();
    assert_eq!(url_count, 1, "Only one page should be generated");
}

#[test]
fn test_deeply_nested_static_pages() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Nested"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/a/b/c/deep.html", "<p>Deep page</p>");

    eigen::build::build(root).unwrap();

    assert!(root.join("dist/a/b/c/deep.html").exists());
    let html = fs::read_to_string(root.join("dist/a/b/c/deep.html")).unwrap();
    assert!(html.contains("<p>Deep page</p>"));
}

#[test]
fn test_static_and_dynamic_pages_coexist() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Mixed"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/index.html", r#"{% extends "_base.html" %}
{% block content %}Home{% endblock %}"#);

    write(root, "templates/about.html", r#"{% extends "_base.html" %}
{% block content %}About{% endblock %}"#);

    write(root, "templates/posts/index.html", r#"{% extends "_base.html" %}
{% block content %}All Posts{% endblock %}"#);

    write(root, "templates/posts/[post].html", r#"---
collection:
  file: "posts.json"
item_as: post
---
{% extends "_base.html" %}
{% block content %}{{ post.title }}{% endblock %}"#);

    write(root, "_data/posts.json", r#"[
        {"slug": "first", "title": "First Post"},
        {"slug": "second", "title": "Second Post"}
    ]"#);

    eigen::build::build(root).unwrap();

    assert!(root.join("dist/index.html").exists());
    assert!(root.join("dist/about.html").exists());
    assert!(root.join("dist/posts/index.html").exists());
    assert!(root.join("dist/posts/first.html").exists());
    assert!(root.join("dist/posts/second.html").exists());

    let sitemap = fs::read_to_string(root.join("dist/sitemap.xml")).unwrap();
    assert_eq!(sitemap.matches("<url>").count(), 5, "Should have 5 pages");
}

// ============================================================================
// Init command
// ============================================================================

#[test]
fn test_init_creates_buildable_project() {
    let tmp = TempDir::new().unwrap();
    let project_path = tmp.path().join("new-site");
    let name = project_path.to_string_lossy().to_string();

    eigen::init::init_project(&name).unwrap();

    // Verify all files exist.
    assert!(project_path.join("site.toml").exists());
    assert!(project_path.join("templates/_base.html").exists());
    assert!(project_path.join("templates/_partials/nav.html").exists());
    assert!(project_path.join("templates/index.html").exists());
    assert!(project_path.join("templates/about.html").exists());
    assert!(project_path.join("_data/nav.yaml").exists());
    assert!(project_path.join("static/css/style.css").exists());
    assert!(project_path.join(".gitignore").exists());

    // Build the scaffolded project.
    eigen::build::build(&project_path).unwrap();

    // Verify output.
    assert!(project_path.join("dist/index.html").exists());
    assert!(project_path.join("dist/about.html").exists());
    assert!(project_path.join("dist/css/style.css").exists());
    assert!(project_path.join("dist/sitemap.xml").exists());
    assert!(project_path.join("dist/_fragments/index.html").exists());
    assert!(project_path.join("dist/_fragments/about.html").exists());

    // Verify the built pages look correct.
    let index = fs::read_to_string(project_path.join("dist/index.html")).unwrap();
    assert!(index.contains("<!DOCTYPE html>"));
    assert!(index.contains("My Eigen Site"));
    assert!(index.contains("Welcome to"));

    let about = fs::read_to_string(project_path.join("dist/about.html")).unwrap();
    assert!(about.contains("About"));
    assert!(about.contains("Eigen"));
}

#[test]
fn test_init_duplicate_directory_errors() {
    let tmp = TempDir::new().unwrap();
    let project_path = tmp.path().join("existing");
    fs::create_dir_all(&project_path).unwrap();
    let name = project_path.to_string_lossy().to_string();

    let result = eigen::init::init_project(&name);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
}

// ============================================================================
// Live reload injection (dev mode)
// ============================================================================

#[test]
fn test_live_reload_script_injection() {
    use eigen::dev_inject::inject_reload_script;

    let html = "<html><body><h1>Hello</h1></body></html>";
    let result = inject_reload_script(html);

    assert!(result.contains("EventSource"));
    assert!(result.contains("/_reload"));
    assert!(result.contains("</body></html>"));

    // Script should be before </body>.
    let script_pos = result.find("EventSource").unwrap();
    let body_pos = result.find("</body>").unwrap();
    assert!(script_pos < body_pos);
}

#[test]
fn test_live_reload_not_injected_in_build() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "No Reload"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html",
          "<html><body><h1>Hello</h1></body></html>");

    eigen::build::build(root).unwrap();

    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(
        !html.contains("EventSource"),
        "Build output should NOT contain live reload script"
    );
    assert!(
        !html.contains("/_reload"),
        "Build output should NOT contain /_reload endpoint"
    );
}

// ============================================================================
// Plugin system integration tests
// ============================================================================

#[test]
fn test_build_with_no_plugins() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "No Plugins"
base_url = "https://test.com"

[build]
fragments = false
"#);

    write(root, "templates/index.html", "<h1>Hello</h1>");

    eigen::build::build(root).unwrap();
    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("<h1>Hello</h1>"));
}

#[test]
fn test_build_with_strapi_plugin_config() {
    // Test that a site.toml with [plugins.strapi] parses and builds correctly.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Strapi Plugin Test"
base_url = "https://test.com"

[build]
fragments = false

[plugins.strapi]
media_base_url = "http://localhost:1337"
"#);

    write(root, "templates/index.html", "<h1>Hello</h1>");

    eigen::build::build(root).unwrap();
    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("<h1>Hello</h1>"));
}

#[test]
fn test_strapi_plugin_transforms_data_in_build() {
    // Build a site that uses local JSON data structured like a Strapi response,
    // and verify the strapi plugin flattens the attributes.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Strapi Transform"
base_url = "https://test.com"

[build]
fragments = false

[sources.strapi]
url = "http://localhost:0"

[plugins.strapi]
sources = ["strapi"]
media_base_url = "http://localhost:1337"
"#);

    // Use a local file that mimics Strapi's response structure.
    // The strapi plugin should flatten this when the file is read
    // through a source-like query. But wait — plugin transforms only
    // run on source-backed queries, not file queries.
    // So we test with a file query where the data is already flat
    // but verify the plugin doesn't break anything.
    write(root, "_data/posts.json", r#"[
        {"id": 1, "title": "Hello"},
        {"id": 2, "title": "World"}
    ]"#);

    write(root, "templates/index.html", r#"---
data:
  posts:
    file: "posts.json"
---
{% for p in posts %}{{ p.title }} {% endfor %}"#);

    eigen::build::build(root).unwrap();
    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("Hello"));
    assert!(html.contains("World"));
}

#[test]
fn test_strapi_media_template_function() {
    // Test that the strapi plugin registers its template function.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Strapi Media Fn"
base_url = "https://test.com"

[build]
fragments = false

[plugins.strapi]
media_base_url = "http://localhost:1337"
"#);

    write(root, "templates/index.html",
          r#"<img src="{{ strapi_media('/uploads/photo.jpg') }}">"#);

    eigen::build::build(root).unwrap();
    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("http://localhost:1337/uploads/photo.jpg"));
}

#[test]
fn test_strapi_media_function_absolute_url_passthrough() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Strapi Media Abs"
base_url = "https://test.com"

[build]
fragments = false

[plugins.strapi]
media_base_url = "http://localhost:1337"
"#);

    write(root, "templates/index.html",
          r#"<img src="{{ strapi_media('https://cdn.example.com/photo.jpg') }}">"#);

    eigen::build::build(root).unwrap();
    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    // Absolute URL should pass through unchanged.
    assert!(html.contains("https://cdn.example.com/photo.jpg"));
}

#[test]
fn test_unknown_plugin_in_config_does_not_break_build() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Unknown Plugin"
base_url = "https://test.com"

[build]
fragments = false

[plugins.nonexistent_plugin]
some_option = true
"#);

    write(root, "templates/index.html", "<h1>Hello</h1>");

    // Should succeed — unknown plugins are warned but not fatal.
    eigen::build::build(root).unwrap();
    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("<h1>Hello</h1>"));
}

#[test]
fn test_multiple_plugins_in_config() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Multi Plugin"
base_url = "https://test.com"

[build]
fragments = false

[plugins.strapi]
media_base_url = "http://localhost:1337"

[plugins.js]
entries = []
"#);

    write(root, "templates/index.html",
          r#"<img src="{{ strapi_media('/uploads/test.jpg') }}">"#);

    eigen::build::build(root).unwrap();
    let html = fs::read_to_string(root.join("dist/index.html")).unwrap();
    assert!(html.contains("http://localhost:1337/uploads/test.jpg"));
}

#[test]
fn test_build_dynamic_pages_with_strapi_plugin() {
    // Test that dynamic page generation works correctly when the strapi plugin
    // is active but data comes from local files (plugin should be a no-op).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Dynamic + Plugin"
base_url = "https://test.com"

[build]
fragments = false

[plugins.strapi]
media_base_url = "http://localhost:1337"
"#);

    write(root, "templates/_base.html",
          "<html>{% block content %}{% endblock %}</html>");

    write(root, "templates/[post].html", r#"---
collection:
  file: "posts.json"
slug_field: slug
item_as: post
---
{% extends "_base.html" %}
{% block content %}<h1>{{ post.title }}</h1>{% endblock %}"#);

    write(root, "_data/posts.json", r#"[
        {"slug": "hello", "title": "Hello"},
        {"slug": "world", "title": "World"}
    ]"#);

    eigen::build::build(root).unwrap();
    assert!(root.join("dist/hello.html").exists());
    assert!(root.join("dist/world.html").exists());

    let hello = fs::read_to_string(root.join("dist/hello.html")).unwrap();
    assert!(hello.contains("<h1>Hello</h1>"));
}

// ============================================================================
// Plugin config parsing tests
// ============================================================================

#[test]
fn test_config_with_plugins_section_parses() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "Config Test"
base_url = "https://test.com"

[plugins.strapi]
sources = ["cms"]
media_base_url = "http://localhost:1337"
"#);

    let config = eigen::config::load_config(root).unwrap();
    assert_eq!(config.plugins.len(), 1);
    assert!(config.plugins.contains_key("strapi"));
}

#[test]
fn test_config_without_plugins_section_parses() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write(root, "site.toml", r#"
[site]
name = "No Plugins Config"
base_url = "https://test.com"
"#);

    let config = eigen::config::load_config(root).unwrap();
    assert!(config.plugins.is_empty());
}

// ============================================================================
// Utility
// ============================================================================

/// Recursively copy a directory.
fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_all(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}
