//! CSS syntax primitives shared by EPUB discovery and KF8 transport.
//!
//! Policy and validation stay with their owning stages. This module contains
//! only lossless lexical helpers and reference/path mechanics used by both
//! stages.

use std::collections::HashSet;

use crate::xhtml::path::{is_external_reference, normalize_path};

pub(crate) const SYNTHETIC_INLINE_CSS_PROPERTY: &str = "__synthetic_inline_css";

pub(crate) fn traverse_css_dependencies<F>(
    roots: impl IntoIterator<Item = (String, String)>,
    mut load: F,
) -> Vec<String>
where
    F: FnMut(&str, &str) -> Option<(String, String, Vec<String>)>,
{
    fn visit<F>(
        base_href: &str,
        reference: &str,
        load: &mut F,
        visited: &mut HashSet<String>,
        result: &mut Vec<String>,
    ) where
        F: FnMut(&str, &str) -> Option<(String, String, Vec<String>)>,
    {
        let Some((resolved, import_base_href, imports)) = load(base_href, reference) else {
            return;
        };
        if !visited.insert(resolved.clone()) {
            return;
        }
        for import in imports {
            visit(&import_base_href, &import, load, visited, result);
        }
        result.push(resolved);
    }

    let mut visited = HashSet::new();
    let mut result = Vec::new();
    for (base_href, reference) in roots {
        visit(&base_href, &reference, &mut load, &mut visited, &mut result);
    }
    result
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CssImportSpan {
    pub(crate) statement_start: usize,
    pub(crate) wrapper_start: usize,
    pub(crate) wrapper_end: usize,
    pub(crate) target_start: usize,
    pub(crate) target_end: usize,
    pub(crate) scan_end: usize,
}

pub(crate) fn css_import_targets(source: &str) -> Vec<String> {
    css_import_spans(source)
        .into_iter()
        .map(|span| source[span.target_start..span.target_end].to_owned())
        .filter(|target| !is_external_reference(target))
        .collect()
}

pub(crate) fn is_remote_reference(target: &str) -> bool {
    target.starts_with("//")
        || (target.contains("://")
            && !target.starts_with("data:")
            && !target.starts_with("kindle:"))
}

pub(crate) fn css_import_spans(source: &str) -> Vec<CssImportSpan> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            cursor = skip_css_comment(source, cursor).unwrap_or(bytes.len());
            continue;
        }
        if matches!(bytes[cursor], b'\'' | b'\"') {
            cursor = skip_css_string(source, cursor).unwrap_or(bytes.len());
            continue;
        }
        if css_keyword_at(source, cursor, "@import") {
            if let Some(span) = parse_css_import_span(source, cursor + "@import".len()) {
                cursor = span.scan_end;
                spans.push(span);
                continue;
            }
        }
        cursor = advance_css_char(source, cursor);
    }
    spans
}

fn parse_css_import_span(source: &str, start: usize) -> Option<CssImportSpan> {
    let (cursor, _) = skip_css_space_comments(source, start)?;
    if css_function_at(source, cursor, "url") && source.as_bytes().get(cursor + 3) == Some(&b'(') {
        let url = parse_css_url_span(source, cursor)?;
        return Some(CssImportSpan {
            statement_start: start - "@import".len(),
            wrapper_start: cursor,
            wrapper_end: url.close_end,
            target_start: url.target_start,
            target_end: url.target_end,
            scan_end: url.close_end,
        });
    }
    let quote = *source.as_bytes().get(cursor)?;
    if !matches!(quote, b'\'' | b'\"') {
        return None;
    }
    let quote_end = skip_css_string(source, cursor)?;
    Some(CssImportSpan {
        statement_start: start - "@import".len(),
        wrapper_start: cursor,
        wrapper_end: quote_end,
        target_start: cursor + 1,
        target_end: quote_end.checked_sub(1)?,
        scan_end: quote_end,
    })
}

#[derive(Debug, Clone, Copy)]
struct CssUrlSpan {
    target_start: usize,
    target_end: usize,
    close_end: usize,
}

fn parse_css_url_span(source: &str, start: usize) -> Option<CssUrlSpan> {
    let open = start.checked_add(3)?;
    if source.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let (mut cursor, _) = skip_css_space_comments(source, open + 1)?;
    if source
        .as_bytes()
        .get(cursor)
        .is_some_and(|byte| *byte == b'\'' || *byte == b'\"')
    {
        let quote_end = skip_css_string(source, cursor)?;
        let target_start = cursor + 1;
        let target_end = quote_end.checked_sub(1)?;
        let (close, _) = skip_css_space_comments(source, quote_end)?;
        if source.as_bytes().get(close) != Some(&b')') {
            return None;
        }
        return Some(CssUrlSpan {
            target_start,
            target_end,
            close_end: close + 1,
        });
    }

    let target_start = cursor;
    while cursor < source.len() {
        let byte = source.as_bytes()[cursor];
        if byte == b')'
            || byte.is_ascii_whitespace()
            || (byte == b'/' && source.as_bytes().get(cursor + 1) == Some(&b'*'))
        {
            break;
        }
        cursor = advance_css_char(source, cursor);
    }
    let target_end = cursor;
    if target_end == target_start {
        return None;
    }
    let (close, _) = skip_css_space_comments(source, cursor)?;
    if source.as_bytes().get(close) != Some(&b')') {
        return None;
    }
    Some(CssUrlSpan {
        target_start,
        target_end,
        close_end: close + 1,
    })
}

