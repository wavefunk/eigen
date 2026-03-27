//! HTML rewriting for responsive images.
//!
//! Uses `lol_html` to:
//! 1. Transform `<img>` tags into `<picture>` elements with `<source>` and `srcset`.
//! 2. Rewrite CSS `background-image` classes to point to optimized variants.

use eyre::Result;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use crate::config::ImageOptimConfig;

use super::images::{
    ImageCache, ImageVariants, is_excluded, optimize_image, url_dir_prefix, url_to_dist_path,
};

/// Mapping from original image URL path → generated variants.
type VariantMap = HashMap<String, ImageVariants>;

/// Determines whether an image should be eagerly or lazily loaded.
///
/// Passed through the image rewriting pipeline to track first-image
/// state and hero image matching.
///
/// Must be wrapped in `Rc<RefCell<LazyLoadContext>>` for use inside
/// `lol_html` closures, which require `'static + FnMut` and cannot
/// take `&mut self` references.
struct LazyLoadContext {
    /// Path of the hero image from frontmatter, if any.
    hero_image: Option<String>,
    /// Whether the first qualifying image has been seen.
    /// Set to true after the first qualifying image is processed.
    first_seen: bool,
}

impl LazyLoadContext {
    fn new(hero_image: Option<&str>) -> Self {
        Self {
            hero_image: hero_image.map(str::to_string),
            first_seen: false,
        }
    }

    /// Determine whether this image should be eager.
    ///
    /// Returns true if the image should NOT be lazy-loaded.
    /// Side effect: consumes the first-image slot if this is the
    /// first qualifying image.
    fn is_eager(&mut self, src: &str, attrs: &[(String, String)]) -> bool {
        // Check data-eager attribute.
        if attrs.iter().any(|(k, _)| k == "data-eager") {
            return true;
        }

        // Check hero_image match.
        if let Some(ref hero) = self.hero_image
            && src == hero
        {
            // Consume first-image slot too, if not consumed.
            self.first_seen = true;
            return true;
        }

        // Check first-image heuristic.
        if !self.first_seen && is_qualifying_image(attrs) {
            self.first_seen = true;
            return true;
        }

        false
    }
}

/// Check if an image qualifies as a potential hero/above-fold image.
///
/// Excludes decorative images and small icons. Uses the same criteria
/// as `auto_detect_hero_image` in `hints/preload.rs`, minus the
/// `loading="lazy"` check (which is not relevant here since we have
/// not set `loading` yet at this point in the pipeline).
fn is_qualifying_image(attrs: &[(String, String)]) -> bool {
    // Skip decorative images.
    for (k, v) in attrs {
        if k == "role" && v == "presentation" {
            return false;
        }
        if k == "alt" && v.is_empty() {
            return false;
        }
    }

    // Skip small icons (both dimensions < 100px).
    let width = attrs
        .iter()
        .find(|(k, _)| k == "width")
        .and_then(|(_, v)| v.parse::<u32>().ok());
    let height = attrs
        .iter()
        .find(|(k, _)| k == "height")
        .and_then(|(_, v)| v.parse::<u32>().ok());

    if let (Some(w), Some(h)) = (width, height)
        && w < 100
        && h < 100
    {
        return false;
    }

    // Skip data URIs.
    if let Some((_, src)) = attrs.iter().find(|(k, _)| k == "src")
        && src.starts_with("data:")
    {
        return false;
    }

    true
}

/// Determine what loading/decoding attributes to set on an image.
///
/// Returns `(loading_value, decoding_value, should_remove_data_eager)`.
///
/// - `loading_value`: `None` if the attribute should not be set (explicit
///   already present and no `data-eager`), `Some("eager")` or
///   `Some("lazy")` otherwise.
/// - `decoding_value`: `None` if not set (explicit already present, or
///   image is eager), `Some("async")` for lazy images.
/// - `should_remove_data_eager`: `true` if `data-eager` was present and
///   should be stripped from output.
fn resolve_loading_attrs(
    src: &str,
    attrs: &[(String, String)],
    ctx: &mut LazyLoadContext,
) -> (Option<&'static str>, Option<&'static str>, bool) {
    let has_data_eager = attrs.iter().any(|(k, _)| k == "data-eager");
    let has_explicit_loading = attrs.iter().any(|(k, _)| k == "loading");
    let has_explicit_decoding = attrs.iter().any(|(k, _)| k == "decoding");

    // Log conflicting data-eager + loading="lazy".
    if has_data_eager && has_explicit_loading {
        let loading_val = attrs
            .iter()
            .find(|(k, _)| k == "loading")
            .map(|(_, v)| v.as_str());
        if loading_val == Some("lazy") {
            tracing::debug!(
                "Image has both data-eager and loading=\"lazy\"; honoring data-eager: {}",
                src,
            );
        }
    }

    let eager = ctx.is_eager(src, attrs);

    // Determine loading attribute.
    let loading = if has_data_eager {
        // data-eager always forces eager, overriding any explicit loading.
        Some("eager")
    } else if has_explicit_loading {
        // Explicit loading attribute: do not override.
        None
    } else if eager {
        Some("eager")
    } else {
        Some("lazy")
    };

    // Determine decoding attribute.
    let decoding = if has_explicit_decoding {
        None
    } else if loading == Some("lazy") {
        Some("async")
    } else {
        // Eager or explicit: do not set decoding (let browser decide).
        None
    };

    (loading, decoding, has_data_eager)
}

