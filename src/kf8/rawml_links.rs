//! KF8 internal-link rewriting and final position materialization.

use super::css_flow::{CssResourceIndex, SectionIndex, css_flow_number};
use super::format::to_base32_fixed;
use super::position::{self, PositionMap};
use super::rawml::{POSFID_PLACEHOLDER, SectionParts};
use super::rawml_attributes::{
    AttributeRewrite, rewrite_quoted_attributes, tag_has_attribute_token,
};
use crate::WarningCode;
use crate::WarningCollector;
use crate::error::Result;
use crate::kindle::KindleSection;
use crate::xhtml::path::{is_external_reference, percent_decode, resolve_path};
use quick_xml::{Reader, events::Event};

fn generated_section_path(index: usize) -> String {
    format!("Text/part{index:04}.xhtml")
}

#[derive(Debug, Clone)]
pub(super) struct PendingInternalLink {
    section_index: usize,
    fragment: Option<String>,
}

pub(super) fn rewrite_internal_links(
    source: String,
    section_href: &str,
    section_index: &SectionIndex,
    section_number: usize,
    anchor_indices: &[position::AnchorIndex],
    css_resources: &CssResourceIndex<'_>,
    dropped_stylesheets: &[String],
    warnings: &mut WarningCollector,
) -> Result<(String, Vec<PendingInternalLink>)> {
    if anchor_indices.len() != section_index.len() || section_number >= anchor_indices.len() {
        return Err(crate::error::Error::Output(
            "internal link anchor indexes do not match sections".to_owned(),
        ));
    }
    let mut pending = Vec::new();
    let rewritten = rewrite_quoted_attributes(
        source,
        &["href="],
        |_source, tag_name, tag, target| {
            let tag_name = tag_name.rsplit(':').next().unwrap_or(tag_name);
            if tag_name.eq_ignore_ascii_case("image") || tag_name.eq_ignore_ascii_case("use") {
                return Ok(None);
            }
            if tag_name.eq_ignore_ascii_case("link")
                && tag_has_attribute_token(tag, "rel", "pronunciation")
            {
                return Ok(None);
            }
            if tag_name.eq_ignore_ascii_case("link")
                && tag_has_attribute_token(tag, "rel", "stylesheet")
                && resolve_path(section_href, target)
                    .is_some_and(|resolved| dropped_stylesheets.contains(&resolved))
            {
                return Ok(None);
            }
            if tag_name.eq_ignore_ascii_case("link")
                && tag_has_attribute_value(tag, "type", "application/adobe-page-template+xml")
            {
                return Ok(None);
            }
            // CSS is a non-document transport resource only when it resolves in
            // the planned graph. Do not infer that role from a filename suffix;
            // extensionless CSS resources are valid and must not become anchors.
            if css_flow_number(section_href, target, css_resources).is_some()
                || is_external_reference(target)
            {
                return Ok(None);
            }
            let (target_path, fragment) = split_target(target);
            let target_section = resolve_section_target(
                section_index,
                section_href,
                if target_path.is_empty() {
                    section_href
                } else {
                    target_path
                },
            );
            let Some(target_section) = target_section else {
                if is_unresolved_root_relative_web_link(tag_name, target_path) {
                    warnings.add_category_once(
                        WarningCode::W006,
                        "unresolvable root-relative web-style hyperlinks were dropped while preserving readable link text",
                    );
                    return Ok(Some(AttributeRewrite::Remove));
                }
                if target_path.is_empty()
                    || target_path
                        .rsplit('/')
                        .next()
                        .is_some_and(|name| name.contains('.'))
                {
                    return Err(crate::error::Error::Output(format!(
                        "internal link target does not resolve to a generated document: {target}"
                    )));
                }
                return Ok(None);
            };
            if let Some(fragment) = fragment {
                let anchor_offset = anchor_indices[target_section].offset(fragment);
                if anchor_offset.is_none() {
                    return Err(crate::error::Error::Output(format!(
                        "internal link fragment does not resolve in generated document: {target}"
                    )));
                }
                pending.push(PendingInternalLink {
                    section_index: target_section,
                    fragment: Some(fragment.to_owned()),
                });
            } else {
                pending.push(PendingInternalLink {
                    section_index: target_section,
                    fragment: None,
                });
            }
            Ok(Some(AttributeRewrite::Replace(
                POSFID_PLACEHOLDER.to_owned(),
            )))
        },
    )?;
    Ok((rewritten, pending))
}

