//! Materialize supported inline image data before KF8 projection.
//!
//! This stays in the Kindle normalization boundary so CSS and XHTML both feed
//! the existing Kindle resource index. KF8 code only sees ordinary local
//! resource references and never needs to decode a data URI.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use super::{KindleResource, KindleSection};
use crate::css::{
    SYNTHETIC_INLINE_CSS_PROPERTY, advance_css_char, css_function_at, skip_css_comment,
    skip_css_string,
};
use crate::xhtml::path::{normalize_path, resolve_path};
use crate::xhtml::scan::{html_tag_end, html_tag_name_range};

const MAX_INLINE_IMAGE_BYTES: usize = 30_000_000;
const MAX_INLINE_IMAGE_DIMENSION: u32 = 16_384;
const MAX_INLINE_IMAGE_PIXELS: u64 = 16_777_216;
const MAX_INLINE_IMAGE_DECODED_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn materialize_data_images(
    sections: &mut [KindleSection],
    resources: &mut Vec<KindleResource>,
) {
    let synthetic_css_bases = synthetic_css_bases(sections, resources);
    let mut occupied_hrefs = resources
        .iter()
        .filter_map(|resource| normalize_path(&resource.href))
        .collect::<HashSet<_>>();
    let mut occupied_ids = resources
        .iter()
        .map(|resource| resource.id.clone())
        .collect::<HashSet<_>>();
    let mut synthetic_by_payload = HashMap::<(String, Vec<u8>), String>::new();

    for section in sections {
        section.source_xhtml = rewrite_xhtml(
            &section.source_xhtml,
            &section.href,
            resources,
            &mut synthetic_by_payload,
            &mut occupied_hrefs,
            &mut occupied_ids,
        );
    }
    for index in 0..resources.len() {
        if !resources[index].media_type.eq_ignore_ascii_case("text/css") {
            continue;
        }
        let source = String::from_utf8_lossy(&resources[index].data).into_owned();
        let href = resources[index].href.clone();
        let base_href = normalize_path(&href)
            .and_then(|href| synthetic_css_bases.get(&href))
            .map(String::as_str)
            .unwrap_or(&href);
        let rewritten = rewrite_css(
            &source,
            base_href,
            resources,
            &mut synthetic_by_payload,
            &mut occupied_hrefs,
            &mut occupied_ids,
        );
        if let Some(rewritten) = rewritten {
            resources[index].data = rewritten.into_bytes();
        }
    }
}

fn synthetic_css_bases(
    sections: &[KindleSection],
    resources: &[KindleResource],
) -> HashMap<String, String> {
    let synthetic_hrefs = resources
        .iter()
        .filter(|resource| {
            resource.media_type.eq_ignore_ascii_case("text/css")
                && resource
                    .properties
                    .iter()
                    .any(|property| property == SYNTHETIC_INLINE_CSS_PROPERTY)
        })
        .filter_map(|resource| normalize_path(&resource.href))
        .collect::<HashSet<_>>();
    let mut bases = HashMap::new();
    for section in sections {
        for reference in &section.referenced_styles {
            let Some(href) = resolve_path(&section.href, reference) else {
                continue;
            };
            if synthetic_hrefs.contains(&href) {
                bases.entry(href).or_insert_with(|| section.href.clone());
            }
        }
    }
    bases
}

