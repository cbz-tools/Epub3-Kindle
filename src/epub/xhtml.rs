//! Extract semantic information from source EPUB XHTML documents.
//!
//! This includes document styles, writing-mode hints, links, and supported EPUB
//! semantic elements. KF8-specific XHTML rewriting belongs to `kf8::rawml`.

use std::collections::HashSet;
use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::css::{
    validate_kf8_inline_style_with_warnings,
    validate_local_resource_paths as validate_css_local_resource_paths, warn_remote_css_references,
};
use super::navigation::has_token;
use super::opf::{attr, local_name_ref};
pub(super) use super::xhtml_validation::sanitize_unsupported_xhtml;
use crate::WarningCollector;
use crate::book::{Layout, PageProgression, SemanticDocument, Styles, WritingMode};
use crate::css::inline_style_href_with_occupied_hrefs;
use crate::error::{Error, Result};
use crate::xhtml::path::normalize_path_lossy as normalize_path;
use crate::xhtml::path::{is_external_reference, resolve_path};
use crate::xhtml::scan::{
    html_local_name_is_text as html_local_name_is, html_tag_end,
    html_tag_name_range_with_leading_space as html_tag_name_range,
};

fn combined_xhtml_semantics(source: &str) -> Result<(SemanticDocument, Option<WritingMode>)> {
    let mut reader = Reader::from_reader(Cursor::new(source.as_bytes()));
    let mut buffer = Vec::new();
    let mut result = SemanticDocument::default();
    let mut html_mode = None;
    let mut body_mode = None;
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(event) | Event::Empty(event) => {
                let event_name = event.name();
                let name = local_name_ref(event_name.as_ref());
                if name.eq_ignore_ascii_case("body")
                    && attr(&event, "type").is_some_and(|value| has_token(&value, "cover"))
                {
                    result.is_cover = true;
                }
                if name.eq_ignore_ascii_case("img") || name.eq_ignore_ascii_case("image") {
                    if let Some(href) = attr(&event, "src").or_else(|| attr(&event, "href")) {
                        result.image_references.push(href);
                    }
                }
                if name.eq_ignore_ascii_case("html") && html_mode.is_none() {
                    html_mode = root_attribute_writing_mode(&event);
                } else if name.eq_ignore_ascii_case("body") && body_mode.is_none() {
                    body_mode = root_attribute_writing_mode(&event);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok((result, html_mode.or(body_mode)))
}

pub(super) fn validate_local_resource_paths(source: &str, document_href: &str) -> Result<()> {
    let mut reader = Reader::from_reader(Cursor::new(source.as_bytes()));
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(event) | Event::Empty(event) => {
                for attribute in event.attributes() {
                    let attribute = attribute.map_err(|error| {
                        Error::InvalidXhtmlCss(format!(
                            "malformed XHTML resource attribute: {error}"
                        ))
                    })?;
                    let name = local_name_ref(attribute.key.as_ref());
                    if !matches!(name, "href" | "src" | "data") {
                        continue;
                    }
                    let target = attribute.unescape_value().map_err(|error| {
                        Error::InvalidXhtmlCss(format!(
                            "malformed XHTML resource attribute value: {error}"
                        ))
                    })?;
                    let target = target.split(['#', '?']).next().unwrap_or_default();
                    if !target.is_empty()
                        && !is_external_reference(target)
                        && resolve_path(document_href, target).is_none()
                    {
                        return Err(Error::InvalidEpub(format!(
                            "XHTML resource path {target} escapes the EPUB root"
                        )));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(())
}

#[allow(dead_code)]
pub(super) fn parse_xhtml_semantics(source: &str) -> Result<SemanticDocument> {
    combined_xhtml_semantics(source).map(|(semantic, _)| semantic)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewportQuality {
    Canonical,
    Degraded,
}

impl ViewportQuality {
    fn combine(self, other: Self) -> Self {
        if self == Self::Degraded || other == Self::Degraded {
            Self::Degraded
        } else {
            Self::Canonical
        }
    }
}

/// Validate every fixed-page viewport declaration and project a numeric
/// resolution when all declarations agree. A nonnumeric or conflicting
/// declaration suppresses the projection, but does not stop validation of
/// later declarations.
pub(super) fn fixed_page_viewport(source: &str) -> Result<(ViewportQuality, Option<String>)> {
    let mut reader = Reader::from_reader(Cursor::new(source.as_bytes()));
    let mut buffer = Vec::new();
    let mut quality = ViewportQuality::Canonical;
    let mut resolution = None;
    let mut saw_viewport = false;
    let mut resolution_available = true;
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(event) | Event::Empty(event)
                if local_name_ref(event.name().as_ref()).eq_ignore_ascii_case("meta")
                    && attr(&event, "name")
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case("viewport")) =>
            {
                saw_viewport = true;
                quality = quality.combine(validate_viewport_attributes(&event)?);
                let Some(content) = event.attributes().flatten().find_map(|attribute| {
                    (local_name_ref(attribute.key.as_ref()).eq_ignore_ascii_case("content"))
                        .then(|| {
                            attribute
                                .unescape_value()
                                .ok()
                                .map(|value| value.into_owned())
                        })
                        .flatten()
                }) else {
                    resolution_available = false;
                    buffer.clear();
                    continue;
                };
                let Some(candidate) = numeric_viewport_resolution(&content) else {
                    resolution_available = false;
                    buffer.clear();
                    continue;
                };
                if resolution
                    .as_deref()
                    .is_some_and(|existing| existing != candidate)
                {
                    resolution_available = false;
                } else if resolution.is_none() {
                    resolution = Some(candidate);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    let resolution = (saw_viewport && resolution_available)
        .then_some(resolution)
        .flatten();
    Ok((quality, resolution))
}

fn numeric_viewport_resolution(content: &str) -> Option<String> {
    let normalized = content
        .replace('=', " = ")
        .chars()
        .map(|character| {
            if matches!(character, ',' | ';') || character.is_ascii_whitespace() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    if tokens.len() != 6 {
        return None;
    }
    let mut width = None;
    let mut height = None;
    for token in tokens.chunks_exact(3) {
        if token[1] != "=" {
            return None;
        }
        let value = viewport_numeric_value(token[2]);
        match token[0].to_ascii_lowercase().as_str() {
            "width" => width = value,
            "height" => height = value,
            _ => {}
        }
    }
    Some(format!("{}x{}", width?, height?))
}

fn validate_viewport_attributes(event: &BytesStart<'_>) -> Result<ViewportQuality> {
    let name_count = event
        .attributes()
        .flatten()
        .filter(|attribute| local_name_ref(attribute.key.as_ref()).eq_ignore_ascii_case("name"))
        .count();
    if name_count != 1 {
        return Err(viewport_error(
            "viewport must have exactly one name attribute",
        ));
    }
    let content = event.attributes().flatten().find_map(|attribute| {
        (local_name_ref(attribute.key.as_ref()).eq_ignore_ascii_case("content")).then(|| {
            attribute
                .unescape_value()
                .ok()
                .map(|value| value.into_owned())
        })
    });
    let content = content
        .flatten()
        .ok_or_else(|| viewport_error("viewport must have exactly one content attribute"))?;
    let content_count = event
        .attributes()
        .flatten()
        .filter(|attribute| local_name_ref(attribute.key.as_ref()).eq_ignore_ascii_case("content"))
        .count();
    if content_count != 1 {
        return Err(viewport_error(
            "viewport must have exactly one content attribute",
        ));
    }

    let normalized = content
        .replace('=', " = ")
        .chars()
        .map(|character| {
            if matches!(character, ',' | ';') || character.is_ascii_whitespace() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() || tokens.len() % 3 != 0 {
        return Err(viewport_error(
            "viewport content has a malformed dimension token",
        ));
    }
    let mut width_seen = false;
    let mut height_seen = false;
    let mut degraded = false;
    for token in tokens.chunks_exact(3) {
        let key = token[0].to_ascii_lowercase();
        let value = token[2].to_ascii_lowercase();
        if token[1] != "=" || key.is_empty() || value.is_empty() {
            return Err(viewport_error(
                "viewport content has a malformed dimension token",
            ));
        }
        if key == "width" || key == "height" {
            let valid_value = ((key == "width" && value == "device-width")
                || (key == "height" && value == "device-height"))
                || viewport_numeric_value(&value).is_some();
            if !valid_value {
                return Err(viewport_error(
                    "viewport dimensions must be positive numbers or device dimensions",
                ));
            }
            let seen = if key == "width" {
                &mut width_seen
            } else {
                &mut height_seen
            };
            if *seen {
                degraded = true;
            }
            *seen = true;
            degraded = degraded || value == "device-width" || value == "device-height";
        } else {
            // KindleGen accepts additional viewport directives but they are
            // not represented in the numeric fixed-page projection.
            degraded = true;
        }
    }
    if !width_seen || !height_seen {
        degraded = true;
    }
    Ok(if degraded {
        ViewportQuality::Degraded
    } else {
        ViewportQuality::Canonical
    })
}

fn viewport_numeric_value(value: &str) -> Option<u32> {
    let value = value
        .strip_suffix("px")
        .or_else(|| value.strip_suffix("PX"))
        .unwrap_or(value);
    value.parse::<u32>().ok().filter(|value| *value > 0)
}

fn viewport_error(detail: &str) -> Error {
    Error::UnsupportedEpub(format!("unsupported fixed-page viewport: {detail}"))
}

pub(super) fn infer_layout(
    styles: &Styles,
    page_progression: PageProgression,
    primary_writing_mode: Option<WritingMode>,
    document_writing_modes: &[Option<WritingMode>],
) -> Layout {
    let mut layout = Layout {
        page_progression,
        ..Layout::default()
    };
    if let Some(writing_mode) =
        primary_writing_mode.or_else(|| dominant_writing_mode(document_writing_modes))
    {
        layout.writing_mode = writing_mode;
    } else {
        layout.writing_mode = css_fallback_writing_mode(styles);
    }
    layout.direction = styles
        .computed
        .iter()
        .filter(|style| !selector_is_root_only(&style.selector) && style.direction.is_some())
        .find_map(|style| style.direction)
        .or_else(|| styles.computed.iter().find_map(|style| style.direction))
        .unwrap_or_default();
    layout
}

fn css_fallback_writing_mode(styles: &Styles) -> WritingMode {
    // A nested `.vrtl` rule describes the dominant body/title context even
    // when a title or navigation document also supplies an `html.hltr` rule.
    // Root-only html selectors are therefore a fallback, not an override for
    // body-context declarations.
    let writing_mode = styles
        .computed
        .iter()
        .filter(|style| !selector_is_root_only(&style.selector) && style.writing_mode.is_some())
        .find_map(|style| style.writing_mode)
        .or_else(|| styles.computed.iter().find_map(|style| style.writing_mode));
    writing_mode.unwrap_or_default()
}

fn dominant_writing_mode(document_writing_modes: &[Option<WritingMode>]) -> Option<WritingMode> {
    let mut counts = [0usize; 3];
    for writing_mode in document_writing_modes.iter().flatten() {
        counts[writing_mode_index(*writing_mode)] += 1;
    }
    let max = *counts.iter().max()?;
    if max == 0 || counts.iter().filter(|count| **count == max).count() != 1 {
        return None;
    }
    Some(
        match counts.iter().position(|count| *count == max).unwrap() {
            0 => WritingMode::HorizontalTb,
            1 => WritingMode::VerticalRl,
            _ => WritingMode::VerticalLr,
        },
    )
}

fn writing_mode_index(writing_mode: WritingMode) -> usize {
    match writing_mode {
        WritingMode::HorizontalTb => 0,
        WritingMode::VerticalRl => 1,
        WritingMode::VerticalLr => 2,
    }
}

#[allow(dead_code)]
pub(super) fn document_root_writing_mode(source: &str) -> Option<WritingMode> {
    combined_xhtml_semantics(source)
        .ok()
        .and_then(|(_, writing_mode)| writing_mode)
}

pub(super) fn parse_xhtml_semantics_and_document_root_writing_mode(
    source: &str,
) -> Result<(SemanticDocument, Option<WritingMode>)> {
    combined_xhtml_semantics(source)
}

fn root_attribute_writing_mode(event: &BytesStart<'_>) -> Option<WritingMode> {
    let style = attr(event, "style")
        .unwrap_or_default()
        .to_ascii_lowercase();
    style_value(&style, "writing-mode")
        .or_else(|| style_value(&style, "-webkit-writing-mode"))
        .or_else(|| style_value(&style, "-epub-writing-mode"))
        .and_then(|value| parse_writing_mode(&value))
        .or_else(|| {
            attr(event, "class")?.split_whitespace().find_map(|class| {
                match class.to_ascii_lowercase().as_str() {
                    "hltr" => Some(WritingMode::HorizontalTb),
                    "vrtl" => Some(WritingMode::VerticalRl),
                    "vltr" => Some(WritingMode::VerticalLr),
                    _ => None,
                }
            })
        })
}

pub(super) fn is_primary_writing_mode_meta(event: &BytesStart<'_>) -> bool {
    [attr(event, "property"), attr(event, "name")]
        .into_iter()
        .flatten()
        .any(|value| {
            value.split_whitespace().any(|token| {
                token
                    .rsplit(':')
                    .next()
                    .is_some_and(|local| local.eq_ignore_ascii_case("primary-writing-mode"))
            })
        })
}

pub(super) fn parse_writing_mode(value: &str) -> Option<WritingMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "horizontal-tb" | "horizontal-lr" => Some(WritingMode::HorizontalTb),
        "vertical-rl" => Some(WritingMode::VerticalRl),
        "vertical-lr" => Some(WritingMode::VerticalLr),
        _ => None,
    }
}

pub(super) fn selector_is_root_only(selector: &str) -> bool {
    let selectors = selector
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut has_selector = false;
    let all_root_only = selectors.fold(true, |all_root_only, selector| {
        has_selector = true;
        let selector = selector.to_ascii_lowercase();
        let root_only = selector.strip_prefix("html").is_some_and(|remainder| {
            remainder.is_empty()
                || (remainder
                    .chars()
                    .next()
                    .is_some_and(|character| matches!(character, '.' | '#' | ':' | '['))
                    && !remainder.chars().any(|character| {
                        character.is_whitespace() || matches!(character, '>' | '+' | '~')
                    }))
        });
        all_root_only && root_only
    });
    has_selector && all_root_only
}

#[derive(Debug, Clone)]
pub(super) struct DocumentStyle {
    pub(super) reference: String,
    pub(super) resource_href: Option<String>,
    pub(super) inline_source: Option<String>,
}

/// Discover a document's stylesheet inputs in source order. Inline styles get
/// synthetic, document-local hrefs so the existing CSS graph can treat them as
/// ordinary scoped resources without adding a manifest-level resource.
pub(super) fn document_styles_with_occupied_hrefs(
    source: &str,
    document_href: &str,
    document_path: &str,
    occupied_hrefs: &mut HashSet<String>,
    warnings: &mut WarningCollector,
) -> Result<Vec<DocumentStyle>> {
    let mut result = Vec::new();
    let mut seen_links = HashSet::new();
    let mut inline_index = 0usize;
    let mut cursor = 0usize;
    while cursor < source.len() {
        let Some(relative) = source[cursor..].find('<') else {
            break;
        };
        let start = cursor + relative;
        if source[start..].starts_with("<!--") {
            cursor = source[start + 4..]
                .find("-->")
                .map_or(source.len(), |end| start + 4 + end + 3);
            continue;
        }
        let Some(tag_end) = html_tag_end(source, start) else {
            return Err(Error::InvalidXhtmlCss(
                "malformed XHTML tag while scanning CSS resources".to_owned(),
            ));
        };
        let Some((name_start, name_end, closing)) = html_tag_name_range(source, start, tag_end)
        else {
            cursor = tag_end + 1;
            continue;
        };
        if closing {
            cursor = tag_end + 1;
            continue;
        }
        let name = &source[name_start..name_end];
        if let Some(style) = tag_attribute(source, start, tag_end, "style")? {
            validate_kf8_inline_style_with_warnings(&style, warnings)?;
            warn_remote_css_references(&style, warnings)?;
        }
        if html_local_name_is(name, "script") {
            if source[..tag_end].trim_end().ends_with('/') {
                cursor = tag_end + 1;
                continue;
            }
            cursor = raw_text_end(source, tag_end, "script").unwrap_or(source.len());
            continue;
        }
        if html_local_name_is(name, "style") {
            let self_closing = source[..tag_end].trim_end().ends_with('/');
            if self_closing {
                cursor = tag_end + 1;
                continue;
            }
            let Some((close_start, close_end)) = closing_tag(source, tag_end + 1, "style") else {
                return Err(Error::InvalidXhtmlCss(
                    "inline style element has no closing tag".to_owned(),
                ));
            };
            let inline_source = strip_cdata_wrappers(&source[tag_end + 1..close_start]);
            validate_css_local_resource_paths(&inline_source, document_path)?;
            let (resource_href, reference) =
                inline_style_href_with_occupied_hrefs(document_href, inline_index, occupied_hrefs);
            occupied_hrefs.insert(normalize_path(&resource_href));
            result.push(DocumentStyle {
                reference,
                resource_href: Some(resource_href),
                inline_source: Some(inline_source),
            });
            inline_index += 1;
            cursor = close_end + 1;
            continue;
        }
        if html_local_name_is(name, "link")
            && tag_attribute(source, start, tag_end, "rel")?
                .is_some_and(|value| is_stylesheet_rel(&value))
        {
            if let Some(href) = tag_attribute(source, start, tag_end, "href")? {
                if seen_links.insert(href.clone()) {
                    result.push(DocumentStyle {
                        reference: href,
                        resource_href: None,
                        inline_source: None,
                    });
                }
            }
        }
        cursor = tag_end + 1;
    }
    Ok(result)
}

/// Remove XML CDATA delimiters while retaining every byte of their CSS body.
/// This also joins ordinary text and multiple CDATA sections in source order.
fn strip_cdata_wrappers(source: &str) -> String {
    const OPEN: &str = "<![CDATA[";
    const CLOSE: &str = "]]>";

    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative_open) = source[cursor..].find(OPEN) {
        let open = cursor + relative_open;
        result.push_str(&source[cursor..open]);
        let content_start = open + OPEN.len();
        let Some(relative_close) = source[content_start..].find(CLOSE) else {
            result.push_str(&source[open..]);
            return result;
        };
        let close = content_start + relative_close;
        result.push_str(&source[content_start..close]);
        cursor = close + CLOSE.len();
    }
    result.push_str(&source[cursor..]);
    result
}

pub(super) fn unique_resource_id(base_id: &str, occupied_ids: &HashSet<String>) -> String {
    let mut resource_id = base_id.to_owned();
    let mut collision_index = 0usize;
    while occupied_ids.contains(&resource_id) {
        collision_index += 1;
        resource_id = format!("{base_id}-collision-{collision_index:04}");
    }
    resource_id
}

pub(super) fn tag_attribute(
    source: &str,
    start: usize,
    tag_end: usize,
    wanted: &str,
) -> Result<Option<String>> {
    let (_, mut cursor, _) = html_tag_name_range(source, start, tag_end)
        .ok_or_else(|| Error::InvalidXhtmlCss("malformed XHTML tag attributes".to_owned()))?;
    let bytes = source.as_bytes();
    let mut result = None;
    while cursor < tag_end {
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag_end || bytes[cursor] == b'/' {
            break;
        }
        let name_start = cursor;
        while cursor < tag_end
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/' | b'>')
        {
            cursor += source[cursor..].chars().next().map_or(1, char::len_utf8);
        }
        let name_end = cursor;
        if name_start == name_end {
            cursor += 1;
            continue;
        }
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            if source[name_start..name_end].eq_ignore_ascii_case(wanted) {
                return Err(Error::InvalidXhtmlCss(format!(
                    "XHTML {wanted} attribute has no value"
                )));
            }
            while cursor < tag_end && !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b'/'
            {
                cursor += source[cursor..].chars().next().map_or(1, char::len_utf8);
            }
            continue;
        }
        cursor += 1;
        while cursor < tag_end && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let (value_start, value_end) = if matches!(bytes.get(cursor), Some(b'"') | Some(b'\'')) {
            let quote = bytes[cursor];
            let value_start = cursor + 1;
            let Some(relative_end) = source[value_start..tag_end].find(quote as char) else {
                return Err(Error::InvalidXhtmlCss(
                    "XHTML attribute has an unterminated quoted value".to_owned(),
                ));
            };
            let value_end = value_start + relative_end;
            cursor = value_end + 1;
            (value_start, value_end)
        } else {
            let value_start = cursor;
            while cursor < tag_end && !bytes[cursor].is_ascii_whitespace() {
                cursor += source[cursor..].chars().next().map_or(1, char::len_utf8);
            }
            (value_start, cursor)
        };
        if source[name_start..name_end].eq_ignore_ascii_case(wanted) {
            if result.is_some() && wanted.eq_ignore_ascii_case("style") {
                return Err(Error::InvalidXhtmlCss(
                    "XHTML element has duplicate style attributes".to_owned(),
                ));
            }
            result = Some(source[value_start..value_end].to_owned());
        }
    }
    Ok(result)
}

fn is_stylesheet_rel(value: &str) -> bool {
    value
        .split_whitespace()
        .any(|token| token.eq_ignore_ascii_case("stylesheet"))
}

pub(super) fn raw_text_end(source: &str, tag_end: usize, name: &str) -> Option<usize> {
    closing_tag(source, tag_end + 1, name).map(|(_, end)| end + 1)
}

pub(super) fn closing_tag(source: &str, start: usize, name: &str) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = start;
    let mut quote = None;
    let mut block_comment = false;
    let mut line_comment = false;
    let mut html_comment = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if html_comment {
            if byte == b'-'
                && bytes.get(cursor + 1) == Some(&b'-')
                && bytes.get(cursor + 2) == Some(&b'>')
            {
                html_comment = false;
                cursor += 3;
            } else {
                cursor = advance_raw_text_char(source, cursor);
            }
            continue;
        }
        if block_comment {
            if byte == b'*' && bytes.get(cursor + 1) == Some(&b'/') {
                block_comment = false;
                cursor += 2;
            } else {
                cursor = advance_raw_text_char(source, cursor);
            }
            continue;
        }
        if line_comment {
            if byte == b'\r' || byte == b'\n' {
                line_comment = false;
            }
            cursor = advance_raw_text_char(source, cursor);
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                cursor = advance_raw_text_char(source, cursor);
                if cursor < bytes.len() {
                    cursor = advance_raw_text_char(source, cursor);
                }
            } else {
                if byte == delimiter {
                    quote = None;
                }
                cursor = advance_raw_text_char(source, cursor);
            }
            continue;
        }
        if byte == b'<'
            && bytes.get(cursor + 1) == Some(&b'!')
            && bytes.get(cursor + 2) == Some(&b'-')
            && bytes.get(cursor + 3) == Some(&b'-')
        {
            html_comment = true;
            cursor += 4;
            continue;
        }
        if byte == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            block_comment = true;
            cursor += 2;
            continue;
        }
        if name.eq_ignore_ascii_case("script")
            && byte == b'/'
            && bytes.get(cursor + 1) == Some(&b'/')
        {
            line_comment = true;
            cursor += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            cursor = advance_raw_text_char(source, cursor);
            continue;
        }
        if byte == b'<' && bytes.get(cursor + 1) == Some(&b'/') {
            let close_start = cursor;
            let tag_end = html_tag_end(source, close_start)?;
            if let Some((name_start, name_end, closing)) =
                html_tag_name_range(source, close_start, tag_end)
            {
                if closing && html_local_name_is(&source[name_start..name_end], name) {
                    return Some((close_start, tag_end));
                }
            }
            cursor = tag_end + 1;
            continue;
        }
        cursor = advance_raw_text_char(source, cursor);
    }
    None
}

fn advance_raw_text_char(source: &str, cursor: usize) -> usize {
    source
        .get(cursor..)
        .and_then(|remaining| remaining.chars().next())
        .map_or(source.len(), |character| cursor + character.len_utf8())
}

fn style_value(style: &str, property: &str) -> Option<String> {
    style.split(';').find_map(|part| {
        let (name, value) = part.split_once(':')?;
        (name.trim() == property).then(|| value.trim().to_owned())
    })
}
