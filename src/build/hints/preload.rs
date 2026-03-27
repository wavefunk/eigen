//! Hero image preload: detect the hero image, extract responsive variant
//! information from `<picture>` elements or dist_dir scanning, and build
//! the preload `<link>` tag data.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Data needed to build a hero image `<link rel="preload">` tag.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroPreload {
    /// The image URL path (e.g. "/assets/hero.jpg").
    pub href: String,
    /// MIME type of the image (e.g. "image/avif").
    pub mime_type: String,
    /// If the image has responsive variants, the srcset string.
    pub imagesrcset: Option<String>,
    /// The `imagesizes` value (e.g. "100vw").
    pub imagesizes: Option<String>,
}

/// Determine the hero image URL for a page.
///
/// Priority:
/// 1. Explicit `hero_image` from frontmatter.
/// 2. Auto-detected first `<img>` without `loading="lazy"` (if enabled).
/// 3. None.
pub fn resolve_hero_image(
    html: &str,
    frontmatter_hero: Option<&str>,
    auto_detect: bool,
) -> Option<String> {
    // Priority 1: frontmatter.
    if let Some(hero) = frontmatter_hero {
        let hero = hero.trim();
        if !hero.is_empty() {
            return Some(hero.to_string());
        }
    }

    // Priority 2: auto-detect.
    if auto_detect {
        return auto_detect_hero_image(html);
    }

    None
}

/// Build the hero preload hint by extracting responsive variant
/// information from the rendered HTML's `<picture>` elements.
///
/// First checks for a `<picture>` element with a matching `<img src>`.
/// If found, extracts `srcset` and `type` from the first `<source>`.
/// If not found, falls back to scanning dist_dir for variant files.
pub fn build_hero_preload(
    hero_src: &str,
    html: &str,
    dist_dir: &Path,
    image_sizes_fallback: &str,
) -> HeroPreload {
    // Try to find a <picture> element containing this hero image.
    if let Some(preload) = extract_from_picture(hero_src, html, image_sizes_fallback) {
        return preload;
    }

    // Fallback: scan dist_dir for variant files.
    if let Some(preload) = scan_dist_for_variants(hero_src, dist_dir, image_sizes_fallback) {
        return preload;
    }

    // Last resort: simple preload with just href and inferred type.
    HeroPreload {
        href: hero_src.to_string(),
        mime_type: infer_mime_type(hero_src),
        imagesrcset: None,
        imagesizes: None,
    }
}

/// Auto-detect the hero image from rendered HTML.
///
/// Finds the first `<img>` that is not:
/// - `loading="lazy"`
/// - smaller than 100px in both width and height
/// - `role="presentation"` or empty `alt=""`
fn auto_detect_hero_image(html: &str) -> Option<String> {
    let found: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let found_clone = found.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img[src]", move |el| {
                // Only take the first qualifying image.
                if found_clone.borrow().is_some() {
                    return Ok(());
                }

                // Skip lazy-loaded images.
                if let Some(loading) = el.get_attribute("loading") {
                    if loading.eq_ignore_ascii_case("lazy") {
                        return Ok(());
                    }
                }

                // Skip small icons (both dimensions < 100px).
                let width = el
                    .get_attribute("width")
                    .and_then(|v| v.parse::<u32>().ok());
                let height = el
                    .get_attribute("height")
                    .and_then(|v| v.parse::<u32>().ok());
                if let (Some(w), Some(h)) = (width, height) {
                    if w < 100 && h < 100 {
                        return Ok(());
                    }
                }

                // Skip decorative images.
                if let Some(role) = el.get_attribute("role") {
                    if role == "presentation" {
                        return Ok(());
                    }
                }
                if let Some(alt) = el.get_attribute("alt") {
                    if alt.is_empty() {
                        return Ok(());
                    }
                }

                if let Some(src) = el.get_attribute("src") {
                    if !src.is_empty() {
                        *found_clone.borrow_mut() = Some(src);
                    }
                }

                Ok(())
            })],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    let result = found.borrow().clone();
    result
}

