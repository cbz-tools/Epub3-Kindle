//! KF8 content preparation stage: classification, projection, and link setup.

use super::css_flow::{CssResourceIndex, ResourceIndex, SectionIndex, referenced_css_resources};
use super::format::to_base32_fixed;
use super::position::{AnchorIndex, assign_aids};
use super::rawml::{
    PendingInternalLink, lower_pre_paginated_section, rewrite_body_aid, rewrite_internal_links,
    rewrite_projected_attributes,
};
use super::rawml_styles::rewrite_stylesheet_links_with_references;
use crate::error::Result;
use crate::kindle::{KindleLayoutSemantic, KindleResource, KindleSection};

pub(super) struct PreparedContent<'a> {
    pub(super) sections: Vec<KindleSection>,
    pub(super) resource_index: ResourceIndex<'a>,
    pub(super) css_resources: CssResourceIndex<'a>,
    pub(super) section_index: SectionIndex,
    pub(super) page_flows: Vec<Vec<u8>>,
    pub(super) pending_links: Vec<Vec<PendingInternalLink>>,
    pub(super) library_thumbnail: Option<Vec<u8>>,
}

pub(super) fn prepare_content<'a>(
    sections: &mut Vec<KindleSection>,
    resources: &'a [KindleResource],
    library_thumbnail: Option<Vec<u8>>,
    cover_resource_id: Option<&str>,
) -> Result<PreparedContent<'a>> {
    let (resource_index, section_lookup, css_resources) = classify_content(sections, resources);
    let sections =
        project_style_and_resources(sections, &resource_index, &css_resources, cover_resource_id)?;
    let (mut sections, page_flows) = normalize_structural_content(sections, &css_resources)?;
    let anchor_indices = assign_positioning_metadata(&mut sections)?;
    let pending_links = prepare_link_materialization(
        &mut sections,
        &section_lookup,
        &anchor_indices,
        &css_resources,
    )?;
    Ok(PreparedContent {
        sections,
        resource_index,
        css_resources,
        section_index: section_lookup,
        page_flows,
        pending_links,
        library_thumbnail,
    })
}

fn classify_content<'a>(
    sections: &[KindleSection],
    resources: &'a [KindleResource],
) -> (
    ResourceIndex<'a>,
    SectionIndex,
    super::css_flow::CssResourceIndex<'a>,
) {
    let resource_index = ResourceIndex::new(resources);
    let section_lookup = SectionIndex::new(sections);
    let css_resources = referenced_css_resources(sections, &resource_index, &section_lookup);
    (resource_index, section_lookup, css_resources)
}

fn project_style_and_resources(
    sections: &mut Vec<KindleSection>,
    resource_index: &ResourceIndex<'_>,
    css_resources: &super::css_flow::CssResourceIndex<'_>,
    cover_resource_id: Option<&str>,
) -> Result<Vec<KindleSection>> {
    let mut projected = Vec::with_capacity(sections.len());
    for (index, section) in std::mem::take(sections).into_iter().enumerate() {
        let source = rewrite_body_aid(section.source_xhtml, &super::rawml::generated_aid(index));
        let source = rewrite_stylesheet_links_with_references(
            source,
            &section.href,
            css_resources,
            Some(&section.referenced_styles),
        );
        let source =
            rewrite_projected_attributes(source, &section.href, cover_resource_id, resource_index)?;
        projected.push(KindleSection {
            id: section.id,
            href: section.href,
            source_xhtml: source,
            referenced_styles: section.referenced_styles,
            linear: section.linear,
            layout: section.layout,
            rendition: section.rendition,
            source_properties: section.source_properties,
            source_spine_index: section.source_spine_index,
        });
    }
    Ok(projected)
}

fn normalize_structural_content(
    mut sections: Vec<KindleSection>,
    css_resources: &super::css_flow::CssResourceIndex<'_>,
) -> Result<(Vec<KindleSection>, Vec<Vec<u8>>)> {
    for section in &mut sections {
        let source = std::mem::take(&mut section.source_xhtml);
        section.source_xhtml = super::rawml::materialize_ordered_list_values(source)?;
    }
    let page_flow_start = css_resources
        .len()
        .checked_add(1)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or_else(|| crate::error::Error::Output("page flow number overflow".to_owned()))?;
    let mut page_flows = Vec::new();
    for section in &mut sections {
        if section.layout != KindleLayoutSemantic::PrePaginated {
            continue;
        }
        let flow_number = page_flow_start
            .checked_add(u32::try_from(page_flows.len()).map_err(|_| {
                crate::error::Error::Output("page flow count exceeds u32".to_owned())
            })?)
            .ok_or_else(|| crate::error::Error::Output("page flow number overflow".to_owned()))?;
        let flow_reference = format!(
            "kindle:flow:{}?mime=image/svg+xml",
            to_base32_fixed(flow_number, 4)?
        );
        let css_reference = section.referenced_styles.iter().find_map(|style_href| {
            super::css_flow::css_flow_number(&section.href, style_href, css_resources)
                .map(super::css_flow::stylesheet_flow_reference)
        });
        let Some((source_xhtml, page_flow)) = lower_pre_paginated_section(
            &section.source_xhtml,
            &flow_reference,
            css_reference.as_deref(),
        ) else {
            return Err(crate::error::Error::Output(format!(
                "pre-paginated section {} has no page presentation",
                section.href
            )));
        };
        section.source_xhtml = source_xhtml;
        page_flows.push(page_flow);
    }
    Ok((sections, page_flows))
}

fn assign_positioning_metadata(sections: &mut [KindleSection]) -> Result<Vec<AnchorIndex>> {
    let mut next_aid = 0u32;
    let mut anchor_indices = Vec::with_capacity(sections.len());
    for section in &mut *sections {
        let source = std::mem::take(&mut section.source_xhtml);
        let assignment = assign_aids(source, &mut next_aid)?;
        section.source_xhtml = assignment.xhtml;
        anchor_indices.push(assignment.anchors);
    }
    Ok(anchor_indices)
}

fn prepare_link_materialization(
    sections: &mut [KindleSection],
    section_lookup: &SectionIndex,
    anchor_indices: &[AnchorIndex],
    css_resources: &super::css_flow::CssResourceIndex<'_>,
) -> Result<Vec<Vec<PendingInternalLink>>> {
    let mut pending_links = Vec::with_capacity(sections.len());
    for (section_number, section) in sections.iter_mut().enumerate() {
        let source = std::mem::take(&mut section.source_xhtml);
        let (source, links) = rewrite_internal_links(
            source,
            &section.href,
            section_lookup,
            section_number,
            anchor_indices,
            css_resources,
        )?;
        section.source_xhtml = source;
        pending_links.push(links);
    }
    Ok(pending_links)
}