fn is_unresolved_root_relative_web_link(tag_name: &str, target_path: &str) -> bool {
    if !matches!(tag_name.to_ascii_lowercase().as_str(), "a" | "area")
        || !target_path.starts_with('/')
    {
        return false;
    }

    let path = target_path.split('?').next().unwrap_or_default();
    let Some(path) = percent_decode(path) else {
        return false;
    };
    let last_component = path.rsplit('/').next().unwrap_or_default();
    match last_component.rsplit_once('.') {
        None => true,
        Some((_, extension)) => extension.eq_ignore_ascii_case("php"),
    }
}

fn tag_has_attribute_value(tag: &str, name: &str, value: &str) -> bool {
    let mut reader = Reader::from_str(tag);
    let Ok(event) = reader.read_event() else {
        return false;
    };
    let element = match event {
        Event::Start(element) | Event::Empty(element) => element,
        _ => return false,
    };
    element.attributes().flatten().any(|attribute| {
        attribute.key.as_ref().eq_ignore_ascii_case(name.as_bytes())
            && attribute
                .unescape_value()
                .is_ok_and(|attribute_value| attribute_value.eq_ignore_ascii_case(value))
    })
}

pub(super) fn materialize_internal_links(
    sections: &mut [KindleSection],
    pending_links: &[Vec<PendingInternalLink>],
    position_map: &PositionMap,
    section_parts: &mut [SectionParts],
) -> Result<()> {
    if sections.len() != pending_links.len() || sections.len() != section_parts.len() {
        return Err(crate::error::Error::Output(
            "internal link sections and fragments do not match".to_owned(),
        ));
    }
    let href_snapshot = sections
        .iter()
        .map(|section| section.href.clone())
        .collect::<Vec<_>>();
    for (section_index, (section, pending)) in sections.iter_mut().zip(pending_links).enumerate() {
        // The source is only needed to locate placeholders. Rebuilding the
        // complete XHTML here creates a temporary copy that is discarded by
        // build_geometry immediately after materialization.
        let source = section.source_xhtml.as_str();
        let mut cursor = 0;
        let mut context_index = 0;
        let mut removed_before = 0;
        for target in pending {
            let Some(relative) = source[cursor..].find(POSFID_PLACEHOLDER) else {
                return Err(crate::error::Error::Output(format!(
                    "internal link placeholder is missing in generated document {}",
                    generated_section_path(section_index)
                )));
            };
            let start = cursor + relative;
            let target_href = if let Some(fragment) = target.fragment.as_deref() {
                format!(
                    "{}#{fragment}",
                    href_snapshot[target.section_index].as_str()
                )
            } else {
                href_snapshot[target.section_index].clone()
            };
            let resolved = position_map.resolve(&target_href)?;
            let mut replacement = String::with_capacity(POSFID_PLACEHOLDER.len());
            replacement.push_str("kindle:pos:fid:");
            replacement.push_str(&to_base32_fixed(resolved.sequence_number, 4)?);
            replacement.push_str(":off:");
            replacement.push_str(&to_base32_fixed(resolved.payload_offset, 10)?);
            if replacement.len() != POSFID_PLACEHOLDER.len() {
                return Err(crate::error::Error::Output(
                    "internal link replacement changed XHTML geometry".to_owned(),
                ));
            }
            replace_section_part_bytes(
                &mut section_parts[section_index],
                start,
                replacement.as_bytes(),
                &mut context_index,
                &mut removed_before,
            )?;
            cursor = start + POSFID_PLACEHOLDER.len();
        }
        drop(std::mem::take(&mut section.source_xhtml));
    }
    Ok(())
}

