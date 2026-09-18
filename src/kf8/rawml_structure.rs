//! Structural HTML projection for KF8 RawML.

use crate::error::Result;
use crate::xhtml::scan::{
    find_ascii_case_insensitive, html_raw_text_end, html_tag_end, html_tag_name_range,
};

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