/// Extract preload data from a `<picture>` element whose `<img>` has a
/// matching `src`. Returns the srcset and type from the first `<source>`.
fn extract_from_picture(
    hero_src: &str,
    html: &str,
    image_sizes_fallback: &str,
) -> Option<HeroPreload> {
    // State tracking for the streaming parser.
    // We track whether we are inside a <picture> that contains our hero <img>.
    let in_picture: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let found_match: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let first_source: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
    let img_sizes: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Since lol_html processes elements in document order, we track
    // picture/source/img elements as we encounter them.

    let hero_src_owned = hero_src.to_string();

    let in_picture_c1 = in_picture.clone();
    let first_source_c1 = first_source.clone();
    let img_sizes_c1 = img_sizes.clone();
    let in_picture_c2 = in_picture.clone();
    let first_source_c2 = first_source.clone();
    let in_picture_c3 = in_picture.clone();
    let found_match_c = found_match.clone();
    let img_sizes_c2 = img_sizes.clone();

    let _ = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![
                lol_html::element!("picture", move |_el| {
                    *in_picture_c1.borrow_mut() = true;
                    *first_source_c1.borrow_mut() = None;
                    *img_sizes_c1.borrow_mut() = None;
                    Ok(())
                }),
                lol_html::element!("source[srcset]", move |el| {
                    if *in_picture_c2.borrow() && first_source_c2.borrow().is_none() {
                        let srcset = el.get_attribute("srcset").unwrap_or_default();
                        let source_type = el.get_attribute("type").unwrap_or_default();
                        if !srcset.is_empty() && !source_type.is_empty() {
                            *first_source_c2.borrow_mut() = Some((srcset, source_type));
                        }
                    }
                    Ok(())
                }),
                lol_html::element!("img[src]", move |el| {
                    if *in_picture_c3.borrow() {
                        if let Some(src) = el.get_attribute("src") {
                            if src == hero_src_owned {
                                *found_match_c.borrow_mut() = true;
                                if let Some(sizes) = el.get_attribute("sizes") {
                                    *img_sizes_c2.borrow_mut() = Some(sizes);
                                }
                            }
                        }
                        // Reset picture tracking (each picture is independent).
                        *in_picture_c3.borrow_mut() = false;
                    }
                    Ok(())
                }),
            ],
            ..lol_html::RewriteStrSettings::new()
        },
    );

    if !*found_match.borrow() {
        return None;
    }

    let source_data = first_source.borrow().clone();
    match source_data {
        Some((srcset, source_type)) => {
            let sizes = img_sizes
                .borrow()
                .clone()
                .unwrap_or_else(|| image_sizes_fallback.to_string());
            Some(HeroPreload {
                href: hero_src.to_string(),
                mime_type: source_type,
                imagesrcset: Some(srcset),
                imagesizes: Some(sizes),
            })
        }
        None => {
            // Picture found but no <source> with srcset/type -- fallback to simple preload.
            None
        }
    }
}

/// Scan dist_dir for responsive variants of an image.
///
/// Looks for files matching `{stem}-*w-*.{format}` in the same directory.
/// Groups by format, prefers AVIF, then WebP, then original.
fn scan_dist_for_variants(
    hero_src: &str,
    dist_dir: &Path,
    image_sizes_fallback: &str,
) -> Option<HeroPreload> {
    // Extract the directory and stem from the hero image path.
    let hero_path = Path::new(hero_src);
    let stem = hero_path.file_stem()?.to_str()?;
    let parent = hero_path.parent()?;

    // Build the dist directory path for this image's directory.
    let search_dir = dist_dir.join(parent.to_str()?.trim_start_matches('/'));

    if !search_dir.is_dir() {
        return None;
    }

    // Scan for variant files matching {stem}-{width}w-{hash}.{format}
    let variant_pattern = format!("{}-*w-*.*", stem);
    let variant_re = match regex::Regex::new(&format!(
        r"^{}-(\d+)w-[a-f0-9]+\.(\w+)$",
        regex::escape(stem)
    )) {
        Ok(re) => re,
        Err(_) => return None,
    };

    let entries: Vec<_> = match std::fs::read_dir(&search_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                glob::Pattern::new(&variant_pattern)
                    .map(|p| p.matches(&name))
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            tracing::debug!("Failed to read dir for variant scan: {}", e);
            return None;
        }
    };

    if entries.is_empty() {
        return None;
    }

    // Group variants by format, preferring AVIF > WebP > original.
    let mut avif_variants: Vec<(String, u32)> = Vec::new();
    let mut webp_variants: Vec<(String, u32)> = Vec::new();
    let mut other_variants: Vec<(String, u32)> = Vec::new();

    let parent_str = parent.to_str()?;

    for entry in &entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(caps) = variant_re.captures(&name_str) {
            let width: u32 = match caps[1].parse() {
                Ok(w) => w,
                Err(_) => continue,
            };
            let format = &caps[2];
            let url = format!("{}/{}", parent_str, name_str);

            match format {
                "avif" => avif_variants.push((url, width)),
                "webp" => webp_variants.push((url, width)),
                _ => other_variants.push((url, width)),
            }
        }
    }

    // Pick best format group.
    let (variants, mime_type): (Vec<(String, u32)>, String) = if !avif_variants.is_empty() {
        (avif_variants, "image/avif".to_string())
    } else if !webp_variants.is_empty() {
        (webp_variants, "image/webp".to_string())
    } else if !other_variants.is_empty() {
        let mime = infer_mime_type(&other_variants[0].0);
        (other_variants, mime)
    } else {
        return None;
    };

    // Build srcset string: "url1 480w, url2 768w, ..."
    let mut sorted = variants;
    sorted.sort_by_key(|(_, w)| *w);
    let srcset: String = sorted
        .iter()
        .map(|(url, w)| format!("{} {}w", url, w))
        .collect::<Vec<_>>()
        .join(", ");

    Some(HeroPreload {
        href: hero_src.to_string(),
        mime_type,
        imagesrcset: Some(srcset),
        imagesizes: Some(image_sizes_fallback.to_string()),
    })
}

