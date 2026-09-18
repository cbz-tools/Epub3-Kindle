use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::super::opf::{cover_image_paths, is_legacy_svg_cover_document};
use super::super::xhtml::{
    ViewportQuality, document_styles_with_occupied_hrefs,
    parse_xhtml_semantics_and_document_root_writing_mode, unique_resource_id,
};
use super::manifest::{ManifestIdIndex, merge_rendition};
use super::resolve_href;
use crate::book::{ContentDocument, ReadingOrderItem, Resource, Resources};
use crate::css::SYNTHETIC_INLINE_CSS_PROPERTY;
use crate::error::{Error, Result};
use crate::xhtml::path::{normalize_path_lossy as normalize_path, resolve_path};
use crate::{WarningCode, WarningCollector};

pub(super) struct DiscoveredContent {
    pub(super) content: Vec<ContentDocument>,
    pub(super) reading_items: Vec<ReadingOrderItem>,
    pub(super) document_writing_modes: Vec<Option<crate::book::WritingMode>>,
    pub(super) fixed_page_viewports: Vec<Option<String>>,
}

pub(super) struct ContentDiscoveryContext<'a> {
    pub(super) parsed: &'a super::super::opf::ParsedOpf,
    pub(super) base: &'a Path,
    pub(super) manifest_id_index: &'a ManifestIdIndex,
    pub(super) spine_content_source_indices: &'a [Option<usize>],
    pub(super) dropped_css_hrefs: &'a HashSet<String>,
}

pub(super) fn discover_content(
    context: ContentDiscoveryContext<'_>,
    resources: &mut Resources,
    xhtml: &mut HashMap<String, String>,
    warnings: &mut WarningCollector,
) -> Result<DiscoveredContent> {
    let ContentDiscoveryContext {
        parsed,
        base,
        manifest_id_index,
        spine_content_source_indices,
        dropped_css_hrefs,
    } = context;
    let mut occupied_hrefs = resources
        .items
        .iter()
        .map(|resource| normalize_path(&resource.href))
        .collect::<HashSet<_>>();
    let mut content = Vec::new();
    let mut reading_items = Vec::new();
    let mut document_writing_modes = Vec::new();
    let mut fixed_page_viewports = Vec::new();
    let cover_image_paths = cover_image_paths(parsed, base);
    let mut occupied_resource_ids = resources
        .items
        .iter()
        .map(|resource| resource.id.clone())
        .collect::<HashSet<_>>();
    if spine_content_source_indices.len() != parsed.spine.len() {
        return Err(Error::Output(
            "spine content source index does not match spine".to_owned(),
        ));
    }
    let mut remaining_source_uses = HashMap::<usize, usize>::new();
    for &effective_index in spine_content_source_indices.iter().flatten() {
        *remaining_source_uses.entry(effective_index).or_default() += 1;
    }
    for (spine_index, spine_item) in parsed.spine.iter().enumerate() {
        let source_item = manifest_id_index
            .get(&parsed.manifest, &spine_item.idref)
            .ok_or_else(|| {
                Error::InvalidEpub(format!(
                    "spine references missing manifest item {}",
                    spine_item.idref
                ))
            })?;
        let source = spine_content_source_indices[spine_index].and_then(|effective_index| {
            let effective_id = &parsed.manifest[effective_index].id;
            match remaining_source_uses.get_mut(&effective_index) {
                Some(remaining) if *remaining == 1 => xhtml.remove(effective_id),
                Some(remaining) => {
                    *remaining -= 1;
                    xhtml.get(effective_id).cloned()
                }
                None => None,
            }
        });
        if let Some(source) = source {
            let effective_item = spine_content_source_indices[spine_index]
                .and_then(|index| parsed.manifest.get(index))
                .expect("content source was validated during package loading");
            let page_viewport = if spine_item.layout == super::super::opf::SpineLayout::PrePaginated
            {
                let (viewport_quality, resolution) =
                    super::super::xhtml::fixed_page_viewport(&source)?;
                if viewport_quality == ViewportQuality::Degraded {
                    warnings.add_category_once(
                        WarningCode::W004,
                        "fixed-page viewport metadata was incomplete or ambiguous and was degraded",
                    );
                }
                fixed_page_viewports.push(resolution.clone());
                resolution
            } else {
                None
            };
            let (mut semantic, writing_mode) =
                parse_xhtml_semantics_and_document_root_writing_mode(&source)?;
            semantic.is_cover = semantic.is_cover
                || is_legacy_svg_cover_document(
                    effective_item,
                    &semantic,
                    parsed.spine.first().map(|spine_item| &spine_item.idref),
                    &cover_image_paths,
                    base,
                );
            let is_cover = semantic.is_cover;
            drop(semantic);
            document_writing_modes.push(writing_mode);
            let discovered_styles = document_styles_with_occupied_hrefs(
                &source,
                &effective_item.href,
                &resolve_href(base, &effective_item.href),
                &mut occupied_hrefs,
                warnings,
            )?;
            let referenced_styles = discovered_styles
                .iter()
                .map(|style| style.reference.clone())
                .collect::<Vec<_>>();
            let dropped_stylesheets = referenced_styles
                .iter()
                .filter_map(|reference| resolve_path(&effective_item.href, reference))
                .filter(|resolved| dropped_css_hrefs.contains(resolved))
                .collect();
            let rendition = merge_rendition(parsed.rendition, spine_item.rendition);
            let mut inline_index = 0usize;
            for style in &discovered_styles {
                let Some(source) = style.inline_source.as_deref() else {
                    continue;
                };
                let Some(href) = style.resource_href.as_deref() else {
                    continue;
                };
                let base_id = format!("__inline_style_{}_{}", spine_item.idref, inline_index);
                let resource_id = unique_resource_id(&base_id, &occupied_resource_ids);
                occupied_resource_ids.insert(resource_id.clone());
                resources.items.push(Resource {
                    id: resource_id,
                    href: href.to_owned(),
                    media_type: "text/css".to_owned(),
                    properties: vec![SYNTHETIC_INLINE_CSS_PROPERTY.to_owned()],
                    data: source.as_bytes().to_vec(),
                });
                inline_index += 1;
            }
            let mut content_document = ContentDocument {
                id: spine_item.idref.clone(),
                href: effective_item.href.clone(),
                media_type: effective_item.media_type.clone(),
                source_xhtml: source,
                is_cover,
                source_properties: spine_item.properties.clone(),
                source_spine_index: spine_index,
                referenced_styles,
                dropped_stylesheets,
                page_viewport,
                rendition,
                ..ContentDocument::default()
            };
            content_document.set_layout_pre_paginated(
                spine_item.layout == super::super::opf::SpineLayout::PrePaginated,
            );
            content.push(content_document);
        } else if spine_item.layout == super::super::opf::SpineLayout::PrePaginated {
            fixed_page_viewports.push(None);
        }
        let effective_item = spine_content_source_indices[spine_index]
            .and_then(|index| parsed.manifest.get(index))
            .unwrap_or(source_item);
        reading_items.push(ReadingOrderItem {
            id: spine_item.idref.clone(),
            href: effective_item.href.clone(),
            media_type: effective_item.media_type.clone(),
            linear: spine_item.linear,
        });
    }
    Ok(DiscoveredContent {
        content,
        reading_items,
        document_writing_modes,
        fixed_page_viewports,
    })
}
