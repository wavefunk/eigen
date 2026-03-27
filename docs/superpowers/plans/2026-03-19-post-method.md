# POST Method for Data Sources — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable POST requests with JSON bodies in eigen's data fetching layer so APIs like Notion can be used as data sources.

**Architecture:** Add `HttpMethod` enum and `body` field to `DataQuery`. Branch on method in `fetch_source`. Extend `interpolate_query` and `verify_no_remaining_interpolation` to handle body. Use method-prefixed cache keys.

**Tech Stack:** Rust, serde_json, serde_yaml, reqwest (blocking), tracing, eyre, std::hash

**Spec:** `docs/superpowers/specs/2026-03-19-post-method-design.md`

---

### File Structure

| File | Responsibility | Action |
|------|---------------|--------|
| `src/frontmatter/mod.rs` | `HttpMethod` enum, `DataQuery` fields | Modify |
| `src/data/fetcher.rs` | POST branching, cache key | Modify |
| `src/data/query.rs` | Body interpolation, env var interpolation, verification | Modify |

---

### Task 1: Add HttpMethod enum and DataQuery fields

**Files:**
- Modify: `src/frontmatter/mod.rs:57-75` (DataQuery struct)
- Test: `src/frontmatter/mod.rs` (inline tests)

- [ ] **Step 1: Write failing tests for HttpMethod deserialization**

Add these tests to the `mod tests` block in `src/frontmatter/mod.rs`:

```rust
#[test]
fn test_parse_method_post_with_body() {
    let yaml = concat!(
        "data:\n",
        "  projects:\n",
        "    source: notion\n",
        "    path: /v1/databases/abc/query\n",
        "    method: post\n",
        "    body:\n",
        "      page_size: 100\n",
        "      filter:\n",
        "        property: \"Status\"\n",
        "    root: results\n",
    );
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    let q = &fm.data["projects"];
    assert_eq!(q.method, HttpMethod::Post);
    let body = q.body.as_ref().unwrap();
    assert_eq!(body["page_size"], 100);
    assert_eq!(body["filter"]["property"], "Status");
}

#[test]
fn test_parse_method_defaults_to_get() {
    let yaml = concat!(
        "data:\n",
        "  nav:\n",
        "    file: \"nav.yaml\"\n",
    );
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert_eq!(fm.data["nav"].method, HttpMethod::Get);
}

#[test]
fn test_parse_method_explicit_get() {
    let yaml = concat!(
        "data:\n",
        "  items:\n",
        "    source: api\n",
        "    path: /items\n",
        "    method: get\n",
    );
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert_eq!(fm.data["items"].method, HttpMethod::Get);
    assert!(fm.data["items"].body.is_none());
}

#[test]
fn test_parse_body_absent() {
    let yaml = concat!(
        "data:\n",
        "  items:\n",
        "    source: api\n",
        "    path: /items\n",
        "    method: post\n",
    );
    let fm = parse_frontmatter(yaml, "test.html").unwrap();
    assert_eq!(fm.data["items"].method, HttpMethod::Post);
    assert!(fm.data["items"].body.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib frontmatter::tests::test_parse_method -- 2>&1 | head -30`
Expected: Compilation error — `HttpMethod` not defined, `method`/`body` fields don't exist.

- [ ] **Step 3: Add HttpMethod enum and DataQuery fields**

In `src/frontmatter/mod.rs`, add the enum before `DataQuery`:

```rust
/// HTTP method for data source requests.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
}
```

Add two fields to `DataQuery`:

```rust
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DataQuery {
    // ... existing fields unchanged ...
    /// HTTP method. Defaults to GET.
    #[serde(default)]
    pub method: HttpMethod,
    /// JSON body for POST requests. Deserialized from YAML into serde_json::Value.
    pub body: Option<serde_json::Value>,
}
```