fn rewrite_xhtml(
    source: &str,
    base_href: &str,
    resources: &mut Vec<KindleResource>,
    synthetic_by_payload: &mut HashMap<(String, Vec<u8>), String>,
    occupied_hrefs: &mut HashSet<String>,
    occupied_ids: &mut HashSet<String>,
) -> String {
    let mut result = None;
    let mut scan_cursor = 0;
    let mut output_cursor = 0;
    while scan_cursor < source.len() {
        let Some(relative) = source[scan_cursor..].find('<') else {
            break;
        };
        let tag_start = scan_cursor + relative;
        let Some(tag_end) = html_tag_end(source, tag_start) else {
            break;
        };
        let Some((name_start, name_end, closing)) = html_tag_name_range(source, tag_start, tag_end)
        else {
            scan_cursor = tag_end + 1;
            continue;
        };
        if closing {
            scan_cursor = tag_end + 1;
            continue;
        }
        let tag_name = source[name_start..name_end]
            .rsplit(':')
            .next()
            .unwrap_or_default();
        let mut cursor = name_end;
        while cursor < tag_end {
            while cursor < tag_end && source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor >= tag_end || source.as_bytes()[cursor] == b'/' {
                break;
            }
            let attribute_start = cursor;
            while cursor < tag_end
                && !source.as_bytes()[cursor].is_ascii_whitespace()
                && !matches!(source.as_bytes()[cursor], b'=' | b'/' | b'>')
            {
                cursor = advance_css_char(source, cursor);
            }
            let attribute_end = cursor;
            while cursor < tag_end && source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if attribute_start == attribute_end || source.as_bytes().get(cursor) != Some(&b'=') {
                continue;
            }
            cursor += 1;
            while cursor < tag_end && source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let Some(&quote) = source.as_bytes().get(cursor) else {
                break;
            };
            if !matches!(quote, b'"' | b'\'') {
                while cursor < tag_end
                    && !source.as_bytes()[cursor].is_ascii_whitespace()
                    && !matches!(source.as_bytes()[cursor], b'/' | b'>')
                {
                    cursor = advance_css_char(source, cursor);
                }
                continue;
            }
            let value_start = cursor + 1;
            let Some(value_end_relative) = source[value_start..tag_end].find(quote as char) else {
                break;
            };
            let value_end = value_start + value_end_relative;
            let attribute = &source[attribute_start..attribute_end];
            let target = &source[value_start..value_end];
            let replacement = if attribute.eq_ignore_ascii_case("style") {
                rewrite_css(
                    target,
                    base_href,
                    resources,
                    synthetic_by_payload,
                    occupied_hrefs,
                    occupied_ids,
                )
            } else if attribute.eq_ignore_ascii_case("src") && tag_name.eq_ignore_ascii_case("img")
            {
                rewrite_data_reference(
                    target,
                    base_href,
                    resources,
                    synthetic_by_payload,
                    occupied_hrefs,
                    occupied_ids,
                )
            } else {
                None
            };
            if let Some(replacement) = replacement {
                if replacement != target {
                    let output = result.get_or_insert_with(|| String::with_capacity(source.len()));
                    output.push_str(&source[output_cursor..value_start]);
                    output.push_str(&replacement);
                    output.push(quote as char);
                    output_cursor = value_end + 1;
                }
            }
            cursor = value_end + 1;
        }
        scan_cursor = tag_end + 1;
    }
    result.map_or_else(
        || source.to_owned(),
        |mut value| {
            value.push_str(&source[output_cursor..]);
            value
        },
    )
}

fn rewrite_data_reference(
    target: &str,
    base_href: &str,
    resources: &mut Vec<KindleResource>,
    synthetic_by_payload: &mut HashMap<(String, Vec<u8>), String>,
    occupied_hrefs: &mut HashSet<String>,
    occupied_ids: &mut HashSet<String>,
) -> Option<String> {
    if !is_data_uri(target) {
        return None;
    }
    let Some(href) = materialize_image(
        target,
        resources,
        synthetic_by_payload,
        occupied_hrefs,
        occupied_ids,
    ) else {
        // Invalid inline data is a non-fatal unsupported resource. Emptying
        // the reference follows the existing safe-degrade policy.
        return Some(String::new());
    };
    Some(relative_reference(base_href, &href))
}

