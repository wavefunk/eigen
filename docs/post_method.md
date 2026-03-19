# POST Method for Data Sources

Data queries in frontmatter support HTTP POST requests with JSON bodies.
This lets you call APIs that require a request body — such as GraphQL
endpoints, Notion database queries, or any REST API that uses POST for
reads.

## Frontmatter Syntax

Add `method: POST` and a `body` field to any data query:

```yaml
---
data:
  - name: results
    url: https://api.example.com/query
    method: POST
    body:
      filter:
        status: published
      page_size: 100
---
```

The `body` value is serialized as JSON and sent with
`Content-Type: application/json`. The `method` field is
case-insensitive (`POST`, `post`, `Post` all work).

## Example: Notion Database Query

```yaml
---
data:
  - name: posts
    url: https://api.notion.com/v1/databases/${NOTION_DB_ID}/query
    method: POST
    headers:
      Authorization: Bearer ${NOTION_API_KEY}
      Notion-Version: "2022-06-28"
    body:
      filter:
        property: Status
        select:
          equals: Published
      sorts:
        - property: Date
          direction: descending
      page_size: 50
---
```

`${NOTION_DB_ID}` and `${NOTION_API_KEY}` are resolved from environment
variables at query time (see [Env Var Interpolation](#env-var-interpolation-in-body)).

## Example: POST with Item Interpolation

Dynamic collection pages can use `{{ item.field }}` inside the body.
Eigen resolves these per item when rendering the collection:

```yaml
---
collection:
  source: posts
data:
  - name: related
    url: https://api.example.com/related
    method: POST
    body:
      tags: "{{ item.tags }}"
      exclude_id: "{{ item.id }}"
      limit: 5
---
```

Each item in the `posts` collection triggers its own POST request with
the body interpolated for that item's values.

## Cache Key Behavior

- **GET requests** — the cache key is the URL.
- **POST requests** — the cache key is `url + sha256(body)`. Two POST
  requests to the same URL with different bodies are cached separately.

If you use item interpolation in the body, each unique rendered body
produces its own cache entry. This means a collection of 50 items with
50 distinct bodies results in 50 cached responses.

## Env Var Interpolation in Body

`${ENV_VAR}` patterns inside body values are resolved at query time,
before the body is serialized to JSON. This works at any nesting depth:

```yaml
body:
  auth:
    token: ${API_TOKEN}
  workspace: ${WORKSPACE_ID}
```

Interpolation happens after the YAML is parsed, so the substitution
targets are always string values. If the variable is not set, eigen
logs a warning and leaves the placeholder as-is.

## Limitations

**Type preservation** — interpolated values (`{{ item.field }}` and
`${ENV_VAR}`) are always substituted as strings. If your API expects a
number or boolean, wrap the field in a type coercion on the API side or
use a template filter if the value is sourced from a template variable.

**JSON bodies only** — the `body` field must be a YAML mapping that
serializes to a JSON object. Form-encoded bodies and raw string bodies
are not supported.

**Read-only methods** — only `GET` and `POST` are supported. PUT, PATCH,
and DELETE are not available as data source methods.

## Warning Behavior

If you set `body` on a `GET` request, eigen logs a warning and ignores
the body:

```
WARN: data query for "results" has a body but method is GET — body ignored
```

Either remove the `body` field or change `method` to `POST`.
