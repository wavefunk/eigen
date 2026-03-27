# View Transitions API

Eigen can inject the View Transitions API into your pages, making HTMX
partial swaps animate smoothly instead of instantly replacing content.

## Enabling

```toml
[build.view_transitions]
enabled = true
```

## What It Does

When enabled, three things are injected into every full page:

### 1. Cross-Document Transitions Meta Tag

```html
<meta name="view-transition" content="same-origin">
```

Enables the browser's native view transitions for full page navigations
(initial load, non-HTMX links, browser back/forward).

### 2. HTMX Integration Script

A small inline script that sets `htmx.config.globalViewTransitions = true`.
This makes HTMX wrap all partial swaps in `document.startViewTransition()`,
giving you animated transitions between pages.

### 3. Transition Names on Fragment Targets

Elements whose `id` matches a fragment block name (e.g., `content`,
`sidebar`) automatically get `view-transition-name` added. This lets
the browser animate each region independently instead of a whole-page
cross-fade.

## Progressive Enhancement

Browsers without View Transitions support (currently Firefox) get the
same instant swaps as before. No polyfill is loaded.

## Custom Animations

The browser's default transition is a cross-fade. To customize, add
CSS rules targeting the `::view-transition-*` pseudo-elements:

```css
::view-transition-old(content) {
  animation: slide-out 0.2s ease-in;
}

::view-transition-new(content) {
  animation: slide-in 0.2s ease-out;
}
```

The transition names match your fragment block names (`content`,
`sidebar`, etc.), so you can target each region independently.

## Overriding Transition Names

If you set `view-transition-name` on an element in your own CSS or
inline styles, eigen will not overwrite it.
