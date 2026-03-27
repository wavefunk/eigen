# Lazy Loading for Below-Fold Images

Eigen automatically adds `loading="lazy"` and `decoding="async"` to
below-fold `<img>` tags during the build, improving page load
performance without any configuration.

## How It Works

During the image optimization step (`optimize_and_rewrite_images`),
every `<img>` tag is evaluated and assigned loading attributes:

- **First qualifying image** on each page gets `loading="eager"`.
  This preserves Largest Contentful Paint (LCP) performance for the
  above-fold hero image.
- **All subsequent images** get `loading="lazy" decoding="async"`,
  deferring their download until they approach the viewport.

### First-Image Detection

An image "qualifies" as a potential above-fold image if it is NOT:

- Decorative (`alt=""` or `role="presentation"`)
- A small icon (both `width` and `height` < 100px)
- A data URI (`src="data:..."`)

The first qualifying image in document order is treated as eager.
This heuristic matches the criteria used by the preload hints module
(`auto_detect_hero_image`), keeping the two systems consistent.

## Controlling Eagerness

### `data-eager` attribute

Add `data-eager` to any `<img>` to force eager loading regardless of
position:

```html
<img src="/assets/promo-banner.jpg" data-eager alt="Promo">
```

The `data-eager` attribute is stripped from the output HTML. It does
NOT consume the first-image slot -- the actual first qualifying image
still gets eager treatment independently.

### Explicit `loading` attribute

If a template author sets `loading` explicitly, eigen preserves it:

```html
<img src="/assets/photo.jpg" loading="eager" alt="Photo">
```

Exception: if both `data-eager` and `loading="lazy"` are present,
`data-eager` wins and `loading="eager"` is emitted.

### `hero_image` in frontmatter

```yaml
---
hero_image: /assets/hero-banner.jpg
---
```

Any `<img>` whose `src` matches the `hero_image` path is always
eager. This also consumes the first-image slot.

## Behavior When Optimization Is Disabled

```toml
[assets.images]
optimize = false
```

Images remain as plain `<img>` tags (no `<picture>` wrapping), but
lazy loading attributes are still applied via a lightweight lol_html
pass (`apply_lazy_loading`).

## Fragment Behavior

Fragments (partial HTML loaded via HTMX) get their own independent
`LazyLoadContext`. The first qualifying image in each fragment is
treated as eager; subsequent images are lazy. Fragments do not
receive a `hero_image` (it is a page-level concept).

## Interaction with Preload Hints

The preload hints module auto-detects the hero image by finding the
first `<img>` without `loading="lazy"`. Since lazy loading runs
before hints in the pipeline, and the first qualifying image is
marked eager, auto-detection correctly identifies the same image.
No changes to the hints module are needed.

## Attribute Precedence

1. `data-eager` present: `loading="eager"`, strip `data-eager`
2. Explicit `loading` (no `data-eager`): preserve as-is
3. `src` matches `hero_image`: `loading="eager"`
4. First qualifying image: `loading="eager"`, consume counter
5. All others: `loading="lazy"`

For `decoding`:

1. Explicit `decoding`: preserve as-is
2. Eager image: do not set (let browser decide)
3. Lazy image: `decoding="async"`

## Implementation

All changes are in two existing files:

- `src/assets/html_rewrite.rs`: `LazyLoadContext`, `is_qualifying_image`,
  `resolve_loading_attrs`, `apply_lazy_loading`, and modifications to
  `rewrite_img_to_picture` and `build_picture_html`.
- `src/build/render.rs`: passes `hero_image` to `optimize_and_rewrite_images`.
