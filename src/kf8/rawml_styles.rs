//! RawML stylesheet and inline-style transport rewriting.

use std::collections::HashSet;

use super::css_flow::{CssResourceIndex, css_flow_number, stylesheet_flow_reference};
use crate::css::{advance_css_char, inline_style_href};
use crate::xhtml::scan::{
    advance_char, html_local_name_is, html_raw_text_end, html_tag_end, html_tag_name_range,
};

fn preserved_style_attributes(source: &str, start: usize, tag_end: usize) -> String {
    let Some((_, mut cursor, closing)) = html_tag_name_range(source, start, tag_end) else {
        return String::new();
    };
    if closing {
        return String::new();
    }

    let bytes = source.as_bytes();
    let allowed = ["media", "title", "type"];
    let mut attributes = Vec::new();
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
            cursor = advance_char(source, cursor);
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
        let Some(&value_start_byte) = bytes.get(cursor) else {
            break;
        };
        let (value_start, value_end, quote) = if matches!(value_start_byte, b'"' | b'\'') {
            let value_start = cursor + 1;
            let Some(relative_end) = source[value_start..tag_end].find(value_start_byte as char)
            else {
                return String::new();
            };
            let value_end = value_start + relative_end;
            cursor = value_end + 1;
            (value_start, value_end, value_start_byte)
        } else {
            let value_start = cursor;
            while cursor < tag_end && !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b'>'
            {
                cursor = advance_char(source, cursor);
            }
            let value_end = if cursor > value_start
                && bytes[cursor - 1] == b'/'
                && source[cursor..tag_end].trim().is_empty()
            {
                cursor - 1
            } else {
                cursor
            };
            (value_start, value_end, b'"')
        };
        if let Some(name) = allowed
            .iter()
            .find(|name| html_local_name_is(source, attribute_start, attribute_end, name))
        {
            attributes.push((*name, &source[value_start..value_end], quote));
        }
    }

    let mut result = String::new();
    for (name, value, quote) in attributes {
        result.push(' ');
        result.push_str(name);
        result.push('=');
        result.push(quote as char);
        result.push_str(value);
        result.push(quote as char);
    }
    result
}

fn stylesheet_link_href(source: &str, start: usize, tag_end: usize) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let (name_start, name_end, closing) = html_tag_name_range(source, start, tag_end)?;
    if closing || !html_local_name_is(source, name_start, name_end, "link") {
        return None;
    }
    let mut cursor = name_end;
    let mut has_stylesheet_rel = false;
    let mut href = None;
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
            cursor = advance_char(source, cursor);
        }
        let attribute_end = cursor;
        if attribute_start == attribute_end {
            cursor = advance_char(source, cursor);
            continue;
        }
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let attribute_name = &source[attribute_start..attribute_end];
        let quote = *bytes.get(cursor)?;
        if !matches!(quote, b'"' | b'\'') {
            let value_start = cursor;
            while cursor < tag_end && !bytes[cursor].is_ascii_whitespace() {
                cursor = advance_char(source, cursor);
            }
            if attribute_name.eq_ignore_ascii_case("rel") {
                has_stylesheet_rel = is_stylesheet_rel(&source[value_start..cursor]);
            } else if attribute_name.eq_ignore_ascii_case("href") {
                // HTML permits an unquoted attribute value. The value span
                // remains source-relative so only a resolved stylesheet link
                // is replaced and all surrounding bytes stay untouched.
                href = Some((value_start, cursor));
            }
            continue;
        }
        let value_start = cursor + 1;
        let value_end = value_start + source[value_start..tag_end].find(quote as char)?;
        if attribute_name.eq_ignore_ascii_case("rel") {
            has_stylesheet_rel = is_stylesheet_rel(&source[value_start..value_end]);
        } else if attribute_name.eq_ignore_ascii_case("href") {
            href = Some((value_start, value_end));
        }
        cursor = value_end + 1;
    }
    has_stylesheet_rel.then_some(href).flatten()
}