pub(crate) fn skip_css_space_comments(source: &str, mut cursor: usize) -> Option<(usize, bool)> {
    let mut skipped = false;
    loop {
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
            skipped = true;
        }
        if source.as_bytes().get(cursor) == Some(&b'/')
            && source.as_bytes().get(cursor + 1) == Some(&b'*')
        {
            skipped = true;
            cursor = skip_css_comment(source, cursor)?;
            continue;
        }
        return Some((cursor, skipped));
    }
}

pub(crate) fn skip_css_comment(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'/') || bytes.get(start + 1) != Some(&b'*') {
        return None;
    }
    let mut cursor = start + 2;
    while cursor < bytes.len() {
        if bytes[cursor] == b'*' && bytes.get(cursor + 1) == Some(&b'/') {
            return Some(cursor + 2);
        }
        cursor = advance_css_char(source, cursor);
    }
    None
}

pub(crate) fn skip_css_string(source: &str, start: usize) -> Option<usize> {
    let quote = *source.as_bytes().get(start)?;
    if !matches!(quote, b'\'' | b'\"') {
        return None;
    }
    let mut cursor = start + 1;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            byte if byte == quote => return Some(cursor + 1),
            b'\\' => {
                cursor = advance_css_char(source, cursor);
                if cursor < source.len() {
                    cursor = advance_css_char(source, cursor);
                }
            }
            _ => cursor = advance_css_char(source, cursor),
        }
    }
    None
}

pub(crate) fn advance_css_char(source: &str, cursor: usize) -> usize {
    source
        .get(cursor..)
        .and_then(|remaining| remaining.chars().next())
        .map_or(source.len(), |character| cursor + character.len_utf8())
}

fn css_keyword_at(source: &str, start: usize, keyword: &str) -> bool {
    let bytes = source.as_bytes();
    let end = match start.checked_add(keyword.len()) {
        Some(end) if end <= bytes.len() => end,
        _ => return false,
    };
    if !bytes[start..end]
        .iter()
        .zip(keyword.bytes())
        .all(|(actual, expected)| actual.eq_ignore_ascii_case(&expected))
    {
        return false;
    }
    source
        .get(end..)
        .and_then(|remaining| remaining.chars().next())
        .is_none_or(|character| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
}

pub(crate) fn css_function_at(source: &str, start: usize, name: &str) -> bool {
    css_keyword_at(source, start, name) && css_identifier_boundary_before(source, start)
}

fn css_identifier_boundary_before(source: &str, start: usize) -> bool {
    source
        .get(..start)
        .and_then(|prefix| prefix.chars().next_back())
        .is_none_or(|character| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
}

pub(crate) fn inline_style_href(document_href: &str, index: usize) -> (String, String) {
    let document_token = inline_document_token(document_href);
    let resource_href = format!("__inline_css__/doc-{document_token}-style-{index:04}.css");
    let reference = relative_resource_reference(document_href, &resource_href);
    (resource_href, reference)
}

pub(crate) fn inline_style_href_with_occupied_hrefs(
    document_href: &str,
    index: usize,
    occupied_hrefs: &HashSet<String>,
) -> (String, String) {
    let document_token = inline_document_token(document_href);
    let stem = format!("__inline_css__/doc-{document_token}-style-{index:04}");
    let mut resource_href = format!("{stem}.css");
    let mut collision_index = 0usize;
    while occupied_hrefs.contains(&normalize_path(&resource_href).unwrap_or_default()) {
        collision_index += 1;
        resource_href = format!("{stem}-collision-{collision_index:04}.css");
    }
    let reference = relative_resource_reference(document_href, &resource_href);
    (resource_href, reference)
}

fn inline_document_token(document_href: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let normalized = normalize_path(document_href).unwrap_or_default();
    if normalized.is_empty() {
        return "root".to_owned();
    }
    let mut token = String::with_capacity(normalized.len() * 2);
    for byte in normalized.bytes() {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    token
}

fn relative_resource_reference(document_href: &str, resource_href: &str) -> String {
    let normalized = normalize_path(document_href).unwrap_or_default();
    let directory = normalized
        .rsplit_once('/')
        .map(|(directory, _)| directory)
        .unwrap_or_default();
    let parent_prefix = "../".repeat(directory.split('/').filter(|part| !part.is_empty()).count());
    format!("{parent_prefix}{resource_href}")
}