fn rewrite_css(
    source: &str,
    base_href: &str,
    resources: &mut Vec<KindleResource>,
    synthetic_by_payload: &mut HashMap<(String, Vec<u8>), String>,
    occupied_hrefs: &mut HashSet<String>,
    occupied_ids: &mut HashSet<String>,
) -> Option<String> {
    let mut result = None;
    let mut cursor = 0;
    let mut output_cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor] == b'/' && source.as_bytes().get(cursor + 1) == Some(&b'*') {
            cursor = skip_css_comment(source, cursor).unwrap_or(source.len());
            continue;
        }
        if matches!(source.as_bytes()[cursor], b'\'' | b'"') {
            cursor = skip_css_string(source, cursor).unwrap_or(source.len());
            continue;
        }
        if css_function_at(source, cursor, "url")
            && source.as_bytes().get(cursor + 3) == Some(&b'(')
        {
            let Some((target_start, target_end, close_end)) = parse_url(source, cursor) else {
                cursor = advance_css_char(source, cursor);
                continue;
            };
            let target = &source[target_start..target_end];
            if is_data_uri(target) {
                let replacement = materialize_image(
                    target,
                    resources,
                    synthetic_by_payload,
                    occupied_hrefs,
                    occupied_ids,
                )
                .map(|href| relative_reference(base_href, &href))
                .unwrap_or_default();
                let output = result.get_or_insert_with(|| String::with_capacity(source.len()));
                output.push_str(&source[output_cursor..target_start]);
                output.push_str(&replacement);
                output_cursor = target_end;
            }
            cursor = close_end;
            continue;
        }
        cursor = advance_css_char(source, cursor);
    }
    result.map(|mut value| {
        value.push_str(&source[output_cursor..]);
        value
    })
}

fn parse_url(source: &str, start: usize) -> Option<(usize, usize, usize)> {
    let mut cursor = start + 4;
    while source
        .as_bytes()
        .get(cursor)
        .is_some_and(|byte| is_css_whitespace(*byte))
    {
        cursor += 1;
    }
    let (target_start, target_end, quote) =
        if matches!(source.as_bytes().get(cursor), Some(b'\'' | b'"')) {
            let quote = source.as_bytes()[cursor] as char;
            let target_start = cursor + 1;
            let target_end = source[target_start..]
                .find(quote)
                .map(|offset| target_start + offset)?;
            (target_start, target_end, Some(quote))
        } else {
            let target_start = cursor;
            let mut target_end = source[target_start..]
                .find(')')
                .map(|offset| target_start + offset)?;
            while target_end > target_start && is_css_whitespace(source.as_bytes()[target_end - 1])
            {
                target_end -= 1;
            }
            (target_start, target_end, None)
        };
    let mut close = target_end + usize::from(quote.is_some());
    while source
        .as_bytes()
        .get(close)
        .is_some_and(|byte| is_css_whitespace(*byte))
    {
        close += 1;
    }
    (source.as_bytes().get(close) == Some(&b')')).then_some((target_start, target_end, close + 1))
}

fn is_css_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r')
}

fn materialize_image(
    uri: &str,
    resources: &mut Vec<KindleResource>,
    synthetic_by_payload: &mut HashMap<(String, Vec<u8>), String>,
    occupied_hrefs: &mut HashSet<String>,
    occupied_ids: &mut HashSet<String>,
) -> Option<String> {
    let (media_type, data) = decode_image_data_uri(uri)?;
    if let Some(resource) = resources
        .iter()
        .find(|resource| resource.media_type == media_type && resource.data == data)
    {
        return Some(resource.href.clone());
    }
    let key = (media_type.clone(), data.clone());
    if let Some(href) = synthetic_by_payload.get(&key) {
        return Some(href.clone());
    }
    let digest = fnv1a64(&data);
    let extension = media_type.rsplit('/').next().unwrap_or("bin");
    let stem = format!("__data_uri__/image-{digest:016x}");
    let mut href = format!("{stem}.{extension}");
    let mut suffix = 0usize;
    loop {
        let normalized_href = normalize_path(&href)?;
        if occupied_hrefs.insert(normalized_href.clone()) {
            href = normalized_href;
            break;
        }
        suffix += 1;
        href = format!("{stem}-{suffix:04}.{extension}");
    }
    let id_stem = format!("__data_uri_image_{digest:016x}");
    let mut id = id_stem.clone();
    suffix = 0;
    while !occupied_ids.insert(id.clone()) {
        suffix += 1;
        id = format!("{id_stem}_{suffix:04}");
    }
    resources.push(KindleResource {
        id,
        href: href.clone(),
        media_type: media_type.clone(),
        properties: Vec::new(),
        data,
    });
    synthetic_by_payload.insert(key, href.clone());
    Some(href)
}