fn is_stylesheet_rel(value: &str) -> bool {
    value
        .split_whitespace()
        .any(|token| token.eq_ignore_ascii_case("stylesheet"))
}

pub(super) fn rewrite_stylesheet_links_with_references(
    source: String,
    section_href: &str,
    css_resources: &CssResourceIndex<'_>,
    referenced_styles: Option<&[String]>,
) -> String {
    let mut result = None;
    let mut scan_cursor = 0;
    let mut output_cursor = 0;
    let mut inline_style_index = 0usize;
    let mut referenced_style_index = 0usize;
    let mut seen_links = HashSet::new();
    while scan_cursor < source.len() {
        if source.as_bytes()[scan_cursor] != b'<' {
            scan_cursor = advance_css_char(&source, scan_cursor);
            continue;
        }
        if source[scan_cursor..].starts_with("<!--") {
            scan_cursor = source[scan_cursor + 4..]
                .find("-->")
                .map_or(source.len(), |relative| scan_cursor + 4 + relative + 3);
            continue;
        }
        let Some(tag_end) = html_tag_end(&source, scan_cursor) else {
            break;
        };
        if let Some((name_start, name_end, closing)) =
            html_tag_name_range(&source, scan_cursor, tag_end)
        {
            let self_closing = source[..tag_end].trim_end().ends_with('/');
            if !closing
                && !self_closing
                && html_local_name_is(&source, name_start, name_end, "style")
            {
                if let Some(raw_end) = html_raw_text_end(&source, scan_cursor, tag_end) {
                    let inline_reference = referenced_styles
                        .and_then(|styles| styles.get(referenced_style_index).cloned())
                        .unwrap_or_else(|| inline_style_href(section_href, inline_style_index).1);
                    inline_style_index += 1;
                    if referenced_styles.is_some() {
                        referenced_style_index += 1;
                    }
                    if let Some(flow_number) =
                        css_flow_number(section_href, &inline_reference, css_resources)
                    {
                        let mut replacement = format!(
                            "<link rel=\"stylesheet\" href=\"{}\"",
                            stylesheet_flow_reference(flow_number)
                        );
                        replacement.push_str(&preserved_style_attributes(
                            &source,
                            scan_cursor,
                            tag_end,
                        ));
                        replacement.push_str("/>");
                        let output = result.get_or_insert_with(|| {
                            String::with_capacity(source.len() + replacement.len())
                        });
                        output.push_str(&source[output_cursor..scan_cursor]);
                        output.push_str(&replacement);
                        output_cursor = raw_end;
                    }
                    scan_cursor = raw_end;
                    continue;
                }
            }
        }
        if let Some(raw_end) = html_raw_text_end(&source, scan_cursor, tag_end) {
            // HTML script/style contents are raw text. Do not interpret a
            // literal <link ...> example inside either element as markup.
            scan_cursor = raw_end;
            continue;
        }
        let Some((href_start, href_end)) = stylesheet_link_href(&source, scan_cursor, tag_end)
        else {
            scan_cursor = tag_end + 1;
            continue;
        };
        let value = &source[href_start..href_end];
        let path = value.split(['#', '?']).next().unwrap_or(value);
        if referenced_styles.is_some() && seen_links.insert(value.to_owned()) {
            referenced_style_index += 1;
        }
        if let Some(flow_number) = css_flow_number(section_href, path, css_resources) {
            // Only an actual stylesheet link is rewritten. Its document
            // scope and source order are transport contracts; unrelated
            // href attributes must remain byte-for-byte unchanged. Resource
            // resolution, not the filename suffix, identifies CSS here.
            let replacement = stylesheet_flow_reference(flow_number);
            let output = result
                .get_or_insert_with(|| String::with_capacity(source.len() + replacement.len()));
            output.push_str(&source[output_cursor..href_start]);
            output.push_str(&replacement);
            output_cursor = href_end;
        }
        scan_cursor = tag_end + 1;
    }
    if let Some(mut result) = result {
        result.push_str(&source[output_cursor..]);
        result
    } else {
        source
    }
}