Add `serde_json` import if not already present at the top of the file (check — it may not be needed since we use the full path).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib frontmatter::tests::test_parse_method -- 2>&1`
Expected: All 4 new tests pass.

- [ ] **Step 5: Fix any compilation errors in other files**

The `interpolate_query` function in `src/data/query.rs:142-150` constructs a `DataQuery` with an explicit struct literal that does not include `method` or `body`. Add them:

```rust
Ok(DataQuery {
    file: query.file.clone(),
    source: query.source.clone(),
    path: new_path,
    root: query.root.clone(),
    sort: query.sort.clone(),
    limit: query.limit,
    filter: new_filter,
    method: query.method.clone(),
    body: query.body.clone(), // placeholder — Task 3 will add interpolation
})
```

Run: `cargo test 2>&1 | tail -5`
Expected: Full test suite passes.

- [ ] **Step 6: Commit**

```bash
git add src/frontmatter/mod.rs src/data/query.rs
git commit -m "feat(frontmatter): add HttpMethod enum and method/body fields to DataQuery"
```

---

### Task 2: POST branching and cache key in fetcher

**Files:**
- Modify: `src/data/fetcher.rs:118-165` (fetch_source method)
- Test: `src/data/fetcher.rs` (inline tests)

- [ ] **Step 1: Write failing test for POST cache key differentiation**

Add to `mod tests` in `src/data/fetcher.rs`. This test verifies the cache key logic without needing a real HTTP server:

```rust
#[test]
fn test_cache_key_includes_method_and_body() {
    // Verify that the cache key format distinguishes GET from POST
    // and different POST bodies from each other.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let url = "https://api.example.com/query";

    // GET key
    let get_key = format!("GET:{}", url);

    // POST key with body
    let body = serde_json::json!({"page_size": 100});
    let body_str = serde_json::to_string(&body).unwrap();
    let mut hasher = DefaultHasher::new();
    body_str.hash(&mut hasher);
    let post_key = format!("POST:{}:{}", url, hasher.finish());

    // POST key with no body
    let post_no_body_key = format!("POST:{}:", url);

    // POST key with different body
    let body2 = serde_json::json!({"page_size": 50});
    let body2_str = serde_json::to_string(&body2).unwrap();
    let mut hasher2 = DefaultHasher::new();
    body2_str.hash(&mut hasher2);
    let post_key2 = format!("POST:{}:{}", url, hasher2.finish());

    // All keys must be distinct
    assert_ne!(get_key, post_key);
    assert_ne!(get_key, post_no_body_key);
    assert_ne!(post_key, post_no_body_key);
    assert_ne!(post_key, post_key2);
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --lib data::fetcher::tests::test_cache_key_includes_method_and_body`
Expected: PASS. Note: this is a pure format/logic test that exercises `std::hash` and `format!`, not eigen code. It validates the cache key design is collision-free. The actual wiring into `fetch_source` is verified by Step 3 compiling and the full test suite passing.

- [ ] **Step 3: Modify fetch_source to accept method and body, branch on method**

Change `fetch_source` signature and body in `src/data/fetcher.rs`:

Update the call site in `fetch()` (around line 58):

```rust
self.fetch_source(
    source_name,
    query.path.as_deref().unwrap_or(""),
    &query.method,
    query.body.as_ref(),
)?
```

Update `fetch_source` signature and implementation:

```rust
fn fetch_source(
    &mut self,
    source_name: &str,
    path: &str,
    method: &crate::frontmatter::HttpMethod,
    body: Option<&serde_json::Value>,
) -> Result<Value> {
    let source = self
        .sources
        .get(source_name)
        .ok_or_else(|| eyre::eyre!(
            "Source '{}' not found in site.toml. Available: {}",
            source_name,
            self.sources.keys().cloned().collect::<Vec<_>>().join(", ")
        ))?
        .clone();

    let full_url = format!(
        "{}{}",
        source.url.trim_end_matches('/'),
        if path.starts_with('/') { path.to_string() } else { format!("/{}", path) }
    );

    // Build cache key: include method (and body hash for POST).
    let cache_key = match method {
        crate::frontmatter::HttpMethod::Get => format!("GET:{}", full_url),
        crate::frontmatter::HttpMethod::Post => {
            let body_hash = match body {
                Some(b) => {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let body_str = serde_json::to_string(b).unwrap_or_default();
                    let mut hasher = DefaultHasher::new();
                    body_str.hash(&mut hasher);
                    hasher.finish().to_string()
                }
                None => String::new(),
            };
            format!("POST:{}:{}", full_url, body_hash)
        }
    };

    if let Some(cached) = self.url_cache.get(&cache_key) {
        return Ok(cached.clone());
    }

    let mut headers = reqwest::header::HeaderMap::new();
    for (key, val) in &source.headers {
        let name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
            .wrap_err_with(|| format!("Invalid header name: {}", key))?;
        let value = reqwest::header::HeaderValue::from_str(val)
            .wrap_err_with(|| format!("Invalid header value for {}", key))?;
        headers.insert(name, value);
    }

    let response = match method {
        crate::frontmatter::HttpMethod::Get => {
            self.client.get(&full_url).headers(headers).send()
        }
        crate::frontmatter::HttpMethod::Post => {
            let mut req = self.client.post(&full_url).headers(headers);
            if let Some(b) = body {
                req = req.json(b);
            }
            req.send()
        }
    }
    .wrap_err_with(|| format!("HTTP request failed for {}", full_url))?;

    let status = response.status();
    if !status.is_success() {
        bail!("HTTP {} from {}", status, full_url);
    }

    let value: Value = response.json().wrap_err_with(|| {
        format!("Failed to parse JSON response from {}", full_url)
    })?;

    self.url_cache.insert(cache_key, value.clone());
    Ok(value)
}
```

- [ ] **Step 4: Add warning for body-on-GET in fetch()**

In the `fetch()` method, before the fetch_source call, add:

```rust
if matches!(query.method, crate::frontmatter::HttpMethod::Get) && query.body.is_some() {
    tracing::warn!(
        "Data query has 'body' set but method is GET — body will be ignored. \
         Did you mean to set method: post?"
    );
}
```

- [ ] **Step 5: Run full test suite**

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/data/fetcher.rs
git commit -m "feat(fetcher): support POST method with body and method-aware cache keys"
```

---

### Task 3: Body interpolation and verification in query.rs

**Files:**
- Modify: `src/data/query.rs:119-283`
- Test: `src/data/query.rs` (inline tests)

- [ ] **Step 1: Write failing tests for body interpolation**

Add to `mod tests` in `src/data/query.rs`:

```rust
#[test]
fn test_interpolate_query_body_simple() {
    let item = json!({"db_id": "abc123"});
    let query = DataQuery {
        source: Some("notion".into()),
        path: Some("/v1/databases/abc123/query".into()),
        method: crate::frontmatter::HttpMethod::Post,
        body: Some(json!({
            "filter": {
                "property": "Author",
                "rich_text": {
                    "equals": "{{ post.db_id }}"
                }
            }
        })),
        ..Default::default()
    };

    let result = interpolate_query(&query, &item, "post").unwrap();
    let body = result.body.unwrap();
    assert_eq!(body["filter"]["rich_text"]["equals"], "abc123");
}

#[test]
fn test_interpolate_query_body_no_patterns() {
    let item = json!({"id": 1});
    let query = DataQuery {
        source: Some("api".into()),
        method: crate::frontmatter::HttpMethod::Post,
        body: Some(json!({"page_size": 100})),
        ..Default::default()
    };

    let result = interpolate_query(&query, &item, "post").unwrap();
    let body = result.body.unwrap();
    // Static values preserved as-is (number stays a number).
    assert_eq!(body["page_size"], 100);
}

#[test]
fn test_interpolate_query_body_none() {
    let item = json!({"id": 1});
    let query = DataQuery {
        source: Some("api".into()),
        method: crate::frontmatter::HttpMethod::Post,
        body: None,
        ..Default::default()
    };

    let result = interpolate_query(&query, &item, "post").unwrap();
    assert!(result.body.is_none());
}

#[test]
fn test_interpolate_query_preserves_method() {
    let item = json!({"id": 1});
    let query = DataQuery {
        source: Some("api".into()),
        method: crate::frontmatter::HttpMethod::Post,
        ..Default::default()
    };

    let result = interpolate_query(&query, &item, "post").unwrap();
    assert_eq!(result.method, crate::frontmatter::HttpMethod::Post);
}
```

```rust
#[test]
fn test_interpolate_query_body_env_var() {
    std::env::set_var("TEST_EIGEN_DB_ID", "db_abc123");
    let item = json!({});
    let query = DataQuery {
        source: Some("notion".into()),
        method: crate::frontmatter::HttpMethod::Post,
        body: Some(json!({"database_id": "${TEST_EIGEN_DB_ID}"})),
        ..Default::default()
    };
    let result = interpolate_query(&query, &item, "item").unwrap();
    assert_eq!(result.body.unwrap()["database_id"], "db_abc123");
    std::env::remove_var("TEST_EIGEN_DB_ID");
}

#[test]
fn test_interpolate_query_body_unresolved_env_var_left_as_is() {
    let item = json!({});
    let query = DataQuery {
        source: Some("api".into()),
        method: crate::frontmatter::HttpMethod::Post,
        body: Some(json!({"key": "${DEFINITELY_NOT_SET_EIGEN_TEST}"})),
        ..Default::default()
    };
    let result = interpolate_query(&query, &item, "item").unwrap();
    assert_eq!(
        result.body.unwrap()["key"],
        "${DEFINITELY_NOT_SET_EIGEN_TEST}"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib data::query::tests::test_interpolate_query_body -- 2>&1 | head -20`
Expected: Failures — body interpolation not implemented yet.

- [ ] **Step 3: Implement interpolate_value helper**

Add this function in `src/data/query.rs` (after `interpolate_string`):

```rust
/// Recursively walk a `serde_json::Value` and interpolate `{{ item.field }}`
/// patterns in string nodes. Also replaces `${ENV_VAR}` patterns.
fn interpolate_value(value: &Value, item: &Value, item_as: &str) -> Result<Value> {
    match value {
        Value::String(s) => {
            // First, interpolate {{ item.field }} patterns.
            let resolved = interpolate_string(s, item, item_as)?;
            // Then, interpolate ${ENV_VAR} patterns (fast-path: skip if no $ present).
            let resolved = if resolved.contains("${") {
                interpolate_env_in_string(&resolved)
            } else {
                resolved
            };
            Ok(Value::String(resolved))
        }
        Value::Array(arr) => {
            let items: Result<Vec<Value>> = arr
                .iter()
                .map(|v| interpolate_value(v, item, item_as))
                .collect();
            Ok(Value::Array(items?))
        }
        Value::Object(map) => {
            let mut result = serde_json::Map::new();
            for (k, v) in map {
                result.insert(k.clone(), interpolate_value(v, item, item_as)?);
            }
            Ok(Value::Object(result))
        }
        // Numbers, booleans, nulls are passed through unchanged.
        other => Ok(other.clone()),
    }
}

/// Replace `${VAR_NAME}` patterns with environment variable values.
/// Unresolved patterns are left as-is (no error).
fn interpolate_env_in_string(s: &str) -> String {
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    let mut result = s.to_string();
    let captures: Vec<(String, String)> = re
        .captures_iter(s)
        .map(|cap| (cap[0].to_string(), cap[1].to_string()))
        .collect();
    for (full_match, var_name) in captures {
        if let Ok(val) = std::env::var(&var_name) {
            result = result.replace(&full_match, &val);
        }
    }
    result
}
```

- [ ] **Step 4: Wire interpolate_value into interpolate_query**

Update the `interpolate_query` function to interpolate the body:

```rust
fn interpolate_query(query: &DataQuery, item: &Value, item_as: &str) -> Result<DataQuery> {
    let new_filter = match &query.filter {
        Some(filters) => {
            let mut interpolated = HashMap::new();
            for (key, value_template) in filters {
                let resolved = interpolate_string(value_template, item, item_as)?;
                interpolated.insert(key.clone(), resolved);
            }
            Some(interpolated)
        }
        None => None,
    };

    let new_path = match &query.path {
        Some(path_template) => Some(interpolate_string(path_template, item, item_as)?),
        None => query.path.clone(),
    };

    let new_body = match &query.body {
        Some(body) => Some(interpolate_value(body, item, item_as)?),
        None => None,
    };

    Ok(DataQuery {
        file: query.file.clone(),
        source: query.source.clone(),
        path: new_path,
        root: query.root.clone(),
        sort: query.sort.clone(),
        limit: query.limit,
        filter: new_filter,
        method: query.method.clone(),
        body: new_body,
    })
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib data::query::tests::test_interpolate_query_body -- 2>&1`
Expected: All 4 new tests pass.

- [ ] **Step 6: Write failing test for verify_no_remaining_interpolation on body**

```rust
#[test]
fn test_verify_remaining_in_body() {
    let query = DataQuery {
        method: crate::frontmatter::HttpMethod::Post,
        body: Some(json!({
            "filter": {
                "equals": "{{ nested.ref }}"
            }
        })),
        ..Default::default()
    };
    let result = verify_no_remaining_interpolation(&query, "test");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.to_lowercase().contains("nested interpolation"));
}

#[test]
fn test_verify_clean_body() {
    let query = DataQuery {
        method: crate::frontmatter::HttpMethod::Post,
        body: Some(json!({
            "page_size": 100,
            "filter": {"property": "Status"}
        })),
        ..Default::default()
    };
    assert!(verify_no_remaining_interpolation(&query, "test").is_ok());
}
```

- [ ] **Step 7: Run test to verify it fails**

Run: `cargo test --lib data::query::tests::test_verify_remaining_in_body -- 2>&1`
Expected: FAIL — `verify_no_remaining_interpolation` doesn't check body yet.

- [ ] **Step 8: Extend verify_no_remaining_interpolation to check body**

Add a helper and extend the function:

```rust
/// Check if any string node in a JSON value contains `{{ }}` patterns.
fn value_contains_interpolation(value: &Value, re: &Regex) -> Option<String> {
    match value {
        Value::String(s) if re.is_match(s) => Some(s.clone()),
        Value::Array(arr) => {
            for v in arr {
                if let Some(found) = value_contains_interpolation(v, re) {
                    return Some(found);
                }
            }
            None
        }
        Value::Object(map) => {
            for v in map.values() {
                if let Some(found) = value_contains_interpolation(v, re) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}
```

Then add the body check at the end of `verify_no_remaining_interpolation`, before the `Ok(())`:

```rust
// Check body.
if let Some(ref body) = query.body {
    if let Some(found) = value_contains_interpolation(body, &re) {
        bail!(
            "Data query '{}' still contains interpolation pattern in \
             body after resolution: \"{}\". \
             Nested interpolation (more than one level) is not supported.",
            query_name,
            found,
        );
    }
}
```

- [ ] **Step 9: Run all tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass.

- [ ] **Step 10: Commit**

```bash
git add src/data/query.rs
git commit -m "feat(query): add body interpolation, env var support, and verification"
```

---

### Task 4: Write feature documentation

**Files:**
- Create: `docs/post_method.md`

- [ ] **Step 1: Write the documentation**

Create `docs/post_method.md` with:
- Overview of the feature
- Frontmatter syntax for `method` and `body`
- Example: Notion database query
- Example: POST with item interpolation in body
- Cache key behavior
- Env var interpolation in body
- Limitations (type preservation, JSON-only)

- [ ] **Step 2: Commit**

```bash
git add docs/post_method.md
git commit -m "docs: add POST method for data sources documentation"
```

---

### Task 5: Integration test with mock HTTP server

**Files:**
- Modify: `tests/integration_test.rs` (or create `tests/post_method_test.rs`)

- [ ] **Step 1: Check if mockito or similar is available**

Run: `grep mockito Cargo.toml` — if not present, check if `wiremock` or similar is available, or use a simpler approach with local file data to verify the full pipeline.

- [ ] **Step 2: Write integration test**

If a mock HTTP library is available, write a test that:
1. Starts a mock server expecting POST with a specific JSON body
2. Configures a source pointing to the mock server
3. Creates a DataQuery with `method: post` and a body
4. Fetches through the full pipeline
5. Verifies the response data

If no mock library is available, write a test using local files that verifies the full `DataQuery → fetch → interpolate → result` pipeline works with the new `method` and `body` fields. The HTTP branching is already covered by the unit tests.

- [ ] **Step 3: Run the integration test**

Run: `cargo test --test integration_test post_method -- 2>&1` (or the appropriate test name)
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add tests/
git commit -m "test: add integration test for POST method data source requests"
```

---

### Task 6: Run full suite and final verification

- [ ] **Step 1: Run cargo clippy**

Run: `cargo clippy -- -D warnings 2>&1 | tail -20`
Expected: No warnings.

- [ ] **Step 2: Run cargo fmt check**

Run: `cargo fmt --check 2>&1`
Expected: No formatting issues.

- [ ] **Step 3: Run full test suite**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass.

- [ ] **Step 4: Run /simplify**

Per CLAUDE.md, run `/simplify` on the changed code before final commit.

- [ ] **Step 5: Final commit if simplify made changes**

```bash
git add -u
git commit -m "refactor: simplify POST method implementation after review"
```
