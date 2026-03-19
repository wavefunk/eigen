# Support POST Method for Data Source Requests -- Design Spec

Date: 2026-03-19

## Motivation and Goals

Eigen's data fetching layer currently only supports HTTP GET requests.
Some APIs -- notably Notion's database query endpoint -- require POST
with a JSON body to retrieve data. Without POST support, these APIs
cannot be used as data sources.

**Goals:**

- Add `method` and `body` fields to `DataQuery` so frontmatter can
  specify POST requests with JSON bodies.
- Use a typed `HttpMethod` enum (not a free-form string) for compile-time
  safety.
- Support env var interpolation (`${ENV_VAR}`) in `body` string values
  via explicit interpolation in `interpolate_query` (the config-level
  interpolation only covers `site.toml`, not frontmatter).
- Support item interpolation (`{{ item.field }}`) in `body` string values
  for dynamic page per-item queries.
- Cache POST requests using a key that includes both URL and body.
- Zero impact on existing sites (both fields are optional with safe
  defaults; `method` defaults to GET).

**Non-goals:**

- Other HTTP methods (PUT, PATCH, DELETE). Only GET and POST for now.
- Form-encoded bodies. Only JSON bodies are supported.
- Request-level timeout or retry configuration.

## Data Model

### HttpMethod Enum

New enum in `src/frontmatter/mod.rs`:

```rust
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
}
```

Serde deserializes `"get"` and `"post"` from YAML frontmatter. Defaults
to `Get` when omitted.

### DataQuery Changes

Two new fields on `DataQuery`:

```rust
pub struct DataQuery {
    // ... existing fields ...
    #[serde(default)]
    pub method: HttpMethod,
    pub body: Option<serde_json::Value>,
}
```

The `body` field is `serde_json::Value` even though frontmatter is YAML.
`serde_yaml` can deserialize directly into `serde_json::Value` via its
`Deserialize` impl. This ensures the body is ready for reqwest's
`.json(&body)` without conversion. YAML-specific types (e.g., `yes`/`no`
as booleans) are normalized during deserialization.

### Frontmatter Usage

```yaml
data:
  projects:
    source: notion
    path: /v1/databases/${NOTION_DB_ID}/query
    method: post
    body:
      page_size: 100
      filter:
        property: "Status"
        status:
          equals: "Published"
    root: results
```

## Fetcher Changes

In `src/data/fetcher.rs`, the `fetch_source` method branches on the
query's method:

- **GET:** `self.client.get(&full_url).headers(headers).send()` (unchanged)
- **POST:** `self.client.post(&full_url).headers(headers).json(&body).send()`
  If `body` is `None`, send POST with no body.

### Cache Key

The cache key must include the HTTP method to prevent collisions between
GET and POST requests to the same URL. Format:

- **GET:** `GET:<url>` (or just `<url>` for backward compat, but
  prefixing is cleaner)
- **POST:** `POST:<url>:<body_hash>` where `body_hash` is a hash of the
  serialized body (or empty string if no body).

The hash must be computed on the **already-interpolated** body, not the
template body, so that per-item queries with different interpolated
values get distinct cache entries.

## Interpolation

In `src/data/query.rs`, `interpolate_query` is extended to handle the
`body` field. The body is a `serde_json::Value` tree. A new helper
function `interpolate_value` recursively walks it and replaces
`{{ item.field }}` patterns in any `Value::String` nodes, using the
same item-lookup logic that already handles `filter` and `path`
interpolation.

The `interpolate_query` struct literal must also be updated to forward
`method` (cloned) and the interpolated `body`.

### Env Var Interpolation in Body

The config-level `interpolate_env_vars` only operates on the raw
`site.toml` string. Frontmatter is parsed separately, so `${ENV_VAR}`
patterns in body strings are **not** covered by the existing pipeline.
The `interpolate_value` helper must also replace `${ENV_VAR}` patterns
in body string nodes, using `std::env::var`. This is the same semantics
as config interpolation but applied at query resolution time.

### verify_no_remaining_interpolation

`verify_no_remaining_interpolation` in `query.rs` currently checks
`filter` and `path` for leftover `{{ }}` patterns. It must be extended
to also walk body string nodes and reject any remaining unresolved
patterns. Without this, a typo like `{{ item.typo }}` in the body would
be sent verbatim to the API.

### Type Preservation

Interpolation replaces `Value::String` nodes. When `{{ item.field }}`
resolves to a non-string value (number, boolean), the result is still
placed as a string. This is the same limitation as existing filter
interpolation. For the body, static values written directly in YAML
(e.g., `page_size: 100`) are deserialized as their correct JSON types
and are not affected. Only interpolated values become strings. This is
an acceptable tradeoff for now; a future enhancement could add typed
interpolation.

## Error Handling

- **POST with body but method GET:** Log a warning ("body field ignored
  for GET request") to surface likely configuration mistakes. Do not
  error -- GET with body is technically valid HTTP.
- **POST with no body:** Valid. Send POST with empty body.
- **Body interpolation failure:** Same error path as existing filter/path
  interpolation -- returns an `eyre::Result` error with context about
  which field and pattern failed.

## Testing

1. **Unit: deserialization** -- `DataQuery` with `method: post` and a
   nested body deserializes correctly. `HttpMethod` defaults to `Get`
   when omitted.
2. **Unit: body interpolation** -- `{{ item.field }}` patterns in nested
   body JSON are replaced correctly, including deeply nested strings.
3. **Unit: cache key** -- Same URL with different POST bodies produces
   different cache keys. Same URL with same body produces the same key.
4. **Integration: POST request** -- A POST request with body hits a mock
   endpoint (using a local test server or mockito) and returns expected
   data through the full pipeline.

## Files Changed

| File | Change |
|------|--------|
| `src/frontmatter/mod.rs` | Add `HttpMethod` enum, add `method` and `body` fields to `DataQuery` |
| `src/data/fetcher.rs` | Branch on method in `fetch_source`, update cache key for POST |
| `src/data/query.rs` | Extend `interpolate_query` to walk and interpolate `body` values; update struct literal to forward `method` and `body`; extend `verify_no_remaining_interpolation` to check body; add `interpolate_value` helper; add env var interpolation for body strings |
| `src/data/transforms.rs` | No changes needed |
| `src/data/global.rs` | No changes needed |
| `src/config/mod.rs` | No changes needed |
