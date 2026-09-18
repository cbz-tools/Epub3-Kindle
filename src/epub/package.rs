//! Orchestrate EPUB package resources into the final Book representation.
//!
//! Package loading, resource decoding, spine content discovery, styles, and
//! navigation are kept in focused private child modules.

mod content;
mod manifest;
mod navigation;
mod resources;
mod styles;

use std::path::Path;

use super::xhtml::infer_layout;
use crate::WarningCollector;
use crate::book::Book;
use crate::error::Result;
use crate::xhtml::path::resolve_path;

use content::{DiscoveredContent, discover_content};
use manifest::{LoadedPackage, load_package};
use navigation::load_navigation;
use resources::{LoadedResources, load_resources};
use styles::validate_and_parse_styles;

#[allow(dead_code)]
pub fn parse_epub(input: &[u8]) -> Result<Book> {
    let mut warnings = WarningCollector::new();
    parse_epub_with_warnings(input, &mut warnings)
}

pub fn parse_epub_with_warnings(input: &[u8], warnings: &mut WarningCollector) -> Result<Book> {
    let LoadedPackage {
        mut archive,
        parsed,
        base,
        manifest_id_index,
        font_obfuscation_keys,
        content_source_ids,
        spine_content_source_indices,
    } = load_package(input, warnings)?;
    let LoadedResources {
        resources,
        mut xhtml,
        dropped_css_hrefs,
    } = load_resources(
        &mut archive,
        &parsed,
        &base,
        &font_obfuscation_keys,
        &content_source_ids,
        &manifest_id_index,
        warnings,
    )?;
    let mut resources = resources;
    let DiscoveredContent {
        content,
        reading_items,
        document_writing_modes,
        fixed_page_viewports,
    } = discover_content(
        &parsed,
        &base,
        &mut resources,
        &mut xhtml,
        &manifest_id_index,
        &spine_content_source_indices,
        &dropped_css_hrefs,
        warnings,
    )?;
    let styles = validate_and_parse_styles(&content, &resources, &base, warnings)?;

    let navigation = load_navigation(&mut archive, &parsed, &base, &manifest_id_index)?;
    let layout = infer_layout(
        &styles,
        parsed.page_progression,
        parsed.primary_writing_mode,
        &document_writing_modes,
    );
    let cover = parsed.metadata.cover.clone().or_else(|| {
        parsed
            .manifest
            .iter()
            .find(|item| {
                item.properties
                    .iter()
                    .any(|property| property.eq_ignore_ascii_case("cover-image"))
            })
            .map(|item| item.id.clone())
    });
    let mut metadata = parsed.metadata;
    metadata.cover = cover;
    if metadata.original_resolution.is_none()
        && !fixed_page_viewports.is_empty()
        && fixed_page_viewports.iter().all(Option::is_some)
    {
        let first = fixed_page_viewports[0].as_deref();
        if fixed_page_viewports
            .iter()
            .all(|viewport| viewport.as_deref() == first)
        {
            metadata.original_resolution = first.map(str::to_owned);
        }
    }
    Ok(Book {
        metadata,
        reading_order: crate::book::ReadingOrder {
            items: reading_items,
            page_progression: parsed.page_progression,
        },
        navigation,
        content,
        resources,
        layout,
        rendition: parsed.rendition,
        styles,
    })
}

pub(super) fn resolve_href(base: &Path, href: &str) -> String {
    let base = base.to_string_lossy();
    let base = format!("{base}/");
    resolve_path(&base, href).unwrap_or_default()
}