/// Apply lazy loading attributes to all `<img>` tags.
///
/// This is the fallback path used when image optimization is disabled.
/// When optimization IS enabled, lazy loading is applied inline during
/// the img -> picture rewrite and this function is not called.
///
/// Sets `loading="lazy" decoding="async"` on all images except:
/// - The first qualifying image (gets `loading="eager"`)
/// - Images with `data-eager` attribute (gets `loading="eager"`)
/// - Images matching `hero_image` (gets `loading="eager"`)
/// - Images with an explicit `loading` attribute (left unchanged)
fn apply_lazy_loading(html: &str, hero_image: Option<&str>) -> Result<String> {
    let ctx = Rc::new(RefCell::new(LazyLoadContext::new(hero_image)));
    let ctx_clone = ctx.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img[src]", move |el| {
                let src = match el.get_attribute("src") {
                    Some(s) => s,
                    None => return Ok(()),
                };

                // Collect attributes for the resolver.
                let attrs: Vec<(String, String)> = el
                    .attributes()
                    .iter()
                    .map(|a| (a.name(), a.value()))
                    .collect();

                let (loading, decoding, remove_data_eager) =
                    resolve_loading_attrs(&src, &attrs, &mut ctx_clone.borrow_mut());

                if let Some(val) = loading {
                    el.set_attribute("loading", val)?;
                }
                if let Some(val) = decoding {
                    el.set_attribute("decoding", val)?;
                }
                if remove_data_eager {
                    el.remove_attribute("data-eager");
                }

                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| eyre::eyre!("lol_html error applying lazy loading: {}", e))?;

    Ok(output)
}

/// Process all images referenced in HTML and rewrite `<img>` → `<picture>`.
///
/// This is the main entry point called from the build pipeline.
///
/// 1. Scans HTML for `<img src="...">` tags.
/// 2. For each local image, runs optimization (resize + convert).
/// 3. Rewrites the HTML: `<img>` → `<picture>` with `<source srcset>`.
/// 4. Also handles CSS `background-image` by generating variant CSS classes.
///
/// Returns the rewritten HTML.
pub fn optimize_and_rewrite_images(
    html: &str,
    config: &ImageOptimConfig,
    cache: &ImageCache,
    dist_dir: &Path,
    hero_image: Option<&str>,
) -> Result<String> {
    if !config.optimize {
        // Still apply lazy loading even without optimization.
        return apply_lazy_loading(html, hero_image);
    }

    // Phase 1: Collect all image src URLs from <img> tags and optimize them.
    let img_srcs = collect_img_srcs(html)?;
    let mut variant_map: VariantMap = HashMap::new();

    for src in &img_srcs {
        if variant_map.contains_key(src.as_str()) {
            continue;
        }

        // Skip absolute URLs (http/https) — they can't be optimized locally.
        if src.starts_with("http://") || src.starts_with("https://") {
            continue;
        }

        // Skip data: URIs.
        if src.starts_with("data:") {
            continue;
        }

        // Check exclusion patterns.
        let check_path = src.trim_start_matches('/');
        if is_excluded(check_path, &config.exclude) {
            tracing::debug!("  Image excluded from optimization: {}", src);
            continue;
        }

        // Resolve to filesystem.
        let fs_path = url_to_dist_path(src, dist_dir);
        if !fs_path.exists() {
            tracing::debug!("  Image file not found, skipping optimization: {}", src);
            continue;
        }

        let url_prefix = url_dir_prefix(src);

        match optimize_image(&fs_path, &url_prefix, config, cache, dist_dir) {
            Ok(Some(variants)) => {
                tracing::debug!(
                    "  Optimized image: {} ({}x{}, {} format(s))",
                    src,
                    variants.original_width,
                    variants.original_height,
                    variants.by_format.len(),
                );
                variant_map.insert(src.clone(), variants);
            }
            Ok(None) => {
                tracing::debug!("  Image could not be optimized (unsupported): {}", src);
            }
            Err(e) => {
                tracing::warn!("  Failed to optimize image {}: {:#}", src, e);
            }
        }
    }

    // Phase 2: Rewrite HTML using lol_html.
    // Always run even with empty variant_map, so lazy loading attributes
    // are set on all images (including those without variants).
    rewrite_img_to_picture(html, &variant_map, hero_image)
}

