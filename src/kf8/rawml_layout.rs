//! Pre-paginated page-flow projection for KF8 RawML.

use super::fragmentize::body_range;
use crate::book::plain_display_text;
use crate::css::advance_css_char;
use crate::xhtml::scan::{find_ascii_case_insensitive, html_tag_end, html_tag_name_range};

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
    is_svg_document: bool,
    page_viewport: Option<&str>,
) -> Option<(String, Vec<u8>)> {
    // A direct SVG spine document is source content, not by itself evidence
    // of a Kindle fixed-page canvas. Only its explicit XHTML viewport can
    // authorize projecting it to a page presentation flow.
    if is_svg_document && page_viewport.is_none() {
        return None;
    }

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
    if let Some((start, end)) = first_body_image_range(source) {
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
        return Some((main, page_flow.into_bytes()));
    }

    // Explicit XHTML viewport dimensions are the only evidence used to emit
    // a page flow for a blank fixed-layout page. Do not infer a canvas from
    // source images, SVG geometry, or CSS dimensions.
    let (width, height) = page_viewport?.split_once('x')?;
    let width = width.parse::<u32>().ok().filter(|value| *value > 0)?;
    let height = height.parse::<u32>().ok().filter(|value| *value > 0)?;
    let (_, body_start, body_end) = body_range(source)?;
    let body = source.get(body_start..body_end)?;
    if !plain_display_text(body).is_empty() {
        return None;
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
    );
    let page_flow = page_svg_flow(&svg, css_reference);
    let mut main = String::with_capacity(source.len() + flow_reference.len() + 16);
    main.push_str(&source[..body_end]);
    main.push_str("<img src=\"");
    main.push_str(flow_reference);
    main.push_str("\"/>");
    main.push_str(&source[body_end..]);
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
