//! XHTML transport validation and unsupported-feature rejection.
use quick_xml::escape::unescape;

use super::xhtml::{closing_tag, raw_text_end};
use crate::error::{Error, Result};
use crate::xhtml::scan::{
    html_tag_end, html_tag_name_range_with_leading_space as html_tag_name_range,
};
use crate::{WarningCode, WarningCollector};

const UNWRAPPED_UNSUPPORTED_ELEMENTS: &[&str] = &[
    "audio", "button", "datalist", "fieldset", "form", "frame", "iframe", "input", "label",
    "legend", "object", "optgroup", "option", "output", "picture", "select", "textarea", "video",
];
const DROPPED_UNSUPPORTED_ELEMENTS: &[&str] = &["embed", "param", "source"];

/// Remove unsupported XHTML behavior while retaining ordinary text and
/// fallback descendants. This is deliberately a raw-tag pass so script/style
/// payloads and quoted tag attributes are not interpreted as markup.
pub(super) fn sanitize_unsupported_xhtml(
    source: &str,
    warnings: &mut WarningCollector,
) -> Result<String> {
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut mathml_depth = 0usize;
    while cursor < source.len() {
        let Some(relative) = source[cursor..].find('<') else {
            result.push_str(&source[cursor..]);
            break;
        };
        let start = cursor + relative;
        result.push_str(&source[cursor..start]);
        if source[start..].starts_with("<!--") {
            let Some(relative_end) = source[start + 4..].find("-->") else {
                return Err(Error::InvalidXhtmlCss("malformed XHTML comment".to_owned()));
            };
            let end = start + 4 + relative_end + 3;
            result.push_str(&source[start..end]);
            cursor = end;
            continue;
        }
        let Some(tag_end) = html_tag_end(source, start) else {
            return Err(Error::InvalidXhtmlCss("malformed XHTML tag".to_owned()));
        };
        let Some((name_start, name_end, closing)) = html_tag_name_range(source, start, tag_end)
        else {
            result.push_str(&source[start..=tag_end]);
            cursor = tag_end + 1;
            continue;
        };
        let name = &source[name_start..name_end];
        let local_name = name.rsplit(':').next().unwrap_or(name);
        let self_closing = source[..tag_end].trim_end().ends_with('/');

        if !closing && local_name.eq_ignore_ascii_case("script") {
            warnings.add_category_once(
                WarningCode::W001,
                "executable scripting and interactive XHTML semantics were removed",
            );
            cursor = if self_closing {
                tag_end + 1
            } else {
                raw_text_end(source, tag_end, "script").ok_or_else(|| {
                    Error::InvalidXhtmlCss("script element has no closing tag".to_owned())
                })?
            };
            continue;
        }

        if mathml_depth > 0 {
            if closing && local_name.eq_ignore_ascii_case("math") {
                mathml_depth -= 1;
            }
            cursor = tag_end + 1;
            continue;
        }
        if local_name.eq_ignore_ascii_case("math") {
            warnings.add_category_once(
                WarningCode::W005,
                "MathML markup was reduced to readable descendant content",
            );
            if !closing && !self_closing {
                mathml_depth = 1;
            }
            cursor = tag_end + 1;
            continue;
        }

        if !closing && local_name.eq_ignore_ascii_case("style") {
            result.push_str(&sanitize_xhtml_tag(source, start, tag_end, name, warnings)?);
            if !self_closing {
                let end = raw_text_end(source, tag_end, "style").ok_or_else(|| {
                    Error::InvalidXhtmlCss("style element has no closing tag".to_owned())
                })?;
                result.push_str(&source[tag_end + 1..end]);
                cursor = end;
            } else {
                cursor = tag_end + 1;
            }
            continue;
        }

        if DROPPED_UNSUPPORTED_ELEMENTS
            .iter()
            .any(|candidate| local_name.eq_ignore_ascii_case(candidate))
        {
            warnings.add_category_once(
                WarningCode::W002,
                "unsupported rich-media candidates were dropped while preserving fallback content",
            );
            cursor = if closing || self_closing {
                tag_end + 1
            } else {
                closing_tag(source, tag_end + 1, name)
                    .map(|(_, end)| end + 1)
                    .ok_or_else(|| {
                        Error::InvalidXhtmlCss(format!(
                            "unsupported {} element has no closing tag",
                            local_name
                        ))
                    })?
            };
            continue;
        }

        if UNWRAPPED_UNSUPPORTED_ELEMENTS
            .iter()
            .any(|candidate| local_name.eq_ignore_ascii_case(candidate))
        {
            let code = if matches!(local_name.to_ascii_lowercase().as_str(), "audio" | "video") {
                WarningCode::W003
            } else if matches!(
                local_name.to_ascii_lowercase().as_str(),
                "object" | "picture"
            ) {
                WarningCode::W002
            } else {
                WarningCode::W001
            };
            let message = match code {
                WarningCode::W003 => {
                    "audio/video playback semantics were dropped while preserving fallback content"
                }
                WarningCode::W002 => {
                    "object/picture wrapper semantics were reduced to fallback content"
                }
                _ => {
                    "interactive form/control semantics were removed while preserving readable content"
                }
            };
            warnings.add_category_once(code, message);
            cursor = tag_end + 1;
            continue;
        }

        result.push_str(&sanitize_xhtml_tag(source, start, tag_end, name, warnings)?);
        cursor = tag_end + 1;
    }
    Ok(result)
}