/// Infer MIME type from a file path's extension.
pub fn infer_mime_type(path: &str) -> String {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        _ => "image/jpeg", // fallback
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_hero_frontmatter_priority() {
        let html = r#"<img src="/assets/auto.jpg">"#;
        let result = resolve_hero_image(html, Some("/assets/hero.jpg"), true);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_resolve_hero_auto_detect() {
        let html = r#"<html><body><img src="/assets/banner.jpg" alt="Hero"></body></html>"#;
        let result = resolve_hero_image(html, None, true);
        assert_eq!(result, Some("/assets/banner.jpg".to_string()));
    }

    #[test]
    fn test_resolve_hero_auto_detect_disabled() {
        let html = r#"<img src="/assets/banner.jpg" alt="Hero">"#;
        let result = resolve_hero_image(html, None, false);
        assert!(result.is_none());
    }

    #[test]
    fn test_auto_detect_skips_lazy() {
        let html = r#"
            <img src="/assets/lazy.jpg" loading="lazy" alt="Lazy">
            <img src="/assets/hero.jpg" alt="Hero">
        "#;
        let result = auto_detect_hero_image(html);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_auto_detect_skips_small_icons() {
        let html = r#"
            <img src="/assets/icon.png" width="32" height="32" alt="Icon">
            <img src="/assets/hero.jpg" alt="Hero">
        "#;
        let result = auto_detect_hero_image(html);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_auto_detect_skips_decorative() {
        let html = r#"
            <img src="/assets/spacer.png" role="presentation">
            <img src="/assets/decorative.png" alt="">
            <img src="/assets/hero.jpg" alt="Hero">
        "#;
        let result = auto_detect_hero_image(html);
        assert_eq!(result, Some("/assets/hero.jpg".to_string()));
    }

    #[test]
    fn test_auto_detect_no_qualifying_img() {
        let html = r#"
            <img src="/assets/icon.png" width="16" height="16" alt="x">
            <img src="/assets/lazy.jpg" loading="lazy" alt="Lazy">
        "#;
        let result = auto_detect_hero_image(html);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_from_picture() {
        let html = r#"
            <picture>
                <source srcset="/assets/hero-480w-abc.avif 480w, /assets/hero-768w-abc.avif 768w" type="image/avif">
                <source srcset="/assets/hero-480w-abc.webp 480w, /assets/hero-768w-abc.webp 768w" type="image/webp">
                <img src="/assets/hero.jpg" alt="Hero" sizes="100vw">
            </picture>
        "#;
        let result = extract_from_picture("/assets/hero.jpg", html, "100vw");
        assert!(result.is_some());
        let preload = result.unwrap();
        assert_eq!(preload.href, "/assets/hero.jpg");
        assert_eq!(preload.mime_type, "image/avif");
        assert!(preload.imagesrcset.unwrap().contains("480w"));
        assert_eq!(preload.imagesizes, Some("100vw".to_string()));
    }

    #[test]
    fn test_extract_from_picture_avif_preferred() {
        // The first <source> is AVIF (best format), so preload uses it.
        let html = r#"
            <picture>
                <source srcset="/hero-480w.avif 480w" type="image/avif">
                <source srcset="/hero-480w.webp 480w" type="image/webp">
                <img src="/hero.jpg" alt="Hero">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        let preload = result.unwrap();
        assert_eq!(preload.mime_type, "image/avif");
    }

    #[test]
    fn test_extract_from_picture_sizes_from_img() {
        let html = r#"
            <picture>
                <source srcset="/hero-480w.avif 480w" type="image/avif">
                <img src="/hero.jpg" sizes="(max-width: 600px) 100vw, 50vw" alt="Hero">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        let preload = result.unwrap();
        assert_eq!(
            preload.imagesizes,
            Some("(max-width: 600px) 100vw, 50vw".to_string())
        );
    }

    #[test]
    fn test_extract_from_picture_sizes_fallback() {
        let html = r#"
            <picture>
                <source srcset="/hero-480w.avif 480w" type="image/avif">
                <img src="/hero.jpg" alt="Hero">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        let preload = result.unwrap();
        // No sizes on <img>, so fallback is used.
        assert_eq!(preload.imagesizes, Some("100vw".to_string()));
    }

    #[test]
    fn test_extract_no_matching_picture() {
        let html = r#"
            <picture>
                <source srcset="/other-480w.avif 480w" type="image/avif">
                <img src="/other.jpg" alt="Other">
            </picture>
        "#;
        let result = extract_from_picture("/hero.jpg", html, "100vw");
        assert!(result.is_none());
    }

    #[test]
    fn test_build_hero_preload_simple_no_variants() {
        // When there's no <picture> and no variants on disk, generate simple preload.
        let html = r#"<img src="/assets/hero.jpg" alt="Hero">"#;
        let dir = tempfile::tempdir().unwrap();
        let preload = build_hero_preload("/assets/hero.jpg", html, dir.path(), "100vw");
        assert_eq!(preload.href, "/assets/hero.jpg");
        assert_eq!(preload.mime_type, "image/jpeg");
        assert!(preload.imagesrcset.is_none());
        assert!(preload.imagesizes.is_none());
    }

    #[test]
    fn test_build_hero_preload_from_picture() {
        let html = r#"
            <picture>
                <source srcset="/assets/hero-480w-abc.avif 480w" type="image/avif">
                <img src="/assets/hero.jpg" alt="Hero">
            </picture>
        "#;
        let dir = tempfile::tempdir().unwrap();
        let preload = build_hero_preload("/assets/hero.jpg", html, dir.path(), "100vw");
        assert_eq!(preload.mime_type, "image/avif");
        assert!(preload.imagesrcset.is_some());
    }

    #[test]
    fn test_infer_mime_type() {
        assert_eq!(infer_mime_type("/img.jpg"), "image/jpeg");
        assert_eq!(infer_mime_type("/img.jpeg"), "image/jpeg");
        assert_eq!(infer_mime_type("/img.png"), "image/png");
        assert_eq!(infer_mime_type("/img.webp"), "image/webp");
        assert_eq!(infer_mime_type("/img.avif"), "image/avif");
        assert_eq!(infer_mime_type("/img.gif"), "image/gif");
        assert_eq!(infer_mime_type("/img.svg"), "image/svg+xml");
    }

    #[test]
    fn test_scan_dist_for_variants_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();

        // Create variant files matching the pattern.
        std::fs::write(assets_dir.join("hero-480w-abcdef12.avif"), b"fake").unwrap();
        std::fs::write(assets_dir.join("hero-768w-abcdef12.avif"), b"fake").unwrap();
        std::fs::write(assets_dir.join("hero-480w-abcdef12.webp"), b"fake").unwrap();

        let result = scan_dist_for_variants("/assets/hero.jpg", dir.path(), "100vw");
        assert!(result.is_some());
        let preload = result.unwrap();
        // Should prefer AVIF.
        assert_eq!(preload.mime_type, "image/avif");
        let srcset = preload.imagesrcset.unwrap();
        assert!(srcset.contains("480w"));
        assert!(srcset.contains("768w"));
        // Should not contain webp variants in the AVIF srcset.
        assert!(!srcset.contains("webp"));
    }

    #[test]
    fn test_scan_dist_no_variants() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();

        let result = scan_dist_for_variants("/assets/hero.jpg", dir.path(), "100vw");
        assert!(result.is_none());
    }
}