fn decode_image_data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
    let (metadata, payload) = uri
        .strip_prefix_case_insensitive("data:")?
        .split_once(',')?;
    let mut parts = metadata.split(';');
    let media_type = parts.next()?.trim().to_ascii_lowercase();
    if !matches!(media_type.as_str(), "image/png" | "image/jpeg") {
        return None;
    }
    if !parts.any(|part| part.trim().eq_ignore_ascii_case("base64")) {
        return None;
    }
    let data = decode_base64_strict(payload)?;
    if data.is_empty() || data.len() > MAX_INLINE_IMAGE_BYTES {
        return None;
    }
    let format = image::guess_format(&data).ok()?;
    validate_and_decode_image(&data, format)?;
    let matches = (media_type == "image/png" && format == image::ImageFormat::Png)
        || (media_type == "image/jpeg" && format == image::ImageFormat::Jpeg);
    matches.then_some((media_type, data))
}

fn validate_and_decode_image(data: &[u8], format: image::ImageFormat) -> Option<()> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_INLINE_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_INLINE_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_INLINE_IMAGE_DECODED_BYTES);
    let mut dimensions_reader = image::ImageReader::with_format(Cursor::new(data), format);
    dimensions_reader.limits(limits.clone());
    let (width, height) = dimensions_reader.into_dimensions().ok()?;
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if pixels > MAX_INLINE_IMAGE_PIXELS {
        return None;
    }

    let mut reader = image::ImageReader::with_format(Cursor::new(data), format);
    reader.limits(limits);
    reader.decode().ok().map(|_| ())
}

fn decode_base64_strict(payload: &str) -> Option<Vec<u8>> {
    let bytes = payload.as_bytes();
    if bytes.is_empty()
        || bytes.len() % 4 != 0
        || bytes.iter().any(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 || bytes[..bytes.len() - padding].contains(&b'=') {
        return None;
    }
    let decoded_len = bytes.len() / 4 * 3 - padding;
    if decoded_len == 0 || decoded_len > MAX_INLINE_IMAGE_BYTES {
        return None;
    }
    let mut output = Vec::with_capacity(decoded_len);
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let final_chunk = index + 1 == bytes.len() / 4;
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            0
        } else {
            base64_value(chunk[2])?
        };
        let d = if chunk[3] == b'=' {
            0
        } else {
            base64_value(chunk[3])?
        };
        if !final_chunk && (chunk[2] == b'=' || chunk[3] == b'=') {
            return None;
        }
        if chunk[2] == b'=' && chunk[3] != b'=' {
            return None;
        }
        if chunk[2] == b'=' && b & 0x0f != 0 {
            return None;
        }
        if chunk[3] == b'=' && chunk[2] != b'=' && c & 0x03 != 0 {
            return None;
        }
        output.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            output.push((b << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            output.push((c << 6) | d);
        }
    }
    (output.len() == decoded_len).then_some(output)
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn relative_reference(base_href: &str, target_href: &str) -> String {
    let parent_depth = base_href
        .rsplit_once('/')
        .map(|(parent, _)| parent.split('/').filter(|part| !part.is_empty()).count())
        .unwrap_or(0);
    format!("{}{}", "../".repeat(parent_depth), target_href)
}

fn is_data_uri(value: &str) -> bool {
    value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

trait StripPrefixCaseInsensitive {
    fn strip_prefix_case_insensitive(&self, prefix: &str) -> Option<&str>;
}

impl StripPrefixCaseInsensitive for str {
    fn strip_prefix_case_insensitive(&self, prefix: &str) -> Option<&str> {
        self.get(..prefix.len())
            .filter(|value| value.eq_ignore_ascii_case(prefix))
            .map(|_| &self[prefix.len()..])
    }
}