fn sanitize_xhtml_tag(
    source: &str,
    start: usize,
    tag_end: usize,
    element_name: &str,
    warnings: &mut WarningCollector,
) -> Result<String> {
    let (_, mut cursor, _) = html_tag_name_range(source, start, tag_end)
        .ok_or_else(|| Error::InvalidXhtmlCss("malformed XHTML tag name".to_owned()))?;
    let mut result = String::with_capacity(tag_end + 1 - start);
    result.push_str(&source[start..cursor]);
    let local_name = element_name.rsplit(':').next().unwrap_or(element_name);
    let bytes = source.as_bytes();
    while cursor < tag_end {
        let whitespace_start = cursor;
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        result.push_str(&source[whitespace_start..cursor]);
        if cursor >= tag_end {
            break;
        }
        if bytes[cursor] == b'/' {
            result.push('/');
            cursor += 1;
            continue;
        }
        let attribute_start = cursor;
        while cursor < tag_end
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/' | b'>')
        {
            cursor = advance_xhtml_char(source, cursor);
        }
        if attribute_start == cursor {
            result.push(bytes[cursor] as char);
            cursor += 1;
            continue;
        }
        let attribute_end = cursor;
        let mut value_range = None;
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if matches!(bytes.get(cursor), Some(b'"') | Some(b'\'')) {
                let quote = bytes[cursor];
                let value_start = cursor + 1;
                cursor += 1;
                while cursor < tag_end && bytes[cursor] != quote {
                    cursor = advance_xhtml_char(source, cursor);
                }
                value_range = Some((value_start, cursor));
                if cursor < tag_end {
                    cursor += 1;
                }
            } else {
                let value_start = cursor;
                while cursor < tag_end && !bytes[cursor].is_ascii_whitespace() {
                    cursor = advance_xhtml_char(source, cursor);
                }
                value_range = Some((value_start, cursor));
            }
        }
        let attribute_name = &source[attribute_start..attribute_end];
        let local_attribute = attribute_name.rsplit(':').next().unwrap_or(attribute_name);
        let is_event_handler = local_attribute.len() > 2
            && local_attribute
                .get(..2)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("on"));
        let is_srcset = matches!(local_name.to_ascii_lowercase().as_str(), "img" | "source")
            && local_attribute.eq_ignore_ascii_case("srcset");
        let is_executable_url = value_range.is_some_and(|(value_start, value_end)| {
            is_url_attribute(local_attribute) && is_javascript_url(&source[value_start..value_end])
        });
        if is_event_handler || is_executable_url {
            warnings.add_category_once(
                WarningCode::W001,
                "executable scripting and interactive XHTML semantics were removed",
            );
        } else if is_srcset {
            warnings.add_category_once(
                WarningCode::W002,
                "responsive image candidates were dropped while preserving the ordinary image source",
            );
        } else {
            result.push_str(&source[attribute_start..cursor]);
        }
    }
    result.push('>');
    Ok(result)
}

fn is_url_attribute(attribute_name: &str) -> bool {
    matches!(
        attribute_name.to_ascii_lowercase().as_str(),
        "href" | "src" | "action" | "formaction" | "data"
    )
}

fn is_javascript_url(value: &str) -> bool {
    let value = unescape(value)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| value.to_owned());
    let value = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && !character.is_ascii_control())
        .collect::<String>();
    value
        .get(.."javascript:".len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("javascript:"))
}

fn advance_xhtml_char(source: &str, cursor: usize) -> usize {
    source[cursor..]
        .chars()
        .next()
        .map_or(source.len(), |character| cursor + character.len_utf8())
}
