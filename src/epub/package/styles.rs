use std::collections::HashSet;
use std::path::Path;

use super::super::css::font_face_resource_references;
use super::manifest::is_font_media_type;
use super::resolve_href;
use crate::book::{ContentDocument, Resources, Styles};
use crate::css::SYNTHETIC_INLINE_CSS_PROPERTY;
use crate::error::{Error, Result};
use crate::xhtml::path::{
    is_external_reference, normalize_path_lossy as normalize_path, resolve_path,
};
use crate::{WarningCode, WarningCollector};

pub(super) fn validate_and_parse_styles(
    content: &[ContentDocument],
    resources: &Resources,
    base: &Path,
    warnings: &mut WarningCollector,
) -> Result<Styles> {
    let css_resource_index = super::super::css::CssResourceIndex::new(content, &resources.items);
    let active_css_hrefs =
        super::super::css::active_css_stylesheets(&css_resource_index, content, warnings)?;
    let mut styles = Styles::default();
    for resource in resources
        .items
        .iter()
        .filter(|resource| resource.media_type.eq_ignore_ascii_case("text/css"))
    {
        if !active_css_hrefs.contains(&normalize_path(&resource.href)) {
            continue;
        }
        let source = std::str::from_utf8(&resource.data)
            .expect("EPUB CSS resources are normalized to UTF-8");
        if !resource
            .properties
            .iter()
            .any(|property| property == SYNTHETIC_INLINE_CSS_PROPERTY)
        {
            let stylesheet_path = resolve_href(base, &resource.href);
            super::super::css::validate_local_resource_paths(source, &stylesheet_path)?;
        }
        super::super::css::warn_remote_css_references(source, warnings)?;
        super::super::css::validate_kf8_css_with_warnings(source, warnings)?;
        styles
            .sheets
            .push(crate::epub::parse_css(&resource.href, source));
    }
    validate_required_font_resources(
        &active_css_hrefs,
        resources,
        base,
        &css_resource_index,
        warnings,
    )?;
    styles.computed = styles
        .sheets
        .iter()
        .flat_map(|sheet| sheet.computed_styles())
        .collect();
    Ok(styles)
}

fn validate_required_font_resources(
    active_css_hrefs: &HashSet<String>,
    resources: &Resources,
    base: &Path,
    css_resource_index: &super::super::css::CssResourceIndex<'_>,
    warnings: &mut WarningCollector,
) -> Result<()> {
    for stylesheet in resources.items.iter().filter(|resource| {
        resource.media_type.eq_ignore_ascii_case("text/css")
            && active_css_hrefs.contains(&normalize_path(&resource.href))
    }) {
        let targets = font_face_resource_references(
            std::str::from_utf8(&stylesheet.data)
                .expect("EPUB CSS resources are normalized to UTF-8"),
        )?;
        let stylesheet_base_href =
            super::super::css::css_resource_base_href(css_resource_index, stylesheet);
        let stylesheet_path = resolve_href(base, &stylesheet_base_href);
        for target in targets {
            if is_external_reference(&target) {
                continue;
            }
            let resolved = resolve_path(&stylesheet_path, &target).ok_or_else(|| {
                Error::InvalidEpub(format!("font resource path {target} escapes the EPUB root"))
            })?;
            if let Some(font) = resources.items.iter().find(|resource| {
                normalize_path(&resolve_href(base, &resource.href)) == resolved
                    && is_font_media_type(&resource.media_type)
            }) {
                if font.data.is_empty() {
                    return Err(Error::InvalidEpub(format!(
                        "required embedded font {} has zero length",
                        font.id
                    )));
                }
            } else {
                warnings.add_category_once(
                    WarningCode::W004,
                    "one or more local @font-face resources could not be resolved and will use a fallback",
                );
            }
        }
    }
    Ok(())
}
