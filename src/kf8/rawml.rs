//! Prepare Kindle section XHTML for use as KF8 RawML.
//!
//! This boundary owns layout projection, stylesheet and asset references,
//! internal links, and AID-related preparation. MOBI serialization, resource
//! geometry, TBS, and the PositionMap coordinate model remain elsewhere.

use super::css_flow::{
    CssResourceIndex, ResourceIndex, SectionIndex, css_flow_number, resource_reference,
    rewrite_css_urls,
};
use super::format::to_base32_fixed;
use super::fragmentize::{FragmentContext, body_range, fragmentize_body};
use super::position::{self, PositionMap};
use super::rawml_attributes::rewrite_quoted_attributes;
use crate::css::advance_css_char;
use crate::error::Result;
use crate::kindle::{KindleSection, project_inline_style_for_kindle};
use crate::xhtml::path::is_external_reference;
use crate::xhtml::scan::{
    find_ascii_case_insensitive, html_local_name_is, html_raw_text_end, html_tag_end,
    html_tag_name_range,
};

fn generated_section_path(index: usize) -> String {
    format!("Text/part{index:04}.xhtml")
}

#[derive(Debug, Clone)]
pub(super) struct PendingInternalLink {
    section_index: usize,
    fragment: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SectionParts {
    pub(crate) skeleton: Vec<u8>,
    pub(crate) fragments: Vec<Vec<u8>>,
    pub(crate) fragment_contexts: Vec<FragmentContext>,
    pub(crate) insertion_offset: u32,
}

const FRAGMENT_TARGET_SIZE: usize = 8192;
const POSFID_PLACEHOLDER: &str = "kindle:pos:fid:ZZZZ:off:ZZZZZZZZZZ";
const COVER_LANDMARK_MARKER: &str = "kindle:cover-landmark";

pub(super) fn rewrite_projected_attributes(
    source: String,
    section_href: &str,
    cover_resource_id: Option<&str>,
    resources: &ResourceIndex<'_>,
) -> Result<String> {
    let mut result = None;
    let bytes = source.as_bytes();
    let mut scan_cursor = 0;
    let mut output_cursor = 0;
    let mut cover_reference = None;
    while scan_cursor < source.len() {
        let Some(relative) = source[scan_cursor..].find('<') else {
            break;
        };
        let tag_start = scan_cursor + relative;
        if source[tag_start..].starts_with("<!--") {
            scan_cursor = source[tag_start + 4..]
                .find("-->")
                .map_or(source.len(), |end| tag_start + 4 + end + 3);
            continue;
        }
        let Some(tag_end) = html_tag_end(&source, tag_start) else {
            break;
        };
        if let Some(raw_end) = html_raw_text_end(&source, tag_start, tag_end) {
            scan_cursor = raw_end;
            continue;
        }
        let Some((tag_name_start, tag_name_end, closing)) =
            html_tag_name_range(&source, tag_start, tag_end)
        else {
            scan_cursor = tag_end + 1;
            continue;
        };
        let mut cursor = tag_name_end;
        if closing {
            scan_cursor = tag_end + 1;
            continue;
        }
        while cursor < tag_end {
            while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor >= tag_end || bytes[cursor] == b'/' {
                break;
            }
            let attribute_start = cursor;
            while cursor < tag_end
                && !bytes[cursor].is_ascii_whitespace()
                && !matches!(bytes[cursor], b'=' | b'/' | b'>')
            {
                cursor = advance_css_char(&source, cursor);
            }
            let attribute_end = cursor;
            while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if attribute_start == attribute_end || bytes.get(cursor) != Some(&b'=') {
                continue;
            }
            cursor += 1;
            while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let Some(&quote) = bytes.get(cursor) else {
                break;
            };
            if !matches!(quote, b'"' | b'\'') {
                while cursor < tag_end
                    && !bytes[cursor].is_ascii_whitespace()
                    && !matches!(bytes[cursor], b'/' | b'>')
                {
                    cursor = advance_css_char(&source, cursor);
                }
                continue;
            }
            let value_start = cursor + 1;
            let Some(value_end_relative) = source[value_start..tag_end].find(quote as char) else {
                scan_cursor = source.len();
                break;
            };
            let value_end = value_start + value_end_relative;
            let attribute_name = &source[attribute_start..attribute_end];
            let is_object_data = attribute_name.eq_ignore_ascii_case("data")
                && html_local_name_is(&source, tag_name_start, tag_name_end, "object");
            let is_svg_href = attribute_name.eq_ignore_ascii_case("href")
                && (html_local_name_is(&source, tag_name_start, tag_name_end, "image")
                    || html_local_name_is(&source, tag_name_start, tag_name_end, "use"));
            let is_asset = attribute_name.eq_ignore_ascii_case("src")
                || attribute_name.eq_ignore_ascii_case("xlink:href")
                || is_object_data
                || is_svg_href;
            let target = &source[value_start..value_end];
            let replacement = if is_asset {
                resource_reference(section_href, target, resources)
            } else if attribute_name.eq_ignore_ascii_case("style") {
                let projected = project_inline_style_for_kindle(target);
                let rewritten = rewrite_css_urls(&projected, section_href, resources);
                (rewritten != target).then_some(rewritten)
            } else if attribute_name.eq_ignore_ascii_case("href") && target == COVER_LANDMARK_MARKER
            {
                if cover_reference.is_none() {
                    let cover = cover_resource_id
                        .and_then(|id| resources.by_id(id))
                        .ok_or_else(|| {
                            crate::error::Error::Output(
                                "cover landmark has no serialized native cover resource".to_owned(),
                            )
                        })?;
                    cover_reference = Some(
                        resource_reference(section_href, &cover.href, resources).ok_or_else(
                            || {
                                crate::error::Error::Output(
                                    "cover landmark cannot resolve the serialized native cover resource"
                                        .to_owned(),
                                )
                            },
                        )?,
                    );
                }
                cover_reference.clone()
            } else {
                None
            };
            if let Some(value) = replacement {
                if value != target {
                    let output = result
                        .get_or_insert_with(|| String::with_capacity(source.len() + value.len()));
                    output.push_str(&source[output_cursor..value_start]);
                    output.push_str(&value);
                    output.push(quote as char);
                    output_cursor = value_end + 1;
                }
            }
            cursor = value_end + 1;
        }
        if scan_cursor == source.len() {
            break;
        }
        scan_cursor = tag_end + 1;
    }
    if let Some(mut result) = result {
        result.push_str(&source[output_cursor..]);
        Ok(result)
    } else {
        Ok(source)
    }
}

pub(super) fn generated_aid(index: usize) -> String {
    to_base32_unpadded(u32::try_from(index).expect("generated section index fits in u32"))
}

fn to_base32_unpadded(mut value: u32) -> String {
    const DIGITS: &[u8; 32] = b"0123456789ABCDEFGHIJKLMNOPQRSTUV";
    let mut digits = Vec::new();
    while value != 0 {
        digits.push(DIGITS[(value % 32) as usize]);
        value /= 32;
    }
    if digits.is_empty() {
        digits.push(b'0');
    }
    digits.reverse();
    String::from_utf8(digits).expect("base32 alphabet is ASCII")
}

/// Materialize ordered-list semantics for the KF8 HTML projection.
///
/// KF8 readers do not consistently reconstruct EPUB list ordinals from CSS or
/// the HTML `start`/`value` attributes. Emit the effective ordinal on every
/// direct `li` child of each `ol`, while leaving existing `li@value` spelling
/// and all unrelated markup untouched. The element stack makes nested lists
/// independent and prevents `li` elements in `ul` or other containers from
/// being mistaken for ordered-list children.
pub(super) fn materialize_ordered_list_values(source: String) -> Result<String> {
    let mut output: Option<String> = None;
    let mut output_cursor = 0usize;
    let mut scan_cursor = 0usize;
    let mut elements = Vec::new();
    let bytes = source.as_bytes();

    while let Some(relative) = source[scan_cursor..].find('<') {
        let tag_start = scan_cursor + relative;
        if source[tag_start..].starts_with("<!--") {
            scan_cursor = source[tag_start + 4..]
                .find("-->")
                .map_or(source.len(), |end| tag_start + 4 + end + 3);
            continue;
        }
        let Some(tag_end) = html_tag_end(&source, tag_start) else {
            break;
        };
        if let Some(raw_end) = html_raw_text_end(&source, tag_start, tag_end) {
            scan_cursor = raw_end;
            continue;
        }
        let Some((name_start, name_end, closing)) =
            html_tag_name_range(&source, tag_start, tag_end)
        else {
            scan_cursor = tag_end + 1;
            continue;
        };
        let local_name = source[name_start..name_end]
            .rsplit(':')
            .next()
            .unwrap_or(&source[name_start..name_end]);
        let tag = &bytes[tag_start..=tag_end];

        if closing {
            if let Some(index) = elements
                .iter()
                .rposition(|element: &ElementFrame| element.name.eq_ignore_ascii_case(local_name))
            {
                elements.truncate(index);
            }
        } else {
            let direct_ordered_parent = elements
                .last_mut()
                .filter(|element| element.name.eq_ignore_ascii_case("ol"));
            if local_name.eq_ignore_ascii_case("li") {
                if let Some(parent) = direct_ordered_parent {
                    let existing = attribute_value_range(tag, b"value")
                        .and_then(|(start, end)| std::str::from_utf8(&tag[start..end]).ok())
                        .and_then(|value| value.trim().parse::<i64>().ok());
                    let ordinal = existing.unwrap_or(parent.next_ordinal);
                    parent.next_ordinal = ordinal.checked_add(1).ok_or_else(|| {
                        crate::error::Error::Output("ordered-list ordinal exceeds i64".to_owned())
                    })?;
                    if existing.is_none() {
                        let value = ordinal.to_string();
                        let replacement = append_attribute(tag, b"value", value.as_bytes());
                        let rendered =
                            output.get_or_insert_with(|| String::with_capacity(source.len() + 16));
                        rendered.push_str(&source[output_cursor..tag_start]);
                        rendered.push_str(std::str::from_utf8(&replacement).map_err(|_| {
                            crate::error::Error::Output(
                                "generated ordered-list markup is not valid UTF-8".to_owned(),
                            )
                        })?);
                        output_cursor = tag_end + 1;
                    }
                }
            }
            if local_name.eq_ignore_ascii_case("ol") {
                let start = attribute_value_range(tag, b"start")
                    .and_then(|(start, end)| std::str::from_utf8(&tag[start..end]).ok())
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .unwrap_or(1);
                elements.push(ElementFrame {
                    name: local_name.to_owned(),
                    next_ordinal: start,
                });
            } else if !is_self_closing(tag) {
                elements.push(ElementFrame {
                    name: local_name.to_owned(),
                    next_ordinal: 0,
                });
            }
        }
        scan_cursor = tag_end + 1;
    }

    if let Some(mut output) = output {
        output.push_str(&source[output_cursor..]);
        Ok(output)
    } else {
        Ok(source)
    }
}

#[derive(Debug)]
struct ElementFrame {
    name: String,
    next_ordinal: i64,
}

fn is_self_closing(tag: &[u8]) -> bool {
    let mut cursor = tag.len();
    while cursor > 0 && tag[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    cursor >= 2 && tag[cursor - 2] == b'/'
}

fn attribute_value_range(tag: &[u8], wanted: &[u8]) -> Option<(usize, usize)> {
    let mut cursor = 1usize;
    while cursor < tag.len() {
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || matches!(tag[cursor], b'>' | b'/') {
            break;
        }
        let name_start = cursor;
        while cursor < tag.len()
            && !tag[cursor].is_ascii_whitespace()
            && !matches!(tag[cursor], b'=' | b'>' | b'/')
        {
            cursor += 1;
        }
        let name_end = cursor;
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || tag[cursor] != b'=' {
            cursor = name_end.saturating_add(1);
            continue;
        }
        cursor += 1;
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let value_start = cursor;
        let value_end = match tag.get(cursor) {
            Some(b'"') | Some(b'\'') => {
                let quote = tag[cursor];
                cursor += 1;
                let start = cursor;
                while cursor < tag.len() && tag[cursor] != quote {
                    cursor += 1;
                }
                let end = cursor;
                if cursor < tag.len() {
                    cursor += 1;
                }
                (start, end)
            }
            Some(_) => {
                while cursor < tag.len()
                    && !tag[cursor].is_ascii_whitespace()
                    && tag[cursor] != b'>'
                {
                    cursor += 1;
                }
                (value_start, cursor)
            }
            None => break,
        };
        if tag[name_start..name_end].eq_ignore_ascii_case(wanted) {
            return Some(value_end);
        }
    }
    None
}

fn append_attribute(tag: &[u8], name: &[u8], value: &[u8]) -> Vec<u8> {
    let insert = if tag.ends_with(b"/>") {
        tag.len() - 2
    } else {
        tag.len() - 1
    };
    let mut output = Vec::with_capacity(tag.len() + name.len() + value.len() + 5);
    output.extend_from_slice(&tag[..insert]);
    output.push(b' ');
    output.extend_from_slice(name);
    output.extend_from_slice(b"=\"");
    output.extend_from_slice(value);
    output.push(b'"');
    output.extend_from_slice(&tag[insert..]);
    output
}

/// Lower one effective pre-paginated spine item to the KF8 page-flow shape.
///
/// A page presentation SVG is a secondary flow; the section that remains in
/// main RawML contains only the flow reference. This is the A–F C-05..C-12 /
/// E-14 / F-09 boundary. The caller owns flow numbering and supplies the
/// already rewritten resource references.
pub(super) fn lower_pre_paginated_section(
    source: &str,
    flow_reference: &str,
    css_reference: Option<&str>,
) -> Option<(String, Vec<u8>)> {
    if let Some((start, end)) = svg_element_range(source) {
        let svg = &source[start..end];
        let page_flow = page_svg_flow(svg, css_reference);
        let mut main = String::with_capacity(source.len() + flow_reference.len() + 16);
        main.push_str(&source[..start]);
        main.push_str("<img src=\"");
        main.push_str(flow_reference);
        main.push_str("\"/>");
        main.push_str(&source[end..]);
        return Some((main, page_flow.into_bytes()));
    }

    // Some fixed-layout EPUBs use a body-level image instead of an SVG
    // wrapper. Preserve that page semantic by constructing the same minimal
    // presentation SVG around its already lowered kindle:embed reference.
    let (start, end) = first_body_image_range(source)?;
    let image_tag = &source[start..end];
    let image_reference = quoted_attribute_value(image_tag, "src")
        .or_else(|| quoted_attribute_value(image_tag, "xlink:href"))?;
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100%" height="100%"><image width="100%" height="100%" xlink:href="{image_reference}"/></svg>"#
    );
    let page_flow = page_svg_flow(&svg, css_reference);
    let mut main = String::with_capacity(source.len() + flow_reference.len() + 16);
    main.push_str(&source[..start]);
    main.push_str("<img src=\"");
    main.push_str(flow_reference);
    main.push_str("\"/>");
    main.push_str(&source[end..]);
    Some((main, page_flow.into_bytes()))
}

fn page_svg_flow(svg: &str, css_reference: Option<&str>) -> String {
    let stylesheet = css_reference
        .map(|reference| format!(r#"<?xml-stylesheet href="{reference}" type="text/css" ?>"#))
        .unwrap_or_default();
    format!("{stylesheet}{svg}")
}

fn svg_element_range(source: &str) -> Option<(usize, usize)> {
    let start = find_ascii_case_insensitive(source, "<svg", 0)?;
    let open_end = html_tag_end(source, start)?;
    let close_start = find_ascii_case_insensitive(source, "</svg", open_end + 1)?;
    let close_end = html_tag_end(source, close_start)?.checked_add(1)?;
    Some((start, close_end))
}

fn first_body_image_range(source: &str) -> Option<(usize, usize)> {
    let body_start = find_ascii_case_insensitive(source, "<body", 0)?;
    let body_open_end = html_tag_end(source, body_start)?;
    let image_start = find_ascii_case_insensitive(source, "<img", body_open_end + 1)?;
    let image_end = html_tag_end(source, image_start)?.checked_add(1)?;
    Some((image_start, image_end))
}

fn quoted_attribute_value(tag: &str, wanted: &str) -> Option<String> {
    let (_, mut cursor, closing) = html_tag_name_range(tag, 0, tag.len().checked_sub(1)?)?;
    if closing {
        return None;
    }
    let bytes = tag.as_bytes();
    while cursor < tag.len() {
        while cursor < tag.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || bytes[cursor] == b'/' {
            break;
        }
        let name_start = cursor;
        while cursor < tag.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/' | b'>')
        {
            cursor = advance_css_char(tag, cursor);
        }
        let name_end = cursor;
        while cursor < tag.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while cursor < tag.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let quote = *bytes.get(cursor)?;
        if !matches!(quote, b'"' | b'\'') {
            return None;
        }
        let value_start = cursor + 1;
        let value_end = value_start + tag[value_start..].find(quote as char)?;
        if tag[name_start..name_end].eq_ignore_ascii_case(wanted) {
            return Some(tag[value_start..value_end].to_owned());
        }
        cursor = value_end + 1;
    }
    None
}

pub(super) fn rewrite_body_aid(source: String, aid: &str) -> String {
    let Some(body_start) = find_ascii_case_insensitive(&source, "<body", 0) else {
        return source;
    };
    let Some(body_end_relative) = source[body_start..].find('>') else {
        return source;
    };
    let body_end = body_start + body_end_relative;
    let tag = &source[body_start..=body_end];
    let Some(aid_relative) = find_html_attribute(tag, "aid") else {
        let insert_at = if source[body_start..=body_end].ends_with("/>") {
            body_end - 1
        } else {
            body_end
        };
        let mut result = String::with_capacity(source.len() + aid.len() + 7);
        result.push_str(&source[..insert_at]);
        result.push_str(" aid=\"");
        result.push_str(aid);
        result.push('"');
        result.push_str(&source[insert_at..]);
        return result;
    };
    let value_start = body_start + aid_relative + 4;
    let Some(quote) = source.as_bytes().get(value_start).copied() else {
        return source;
    };
    if !matches!(quote, b'"' | b'\'') {
        return source;
    }
    let content_start = value_start + 1;
    let Some(content_end_relative) = source[content_start..].find(quote as char) else {
        return source;
    };
    let content_end = content_start + content_end_relative;
    let mut result = String::with_capacity(source.len() + aid.len());
    result.push_str(&source[..content_start]);
    result.push_str(aid);
    result.push_str(&source[content_end..]);
    result
}

pub(super) fn find_html_attribute(tag: &str, name: &str) -> Option<usize> {
    let mut cursor = 0;
    while let Some(position) = find_ascii_case_insensitive(tag, name, cursor) {
        if (position == 0
            || tag
                .as_bytes()
                .get(position - 1)
                .is_some_and(|byte| byte.is_ascii_whitespace()))
            && tag.as_bytes().get(position + name.len()) == Some(&b'=')
        {
            return Some(position);
        }
        cursor = position + name.len();
    }
    None
}

pub(super) fn rewrite_internal_links(
    source: String,
    section_href: &str,
    section_index: &SectionIndex,
    section_number: usize,
    anchor_indices: &[position::AnchorIndex],
    css_resources: &CssResourceIndex<'_>,
) -> Result<(String, Vec<PendingInternalLink>)> {
    if anchor_indices.len() != section_index.len() || section_number >= anchor_indices.len() {
        return Err(crate::error::Error::Output(
            "internal link anchor indexes do not match sections".to_owned(),
        ));
    }
    let mut pending = Vec::new();
    let rewritten = rewrite_quoted_attributes(source, &["href="], |_source, tag_name, target| {
        let tag_name = tag_name.rsplit(':').next().unwrap_or(tag_name);
        if tag_name.eq_ignore_ascii_case("image") || tag_name.eq_ignore_ascii_case("use") {
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
        Ok(Some(POSFID_PLACEHOLDER.to_owned()))
    })?;
    Ok((rewritten, pending))
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

pub(super) fn split_section_parts(source: &str) -> Result<SectionParts> {
    let (body_open_start, body_start, body_end) = body_range(source).ok_or_else(|| {
        crate::error::Error::Output("generated XHTML is missing a body element".to_owned())
    })?;
    if body_end < body_start {
        return Err(crate::error::Error::Output(
            "generated XHTML body range is invalid".to_owned(),
        ));
    }
    let body = source.as_bytes().get(body_start..body_end).ok_or_else(|| {
        crate::error::Error::Output("generated XHTML fragment is out of bounds".to_owned())
    })?;
    let body_aid = source
        .get(body_open_start..body_start)
        .and_then(|opening| attribute_value(opening.as_bytes(), "aid"));
    let fragmented = fragmentize_body(body, body_aid.as_deref(), FRAGMENT_TARGET_SIZE);
    let contexts = fragmented
        .contexts
        .into_iter()
        .map(|context| FragmentContext {
            selector: context.selector,
            source_start: body_start + context.source_start,
            source_end: body_start + context.source_end,
            skeleton_offset: body_start + context.skeleton_offset,
            starts_tags: context.starts_tags,
            ends_tags: context.ends_tags,
        })
        .collect();
    Ok(SectionParts {
        skeleton: {
            let mut result =
                Vec::with_capacity(source.len() - body.len() + fragmented.skeleton.len());
            result.extend_from_slice(source.as_bytes().get(..body_start).ok_or_else(|| {
                crate::error::Error::Output(
                    "generated XHTML body start is out of bounds".to_owned(),
                )
            })?);
            result.extend_from_slice(&fragmented.skeleton);
            result.extend_from_slice(source.as_bytes().get(body_end..).ok_or_else(|| {
                crate::error::Error::Output("generated XHTML body end is out of bounds".to_owned())
            })?);
            result
        },
        fragments: fragmented
            .fragments
            .into_iter()
            .map(|fragment| fragment.raw)
            .collect(),
        fragment_contexts: contexts,
        insertion_offset: u32::try_from(body_start).map_err(|_| {
            crate::error::Error::Output("fragment insertion position exceeds u32".to_owned())
        })?,
    })
}

fn attribute_value(tag: &[u8], name: &str) -> Option<String> {
    let name = name.as_bytes();
    let mut cursor = 1;
    while cursor < tag.len() {
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || tag[cursor] == b'>' {
            break;
        }
        let start = cursor;
        while cursor < tag.len()
            && !tag[cursor].is_ascii_whitespace()
            && !matches!(tag[cursor], b'=' | b'>')
        {
            cursor += 1;
        }
        let attribute = &tag[start..cursor];
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || tag[cursor] != b'=' {
            continue;
        }
        cursor += 1;
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let quote = *tag.get(cursor)?;
        if !matches!(quote, b'"' | b'\'') {
            return None;
        }
        let value_start = cursor + 1;
        let value_end = value_start
            + tag
                .get(value_start..)?
                .iter()
                .position(|byte| *byte == quote)?;
        if attribute.eq_ignore_ascii_case(name) {
            return String::from_utf8(tag[value_start..value_end].to_vec()).ok();
        }
        cursor = value_end + 1;
    }
    None
}
