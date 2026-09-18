//! Prepare Kindle section XHTML for use as KF8 RawML.
//!
//! This boundary owns section preparation and fragment coordination. The
//! focused child modules handle links, structural projection, and page layout.

use super::css_flow::{ResourceIndex, resource_reference, rewrite_css_urls};
use super::fragmentize::{FragmentContext, body_range, fragmentize_body};
use crate::css::advance_css_char;
use crate::error::Result;
use crate::kindle::project_inline_style_for_kindle;
use crate::xhtml::scan::{
    html_local_name_is, html_raw_text_end, html_tag_end, html_tag_name_range,
};

pub(super) use super::rawml_layout::lower_pre_paginated_section;
pub(super) use super::rawml_links::{
    PendingInternalLink, materialize_internal_links, rewrite_internal_links,
};
pub(super) use super::rawml_structure::{
    generated_aid, materialize_ordered_list_values, rewrite_body_aid,
};

#[derive(Debug, Clone)]
pub(crate) struct SectionParts {
    pub(crate) skeleton: Vec<u8>,
    pub(crate) fragments: Vec<Vec<u8>>,
    pub(crate) fragment_contexts: Vec<FragmentContext>,
    pub(crate) insertion_offset: u32,
}

const FRAGMENT_TARGET_SIZE: usize = 8192;
pub(super) const POSFID_PLACEHOLDER: &str = "kindle:pos:fid:ZZZZ:off:ZZZZZZZZZZ";
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