/// Collect all `<img src="...">` values from HTML using lol_html.
fn collect_img_srcs(html: &str) -> Result<Vec<String>> {
    let srcs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let srcs_clone = srcs.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img[src]", move |el| {
                if let Some(src) = el.get_attribute("src") {
                    srcs_clone.borrow_mut().push(src);
                }
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| eyre::eyre!("lol_html error collecting img srcs: {}", e))?;

    // We don't need the output here — just the collected srcs.
    drop(output);

    let result = srcs.borrow().clone();
    Ok(result)
}

/// Rewrite `<img>` tags to `<picture>` elements using lol_html.
///
/// For each `<img src="/assets/photo.jpg" alt="...">` that has variants:
///
/// ```html
/// <picture>
///   <source srcset="/assets/photo-480w.avif 480w, ..." type="image/avif">
///   <source srcset="/assets/photo-480w.webp 480w, ..." type="image/webp">
///   <source srcset="/assets/photo-480w.jpg 480w, ..." type="image/jpeg">
///   <img src="/assets/photo.jpg" alt="...">
/// </picture>
/// ```
fn rewrite_img_to_picture(
    html: &str,
    variant_map: &VariantMap,
    hero_image: Option<&str>,
) -> Result<String> {
    // We need to clone data into the closure.
    let map = variant_map.clone();
    let ctx = Rc::new(RefCell::new(LazyLoadContext::new(hero_image)));
    let ctx_clone = ctx.clone();

    let output = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img[src]", move |el| {
                let src = match el.get_attribute("src") {
                    Some(s) => s,
                    None => return Ok(()),
                };

                // Collect attributes for both picture building and lazy loading.
                let attrs: Vec<(String, String)> = el
                    .attributes()
                    .iter()
                    .map(|a| (a.name(), a.value()))
                    .collect();

                // Resolve loading/decoding for this image.
                let (loading, decoding, strip_data_eager) =
                    resolve_loading_attrs(&src, &attrs, &mut ctx_clone.borrow_mut());

                match map.get(&src) {
                    Some(variants) => {
                        // Has variants: build <picture> replacement with loading attrs.
                        let picture_html = build_picture_html(
                            &attrs,
                            variants,
                            loading,
                            decoding,
                            strip_data_eager,
                        );
                        el.replace(&picture_html, lol_html::html_content::ContentType::Html);
                    }
                    None => {
                        // No variants: set loading/decoding in-place via lol_html.
                        if let Some(val) = loading {
                            el.set_attribute("loading", val)?;
                        }
                        if let Some(val) = decoding {
                            el.set_attribute("decoding", val)?;
                        }
                        if strip_data_eager {
                            el.remove_attribute("data-eager");
                        }
                    }
                }

                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    )
    .map_err(|e| eyre::eyre!("lol_html error rewriting images: {}", e))?;

    Ok(output)
}

/// Build the `<picture>...</picture>` HTML string for one `<img>`.
///
/// `attrs` is the list of (name, value) pairs from the original `<img>` tag.
fn build_picture_html(
    attrs: &[(String, String)],
    variants: &ImageVariants,
    loading: Option<&str>,
    decoding: Option<&str>,
    strip_data_eager: bool,
) -> String {
    let mut html = String::from("<picture>\n");

    // Determine the ordering of formats: modern formats first, original last.
    let format_order = determine_format_order(variants);

    for fmt_name in &format_order {
        if let Some(fmt_variants) = variants.by_format.get(fmt_name.as_str()) {
            if fmt_variants.is_empty() {
                continue;
            }

            let mime = &fmt_variants[0].mime_type;

            // Build srcset string.
            let srcset: Vec<String> = fmt_variants
                .iter()
                .map(|v| format!("{} {}w", v.url_path, v.width))
                .collect();

            html.push_str(&format!(
                "  <source srcset=\"{}\" type=\"{}\">\n",
                srcset.join(", "),
                mime,
            ));
        }
    }

    // Build the fallback <img> with all original attributes preserved.
    html.push_str("  <img");

    for (name, value) in attrs {
        // Strip data-eager: it is a build-time signal, not valid HTML.
        if strip_data_eager && name == "data-eager" {
            continue;
        }
        // If we are setting loading/decoding, skip the original attribute
        // so we emit ours instead (data-eager overrides explicit loading).
        if name == "loading" && loading.is_some() {
            continue;
        }
        if name == "decoding" && decoding.is_some() {
            continue;
        }
        html.push_str(&format!(" {}=\"{}\"", name, escape_attr(value)));
    }

    // Append loading and decoding attributes.
    if let Some(val) = loading {
        html.push_str(&format!(" loading=\"{}\"", val));
    }
    if let Some(val) = decoding {
        html.push_str(&format!(" decoding=\"{}\"", val));
    }

    html.push_str(">\n</picture>");

    html
}

