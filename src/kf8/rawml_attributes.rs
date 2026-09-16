//! Attribute-level RawML rewriting shared by the link and resource passes.

use crate::css::advance_css_char;
use crate::error::Result;
use crate::xhtml::scan::{
    html_local_name_is, html_raw_text_end, html_tag_end, html_tag_name_range,
};

pub(super) fn rewrite_quoted_attributes(
    source: String,
    attribute_names: &[&str],
    mut replacement: impl FnMut(&str, &str, &str) -> Result<Option<String>>,
) -> Result<String> {
    let mut result = None;
    let bytes = source.as_bytes();
    let mut scan_cursor = 0;
    let mut output_cursor = 0;
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
            let wanted = attribute_names.iter().find_map(|name| {
                let name = name.strip_suffix('=')?;
                source[attribute_start..attribute_end]
                    .eq_ignore_ascii_case(name)
                    .then_some(name)
            });
            let is_object_data = wanted == Some("data")
                && html_local_name_is(&source, tag_name_start, tag_name_end, "object");
            if wanted.is_some() && (wanted != Some("data") || is_object_data) {
                if let Some(value) = replacement(
                    &source,
                    &source[tag_name_start..tag_name_end],
                    &source[value_start..value_end],
                )? {
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