fn replace_section_part_bytes(
    parts: &mut SectionParts,
    source_start: usize,
    replacement: &[u8],
    context_index: &mut usize,
    removed_before: &mut usize,
) -> Result<()> {
    let source_end = source_start.checked_add(replacement.len()).ok_or_else(|| {
        crate::error::Error::Output("internal link replacement range is invalid".to_owned())
    })?;
    let mut source_cursor = source_start;
    let mut replacement_cursor = 0;
    while source_cursor < source_end {
        while let Some(context) = parts.fragment_contexts.get(*context_index) {
            if context.source_end > source_cursor {
                break;
            }
            let removed = context
                .source_end
                .checked_sub(context.source_start)
                .ok_or_else(|| {
                    crate::error::Error::Output(
                        "internal link fragment range is invalid".to_owned(),
                    )
                })?;
            *removed_before = removed_before.checked_add(removed).ok_or_else(|| {
                crate::error::Error::Output("internal link fragment range is invalid".to_owned())
            })?;
            *context_index += 1;
        }

        let next_end = parts
            .fragment_contexts
            .get(*context_index)
            .map(|context| context.source_start)
            .filter(|&start| start > source_cursor)
            .unwrap_or(source_end)
            .min(source_end);

        if let Some(context) = parts.fragment_contexts.get(*context_index) {
            if context.source_start <= source_cursor {
                let copy_end = source_end.min(context.source_end);
                let copy_len = copy_end - source_cursor;
                let target_start = source_cursor - context.source_start;
                let target_end = target_start.checked_add(copy_len).ok_or_else(|| {
                    crate::error::Error::Output(
                        "internal link fragment range is invalid".to_owned(),
                    )
                })?;
                let target = parts.fragments.get_mut(*context_index).ok_or_else(|| {
                    crate::error::Error::Output(
                        "internal link fragment context is invalid".to_owned(),
                    )
                })?;
                let target_range = target.get_mut(target_start..target_end).ok_or_else(|| {
                    crate::error::Error::Output(
                        "internal link fragment range is invalid".to_owned(),
                    )
                })?;
                target_range.copy_from_slice(
                    &replacement[replacement_cursor..replacement_cursor + copy_len],
                );
                source_cursor = copy_end;
                replacement_cursor += copy_len;
                continue;
            }
        }

        let copy_len = next_end - source_cursor;
        let target_start = source_cursor.checked_sub(*removed_before).ok_or_else(|| {
            crate::error::Error::Output("internal link skeleton range is invalid".to_owned())
        })?;
        let target_end = target_start.checked_add(copy_len).ok_or_else(|| {
            crate::error::Error::Output("internal link skeleton range is invalid".to_owned())
        })?;
        let target = parts
            .skeleton
            .get_mut(target_start..target_end)
            .ok_or_else(|| {
                crate::error::Error::Output("internal link skeleton range is invalid".to_owned())
            })?;
        target.copy_from_slice(&replacement[replacement_cursor..replacement_cursor + copy_len]);
        source_cursor = next_end;
        replacement_cursor += copy_len;
    }
    Ok(())
}

pub(super) fn resolve_section_target(
    section_index: &SectionIndex,
    section_href: &str,
    target_path: &str,
) -> Option<usize> {
    section_index.resolve(section_href, target_path)
}

pub(super) fn split_target(target: &str) -> (&str, Option<&str>) {
    let Some(hash) = target.find('#') else {
        return (target.split('?').next().unwrap_or(target), None);
    };
    let fragment = target[hash + 1..].split('?').next().unwrap_or_default();
    (&target[..hash], (!fragment.is_empty()).then_some(fragment))
}