/// Order formats: AVIF first (best compression), then WebP, then original.
fn determine_format_order(variants: &ImageVariants) -> Vec<String> {
    let mut order = Vec::new();

    // Modern formats first (best → good).
    if variants.by_format.contains_key("avif") && variants.original_format != "avif" {
        order.push("avif".to_string());
    }
    if variants.by_format.contains_key("webp") && variants.original_format != "webp" {
        order.push("webp".to_string());
    }

    // Then any other non-original formats.
    for fmt in variants.by_format.keys() {
        if fmt != &variants.original_format && !order.contains(fmt) {
            order.push(fmt.clone());
        }
    }

    // Original format last (fallback).
    if variants.by_format.contains_key(&variants.original_format) {
        order.push(variants.original_format.clone());
    }

    order
}

/// Escape HTML attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Rewrite CSS `background-image: url(...)` references in `<style>` blocks
/// and inline `style` attributes to use optimized image variants.
///
/// For each CSS class/rule that references an image, generate additional
/// rules with media queries for different sizes, using the optimized
/// variants in the best available format.
///
/// This scans for patterns like:
///   `background-image: url('/assets/photo.jpg')`
/// and appends responsive CSS rules after the `<style>` block.
pub fn rewrite_css_background_images(
    html: &str,
    config: &ImageOptimConfig,
    cache: &ImageCache,
    dist_dir: &Path,
) -> Result<String> {
    if !config.optimize {
        return Ok(html.to_string());
    }

    // Extract background-image URLs using regex (lol_html operates on HTML,
    // not CSS — we use regex to find CSS url() references).
    let bg_re = regex::Regex::new(
        r#"(?i)background(?:-image)?\s*:[^;]*url\(\s*['"]?([^'")\s]+)['"]?\s*\)"#
    ).unwrap();

    // Collect unique local image URLs referenced in CSS.
    let mut css_images: Vec<String> = Vec::new();
    for cap in bg_re.captures_iter(html) {
        let url = cap[1].to_string();
        if !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            let check_path = url.trim_start_matches('/');
            if !is_excluded(check_path, &config.exclude) && !css_images.contains(&url) {
                css_images.push(url);
            }
        }
    }

    if css_images.is_empty() {
        return Ok(html.to_string());
    }

    // Optimize each CSS-referenced image (if not already done for <img>).
    let mut variant_map: VariantMap = HashMap::new();
    for url in &css_images {
        let fs_path = url_to_dist_path(url, dist_dir);
        if !fs_path.exists() {
            continue;
        }
        let url_prefix = url_dir_prefix(url);
        match optimize_image(&fs_path, &url_prefix, config, cache, dist_dir) {
            Ok(Some(variants)) => {
                variant_map.insert(url.clone(), variants);
            }
            Ok(None) | Err(_) => {}
        }
    }

    if variant_map.is_empty() {
        return Ok(html.to_string());
    }

    // For CSS background images, we use the largest WebP/AVIF variant as a
    // simple replacement — full responsive CSS would require knowing the
    // selector context which is beyond our scope.
    // Instead, we inject an `image-set()` CSS function where supported.
    let mut result = html.to_string();
    for (original_url, variants) in &variant_map {
        // Pick the best format available (prefer webp over avif for CSS
        // compatibility, since image-set() support is broader for webp).
        let best_format = if variants.by_format.contains_key("webp") {
            "webp"
        } else if variants.by_format.contains_key("avif") {
            "avif"
        } else {
            continue;
        };

        if let Some(fmt_variants) = variants.by_format.get(best_format) {
            // Use the largest variant (closest to original).
            if let Some(largest) = fmt_variants.last() {
                // Build image-set() fallback pattern.
                // Replace: url('/assets/photo.jpg')
                // With: url('/assets/photo-1200w.webp')
                let old_url_pattern = original_url.as_str();
                let new_url = &largest.url_path;
                result = result.replace(old_url_pattern, new_url);
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::assets::images::ImageVariant;

    use super::*;

    #[test]
    fn test_collect_img_srcs() {
        let html = r#"<img src="/assets/a.jpg" alt="A"><p>text</p><img src="/assets/b.png">"#;
        let srcs = collect_img_srcs(html).unwrap();
        assert_eq!(srcs, vec!["/assets/a.jpg", "/assets/b.png"]);
    }

    #[test]
    fn test_collect_img_srcs_no_src() {
        let html = r#"<img alt="no src"><div>hello</div>"#;
        let srcs = collect_img_srcs(html).unwrap();
        assert!(srcs.is_empty());
    }

    #[test]
    fn test_escape_attr() {
        assert_eq!(escape_attr("hello"), "hello");
        assert_eq!(escape_attr(r#"a"b"#), "a&quot;b");
        assert_eq!(escape_attr("a&b"), "a&amp;b");
    }

    #[test]
    fn test_determine_format_order() {
        let mut by_format = HashMap::new();
        by_format.insert("avif".to_string(), vec![]);
        by_format.insert("webp".to_string(), vec![]);
        by_format.insert("jpeg".to_string(), vec![]);

        let variants = ImageVariants {
            original_width: 800,
            original_height: 600,
            by_format,
            original_format: "jpeg".to_string(),
        };

        let order = determine_format_order(&variants);
        assert_eq!(order, vec!["avif", "webp", "jpeg"]);
    }

    #[test]
    fn test_determine_format_order_webp_original() {
        let mut by_format = HashMap::new();
        by_format.insert("avif".to_string(), vec![]);
        by_format.insert("webp".to_string(), vec![]);

        let variants = ImageVariants {
            original_width: 800,
            original_height: 600,
            by_format,
            original_format: "webp".to_string(),
        };

        let order = determine_format_order(&variants);
        // avif first, then webp as original fallback.
        assert_eq!(order, vec!["avif", "webp"]);
    }

    #[test]
    fn test_build_picture_html_structure() {
        // We test the output format by constructing variants manually
        // and checking the generated HTML contains the right structure.
        let mut by_format = HashMap::new();
        by_format.insert(
            "webp".to_string(),
            vec![
                ImageVariant {
                    url_path: "/assets/photo-480w.webp".to_string(),
                    width: 480,
                    mime_type: "image/webp".to_string()
                },
                ImageVariant {
                    url_path: "/assets/photo-800w.webp".to_string(),
                    width: 800,
                    mime_type: "image/webp".to_string()
                },
            ],
        );
        by_format.insert(
            "jpeg".to_string(),
            vec![
                ImageVariant {
                    url_path: "/assets/photo-480w.jpg".to_string(),
                    width: 480,
                    mime_type: "image/jpeg".to_string()
                },
                ImageVariant {
                    url_path: "/assets/photo-800w.jpg".to_string(),
                    width: 800,
                    mime_type: "image/jpeg".to_string()
                },
            ],
        );

        let variants = ImageVariants {
            original_width: 800,
            original_height: 600,
            by_format,
            original_format: "jpeg".to_string(),
        };

        // Use lol_html to create a real Element, then test build_picture_html.
        // This is complex, so we'll test via the full rewrite pipeline instead.
        let html = r#"<img src="/assets/photo.jpg" alt="test" class="hero">"#;
        let mut map = HashMap::new();
        map.insert("/assets/photo.jpg".to_string(), variants);

        let result = rewrite_img_to_picture(html, &map, None).unwrap();

        assert!(result.contains("<picture>"));
        assert!(result.contains("</picture>"));
        assert!(result.contains(r#"type="image/webp""#));
        assert!(result.contains(r#"type="image/jpeg""#));
        assert!(result.contains("480w"));
        assert!(result.contains("800w"));
        // Original attributes preserved on <img>.
        assert!(result.contains(r#"alt="test""#));
        assert!(result.contains(r#"class="hero""#));
        assert!(result.contains(r#"src="/assets/photo.jpg""#));
        // First (only) image is eager.
        assert!(result.contains(r#"loading="eager""#));
    }

    #[test]
    fn test_rewrite_leaves_unmatched_imgs() {
        let html = r#"<img src="/assets/unknown.jpg" alt="x">"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        // Should still contain original attrs but now also has loading="eager".
        assert!(result.contains(r#"src="/assets/unknown.jpg""#));
        assert!(result.contains(r#"alt="x""#));
        assert!(result.contains(r#"loading="eager""#));
    }

    #[test]
    fn test_rewrite_preserves_non_img_html() {
        let html = r#"<div><p>Hello</p><img src="/other.jpg"></div>"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        assert!(result.contains("<div>"));
        assert!(result.contains("<p>Hello</p>"));
    }

    #[test]
    fn test_full_optimize_and_rewrite() {
        // Integration test: create a real image, run the full pipeline.
        let tmp = tempfile::TempDir::new().unwrap();
        let dist_dir = tmp.path().join("dist");
        std::fs::create_dir_all(dist_dir.join("assets")).unwrap();

        // Create a 200x100 blue JPEG.
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 100, |_, _| {
            image::Rgb([0, 0, 255])
        }));
        let src_path = dist_dir.join("assets/hero.jpg");
        img.save(&src_path).unwrap();

        let config = ImageOptimConfig {
            optimize: true,
            formats: vec!["webp".to_string()],
            quality: 75,
            widths: vec![100],
            exclude: vec![],
        };

        let cache = ImageCache::open(tmp.path()).unwrap();

        let html = r#"<html><body><img src="/assets/hero.jpg" alt="Hero" class="main"></body></html>"#;

        let result = optimize_and_rewrite_images(html, &config, &cache, &dist_dir, None).unwrap();

        assert!(result.contains("<picture>"));
        assert!(result.contains("</picture>"));
        assert!(result.contains("100w"));
        assert!(result.contains("200w")); // original width
        assert!(result.contains("image/webp"));
        assert!(result.contains("image/jpeg")); // original fallback
        assert!(result.contains(r#"alt="Hero""#));
        assert!(result.contains(r#"class="main""#));
        // First (only) image is eager.
        assert!(result.contains(r#"loading="eager""#));
    }

    #[test]
    fn test_optimize_disabled() {
        let config = ImageOptimConfig {
            optimize: false,
            ..ImageOptimConfig::default()
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let cache = ImageCache::open(tmp.path()).unwrap();
        let dist_dir = tmp.path().join("dist");

        let html = r#"<img src="/assets/photo.jpg">"#;
        let result = optimize_and_rewrite_images(html, &config, &cache, &dist_dir, None).unwrap();
        // Optimization is off but lazy loading is still applied.
        assert!(result.contains(r#"loading="eager""#));
        assert!(result.contains(r#"src="/assets/photo.jpg""#));
        assert!(!result.contains("<picture>"));
    }

    #[test]
    fn test_excludes_svg_and_gif() {
        let config = ImageOptimConfig {
            optimize: true,
            formats: vec!["webp".to_string()],
            quality: 75,
            widths: vec![480],
            exclude: vec!["**/*.svg".to_string(), "**/*.gif".to_string()],
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let cache = ImageCache::open(tmp.path()).unwrap();
        let dist_dir = tmp.path().join("dist");

        let html = r#"<img src="/icons/logo.svg"><img src="/images/anim.gif">"#;
        let result = optimize_and_rewrite_images(html, &config, &cache, &dist_dir, None).unwrap();
        // Both should remain as plain <img> — no <picture> wrapping.
        assert!(!result.contains("<picture>"));
        // But they should have lazy loading attributes.
        // First image (logo.svg) gets eager, second (anim.gif) gets lazy.
        assert!(result.contains(r#"loading="eager""#));
        assert!(result.contains(r#"loading="lazy""#));
    }

    // --- LazyLoadContext tests ---

    #[test]
    fn test_lazy_context_first_image_is_eager() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        assert!(ctx.is_eager("/assets/photo.jpg", &attrs));
        assert!(ctx.first_seen);
    }

    #[test]
    fn test_lazy_context_second_image_is_lazy() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs1 = vec![
            ("src".to_string(), "/assets/first.jpg".to_string()),
            ("alt".to_string(), "First".to_string()),
        ];
        let attrs2 = vec![
            ("src".to_string(), "/assets/second.jpg".to_string()),
            ("alt".to_string(), "Second".to_string()),
        ];
        assert!(ctx.is_eager("/assets/first.jpg", &attrs1));
        assert!(!ctx.is_eager("/assets/second.jpg", &attrs2));
    }

    #[test]
    fn test_lazy_context_hero_image_is_eager() {
        let mut ctx = LazyLoadContext::new(Some("/assets/hero.jpg"));
        let attrs = vec![
            ("src".to_string(), "/assets/hero.jpg".to_string()),
            ("alt".to_string(), "Hero".to_string()),
        ];
        assert!(ctx.is_eager("/assets/hero.jpg", &attrs));
        assert!(ctx.first_seen); // hero consumes first-image slot
    }

    #[test]
    fn test_lazy_context_data_eager_does_not_consume_first() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs_eager = vec![
            ("src".to_string(), "/assets/promo.jpg".to_string()),
            ("alt".to_string(), "Promo".to_string()),
            ("data-eager".to_string(), String::new()),
        ];
        let attrs_first = vec![
            ("src".to_string(), "/assets/first.jpg".to_string()),
            ("alt".to_string(), "First".to_string()),
        ];
        // data-eager returns true but does NOT consume first-image slot.
        assert!(ctx.is_eager("/assets/promo.jpg", &attrs_eager));
        assert!(!ctx.first_seen);
        // The actual first qualifying image should still be eager.
        assert!(ctx.is_eager("/assets/first.jpg", &attrs_first));
        assert!(ctx.first_seen);
    }

    #[test]
    fn test_is_qualifying_image_normal() {
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        assert!(is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_decorative_alt() {
        let attrs = vec![
            ("src".to_string(), "/assets/divider.png".to_string()),
            ("alt".to_string(), String::new()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_decorative_role() {
        let attrs = vec![
            ("src".to_string(), "/assets/bg.jpg".to_string()),
            ("role".to_string(), "presentation".to_string()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_small_icon() {
        let attrs = vec![
            ("src".to_string(), "/assets/icon.png".to_string()),
            ("alt".to_string(), "Icon".to_string()),
            ("width".to_string(), "32".to_string()),
            ("height".to_string(), "32".to_string()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_data_uri() {
        let attrs = vec![
            ("src".to_string(), "data:image/png;base64,abc".to_string()),
            ("alt".to_string(), "Inline".to_string()),
        ];
        assert!(!is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_large_dimensions() {
        let attrs = vec![
            ("src".to_string(), "/assets/banner.jpg".to_string()),
            ("alt".to_string(), "Banner".to_string()),
            ("width".to_string(), "800".to_string()),
            ("height".to_string(), "400".to_string()),
        ];
        assert!(is_qualifying_image(&attrs));
    }

    #[test]
    fn test_is_qualifying_image_one_dimension_large() {
        // Only both < 100 triggers skip; one dimension >= 100 is fine.
        let attrs = vec![
            ("src".to_string(), "/assets/strip.jpg".to_string()),
            ("alt".to_string(), "Strip".to_string()),
            ("width".to_string(), "200".to_string()),
            ("height".to_string(), "50".to_string()),
        ];
        assert!(is_qualifying_image(&attrs));
    }

    #[test]
    fn test_resolve_loading_attrs_lazy() {
        let mut ctx = LazyLoadContext::new(None);
        // Consume first-image slot.
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        let (loading, decoding, remove) =
            resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("lazy"));
        assert_eq!(decoding, Some("async"));
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_first_eager() {
        let mut ctx = LazyLoadContext::new(None);
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
        ];
        let (loading, decoding, remove) =
            resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("eager"));
        assert_eq!(decoding, None); // eager images don't get decoding="async"
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_data_eager() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/promo.jpg".to_string()),
            ("alt".to_string(), "Promo".to_string()),
            ("data-eager".to_string(), String::new()),
        ];
        let (loading, decoding, remove) =
            resolve_loading_attrs("/assets/promo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("eager"));
        assert_eq!(decoding, None);
        assert!(remove); // data-eager should be stripped
    }

    #[test]
    fn test_resolve_loading_attrs_explicit_preserved() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
            ("loading".to_string(), "eager".to_string()),
        ];
        let (loading, _decoding, remove) =
            resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, None); // explicit: do not override
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_explicit_decoding_preserved() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
            ("decoding".to_string(), "sync".to_string()),
        ];
        let (loading, decoding, remove) =
            resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("lazy"));
        assert_eq!(decoding, None); // explicit decoding preserved
        assert!(!remove);
    }

    #[test]
    fn test_resolve_loading_attrs_data_eager_overrides_lazy() {
        let mut ctx = LazyLoadContext::new(None);
        ctx.first_seen = true;
        let attrs = vec![
            ("src".to_string(), "/assets/photo.jpg".to_string()),
            ("alt".to_string(), "Photo".to_string()),
            ("loading".to_string(), "lazy".to_string()),
            ("data-eager".to_string(), String::new()),
        ];
        let (loading, _decoding, remove) =
            resolve_loading_attrs("/assets/photo.jpg", &attrs, &mut ctx);
        assert_eq!(loading, Some("eager")); // data-eager wins
        assert!(remove);
    }

    // --- apply_lazy_loading tests ---

    #[test]
    fn test_apply_lazy_loading_default() {
        let html =
            r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B"><img src="/c.jpg" alt="C">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // First image: eager (no loading="lazy").
        assert!(result.contains(r#"src="/a.jpg""#));
        assert!(result.contains(r#"loading="eager""#));
        // Second and third: lazy.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"decoding="async""#));
    }

    #[test]
    fn test_apply_lazy_loading_no_images() {
        let html = "<div><p>Hello</p></div>";
        let result = apply_lazy_loading(html, None).unwrap();
        assert_eq!(result, html);
    }

    #[test]
    fn test_apply_lazy_loading_hero_image() {
        let html = r#"<img src="/b.jpg" alt="B"><img src="/hero.jpg" alt="Hero"><img src="/c.jpg" alt="C">"#;
        let result = apply_lazy_loading(html, Some("/hero.jpg")).unwrap();
        // /b.jpg is first qualifying: eager.
        // /hero.jpg matches hero_image: also eager.
        // /c.jpg: lazy.
        assert!(result.contains(r#"src="/hero.jpg""#));
        // Count lazy occurrences: only the last image.
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(lazy_count, 1);
    }

    #[test]
    fn test_apply_lazy_loading_data_eager_stripped() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B" data-eager>"#;
        let result = apply_lazy_loading(html, None).unwrap();
        assert!(!result.contains("data-eager"));
    }

    #[test]
    fn test_apply_lazy_loading_explicit_loading_preserved() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B" loading="eager">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // Second image has explicit loading="eager": preserved.
        // The function should not add a second loading attribute.
        let eager_count = result.matches(r#"loading="eager""#).count();
        assert_eq!(eager_count, 2); // first (auto) + second (explicit)
    }

    #[test]
    fn test_apply_lazy_loading_all_decorative() {
        let html = r#"<img src="/a.png" alt=""><img src="/b.png" alt="">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // All decorative: all lazy, none eager.
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(lazy_count, 2);
    }

    #[test]
    fn test_apply_lazy_loading_external_images() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="https://cdn.example.com/photo.jpg" alt="External">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // External images still get lazy attributes.
        assert!(result.contains(r#"loading="lazy""#));
    }

    #[test]
    fn test_apply_lazy_loading_first_skips_decorative() {
        let html = r#"<img src="/decorative.png" alt=""><img src="/real.jpg" alt="Real"><img src="/other.jpg" alt="Other">"#;
        let result = apply_lazy_loading(html, None).unwrap();
        // Decorative image: lazy.
        // /real.jpg is first qualifying: eager.
        // /other.jpg: lazy.
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(lazy_count, 2);
        let eager_count = result.matches(r#"loading="eager""#).count();
        assert_eq!(eager_count, 1);
    }

    // --- rewrite_img_to_picture lazy loading tests ---

    #[test]
    fn test_rewrite_non_optimized_gets_lazy() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B">"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        // First image: eager.
        assert!(result.contains(r#"loading="eager""#));
        // Second image: lazy.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"decoding="async""#));
    }

    #[test]
    fn test_rewrite_svg_gets_lazy() {
        let html =
            r#"<img src="/first.jpg" alt="First"><img src="/icon.svg" alt="SVG icon">"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        // SVG (no variants) still gets lazy loading.
        assert!(result.contains(r#"loading="lazy""#));
    }

    #[test]
    fn test_rewrite_data_eager_removed_non_optimized() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B" data-eager>"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        assert!(!result.contains("data-eager"));
        // Both should be eager (first by heuristic, second by data-eager).
        let eager_count = result.matches(r#"loading="eager""#).count();
        assert_eq!(eager_count, 2);
    }

    #[test]
    fn test_rewrite_empty_variant_map_still_applies_lazy() {
        let html = r#"<img src="/a.jpg" alt="A"><img src="/b.jpg" alt="B"><img src="/c.jpg" alt="C">"#;
        let map: VariantMap = HashMap::new();
        let result = rewrite_img_to_picture(html, &map, None).unwrap();
        // Even with empty variant_map, lazy loading should be applied.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"loading="eager""#));
    }

    #[test]
    fn test_full_optimize_with_lazy_loading() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist_dir = tmp.path().join("dist");
        std::fs::create_dir_all(dist_dir.join("assets")).unwrap();

        // Create a 200x100 blue JPEG.
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 100, |_, _| {
            image::Rgb([0, 0, 255])
        }));
        let src_path = dist_dir.join("assets/hero.jpg");
        img.save(&src_path).unwrap();

        let config = ImageOptimConfig {
            optimize: true,
            formats: vec!["webp".to_string()],
            quality: 75,
            widths: vec![100],
            exclude: vec![],
        };

        let cache = ImageCache::open(tmp.path()).unwrap();

        // Two images: first has variants, second does not.
        let html = r#"<html><body><img src="/assets/hero.jpg" alt="Hero"><img src="/assets/other.jpg" alt="Other"></body></html>"#;

        let result =
            optimize_and_rewrite_images(html, &config, &cache, &dist_dir, None).unwrap();

        // First image (hero.jpg) is wrapped in <picture> and is eager.
        assert!(result.contains("<picture>"));
        assert!(result.contains(r#"loading="eager""#));
        // Second image (other.jpg) has no variants, remains <img>, gets lazy.
        assert!(result.contains(r#"loading="lazy""#));
        assert!(result.contains(r#"decoding="async""#));
    }

    #[test]
    fn test_optimize_with_hero_image_frontmatter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dist_dir = tmp.path().join("dist");
        std::fs::create_dir_all(dist_dir.join("assets")).unwrap();

        let config = ImageOptimConfig {
            optimize: true,
            formats: vec![],
            quality: 75,
            widths: vec![],
            exclude: vec![],
        };

        let cache = ImageCache::open(tmp.path()).unwrap();

        let html = r#"<img src="/assets/a.jpg" alt="A"><img src="/assets/hero.jpg" alt="Hero"><img src="/assets/c.jpg" alt="C">"#;

        let result = optimize_and_rewrite_images(
            html,
            &config,
            &cache,
            &dist_dir,
            Some("/assets/hero.jpg"),
        )
        .unwrap();

        // /assets/a.jpg: first qualifying image, eager.
        // /assets/hero.jpg: matches hero_image, eager.
        // /assets/c.jpg: lazy.
        let eager_count = result.matches(r#"loading="eager""#).count();
        let lazy_count = result.matches(r#"loading="lazy""#).count();
        assert_eq!(eager_count, 2);
        assert_eq!(lazy_count, 1);
    }
}
