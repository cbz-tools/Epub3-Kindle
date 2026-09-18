//! Plan CSS dependencies and project them into KF8 CSS flows.
//!
//! The module owns stylesheet ordering, `@import` traversal, asset URL
//! rewriting, and flow references. XHTML semantic parsing and KF8 record
//! serialization remain outside this boundary.

use std::collections::HashMap;

use super::format::{to_base32, to_base32_fixed};
use super::resource::{is_binary_resource, is_css_resource, is_font_resource, is_image_resource};
use crate::css::{
    SYNTHETIC_INLINE_CSS_PROPERTY, advance_css_char, css_function_at, css_import_spans,
    css_import_targets, is_remote_reference, skip_css_comment, skip_css_space_comments,
    skip_css_string, traverse_css_dependencies,
};
use crate::kindle::{KindleResource as Resource, KindleSection};
use crate::xhtml::path::{normalize_path, resolve_path};

#[derive(Debug)]
pub(crate) struct ResourceIndex<'a> {
    resources: &'a [Resource],
    by_id: HashMap<String, usize>,
    by_href: HashMap<String, Vec<usize>>,
    binary_by_href: HashMap<String, usize>,
    binary_resources: Vec<usize>,
    first_image_by_binary: Vec<Option<usize>>,
}

impl<'a> ResourceIndex<'a> {
    pub(crate) fn new(resources: &'a [Resource]) -> Self {
        let mut by_id = HashMap::new();
        let mut by_href: HashMap<String, Vec<usize>> = HashMap::new();
        let mut binary_by_href = HashMap::new();
        let mut binary_resources = Vec::new();
        let mut first_image_by_binary = Vec::new();
        let mut first_image = None;
        for (index, resource) in resources.iter().enumerate() {
            by_id.entry(resource.id.clone()).or_insert(index);
            if let Some(href) = normalize_path(&resource.href) {
                by_href.entry(href).or_default().push(index);
            }
            if is_binary_resource(resource) {
                let binary_index = binary_resources.len();
                if let Some(href) = normalize_path(&resource.href) {
                    binary_by_href.entry(href).or_insert(binary_index);
                }
                binary_resources.push(index);
                if first_image.is_none() && is_image_resource(resource) {
                    first_image = Some(binary_index);
                }
                first_image_by_binary.push(first_image);
            }
        }
        Self {
            resources,
            by_id,
            by_href,
            binary_by_href,
            binary_resources,
            first_image_by_binary,
        }
    }

    pub(crate) fn by_id(&self, id: &str) -> Option<&'a Resource> {
        self.by_id
            .get(id)
            .and_then(|&index| self.resources.get(index))
    }

    pub(crate) fn first_css(&self, normalized_href: &str) -> Option<&'a Resource> {
        self.by_href
            .get(normalized_href)?
            .iter()
            .find_map(|&index| {
                let resource = self.resources.get(index)?;
                is_css_resource(resource).then_some(resource)
            })
    }

    fn resource_reference(&self, base_href: &str, target: &str) -> Option<String> {
        let resolved = resolve_path(base_href, target)?;
        let mut matching_binary_index = None;
        for candidate_start in std::iter::once(0).chain(
            resolved
                .as_bytes()
                .iter()
                .enumerate()
                .filter_map(|(index, byte)| (*byte == b'/').then_some(index + 1)),
        ) {
            let candidate = &resolved[candidate_start..];
            let Some(&binary_index) = self.binary_by_href.get(candidate) else {
                continue;
            };
            if matching_binary_index.is_none_or(|current| binary_index < current) {
                matching_binary_index = Some(binary_index);
            }
        }
        let binary_index = matching_binary_index?;
        let resource = self
            .binary_resources
            .get(binary_index)
            .and_then(|&index| self.resources.get(index))?;
        let embed_index = match self.first_image_by_binary[binary_index] {
            Some(first_image) if binary_index >= first_image => binary_index - first_image + 1,
            Some(_) => return None,
            None if is_font_resource(resource) => binary_index + 1,
            None => return None,
        };
        Some(format!(
            "kindle:embed:{}?mime={}",
            to_base32(u32::try_from(embed_index).ok()?),
            resource.media_type
        ))
    }
}

#[derive(Debug)]
pub(crate) struct SectionIndex {
    by_href: HashMap<String, usize>,
    css_bases: HashMap<String, String>,
    section_count: usize,
}

impl SectionIndex {
    pub(crate) fn new(sections: &[KindleSection]) -> Self {
        let mut by_href = HashMap::new();
        let mut css_bases = HashMap::new();
        for (section_index, section) in sections.iter().enumerate() {
            if let Some(href) = normalize_path(&section.href) {
                by_href.entry(href).or_insert(section_index);
            }
            for style_href in &section.referenced_styles {
                if let Some(resolved) = resolve_path(&section.href, style_href) {
                    css_bases
                        .entry(resolved)
                        .or_insert_with(|| section.href.clone());
                }
            }
        }
        Self {
            by_href,
            css_bases,
            section_count: sections.len(),
        }
    }

    pub(crate) fn resolve(&self, section_href: &str, target_path: &str) -> Option<usize> {
        // Link processing receives both source-relative hrefs and document
        // paths that have already been canonicalized by an earlier stage.
        // Prefer the canonical coordinate directly; only resolve against the
        // owning document when the supplied path is not already indexed.
        let canonical_target = normalize_path(target_path)?;
        if let Some(&section_index) = self.by_href.get(&canonical_target) {
            return Some(section_index);
        }
        let target = resolve_path(section_href, target_path)?;
        self.by_href.get(&target).copied()
    }

    pub(crate) fn len(&self) -> usize {
        self.section_count
    }

    fn css_base_href(&self, normalized_resource_href: &str) -> Option<&str> {
        self.css_bases
            .get(normalized_resource_href)
            .map(String::as_str)
    }
}

#[derive(Debug)]
pub(crate) struct CssResourceIndex<'a> {
    pub(crate) resources: Vec<&'a Resource>,
    by_href: HashMap<String, u32>,
}

impl<'a> CssResourceIndex<'a> {
    fn new(resources: Vec<&'a Resource>) -> Self {
        let mut by_href = HashMap::new();
        for (index, resource) in resources.iter().enumerate() {
            if let Some(href) = normalize_path(&resource.href) {
                if let Ok(index) = u32::try_from(index + 1) {
                    by_href.entry(href).or_insert(index);
                }
            }
        }
        Self { resources, by_href }
    }

    pub(crate) fn len(&self) -> usize {
        self.resources.len()
    }

    pub(crate) fn flow_number(&self, resolved_href: &str) -> Option<u32> {
        self.by_href.get(resolved_href).copied()
    }
}

pub(crate) fn referenced_css_resources<'a>(
    sections: &[KindleSection],
    resource_index: &ResourceIndex<'a>,
    section_index: &SectionIndex,
) -> CssResourceIndex<'a> {
    let roots = sections
        .iter()
        .flat_map(|section| {
            section
                .referenced_styles
                .iter()
                .map(|reference| (section.href.clone(), reference.clone()))
        })
        .collect::<Vec<_>>();
    let resolved = traverse_css_dependencies(roots, |base_href, reference| {
        let resolved = resolve_path(base_href, reference)?;
        let resource = resource_index.first_css(&resolved)?;
        let css = std::str::from_utf8(&resource.data)
            .expect("EPUB CSS resources are normalized to UTF-8");
        Some((
            resolved,
            css_resource_base_href(section_index, resource),
            css_import_targets(css),
        ))
    });
    let resources = resolved
        .into_iter()
        .filter_map(|href| resource_index.first_css(&href))
        .collect();
    CssResourceIndex::new(resources)
}

pub(crate) fn css_resource_base_href(section_index: &SectionIndex, resource: &Resource) -> String {
    let Some(normalized_resource_href) = normalize_path(&resource.href) else {
        return resource.href.clone();
    };
    if !normalized_resource_href.starts_with("__inline_css__/")
        || !resource
            .properties
            .iter()
            .any(|property| property == SYNTHETIC_INLINE_CSS_PROPERTY)
    {
        return resource.href.clone();
    }

    let resource_href = normalized_resource_href.as_str();
    section_index
        .css_base_href(resource_href)
        .map(str::to_owned)
        .unwrap_or_else(|| resource.href.clone())
}

pub(crate) fn rewrite_css_assets(
    source: &[u8],
    css_href: &str,
    resources: &ResourceIndex<'_>,
    css_resources: &CssResourceIndex<'_>,
) -> Vec<u8> {
    let Ok(source) = std::str::from_utf8(source) else {
        // Invalid CSS cannot be scanned safely as text. Preserve its raw
        // bytes rather than corrupting transport through replacement chars.
        return source.to_vec();
    };
    let source = rewrite_css_imports(source, css_href, css_resources);
    rewrite_css_urls(&source, css_href, resources).into_bytes()
}

pub(crate) fn rewrite_css_urls(
    source: &str,
    css_href: &str,
    resources: &ResourceIndex<'_>,
) -> String {
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    for url in css_url_spans(source) {
        let target = &source[url.target_start..url.target_end];
        if is_remote_reference(target) {
            result.push_str(&source[cursor..url.start]);
            cursor = url.close_end;
            continue;
        }
        let Some(reference) = resources.resource_reference(css_href, target) else {
            continue;
        };
        result.push_str(&source[cursor..url.start]);
        if url.preserve_wrappers {
            // Preserve whitespace, comments, and wrappers around a real URL,
            // while removing only source quote delimiters.
            let quote_offset = usize::from(url.quote.is_some());
            result.push_str(&source[url.start..url.target_start - quote_offset]);
            result.push_str(&reference);
            result.push_str(&source[url.target_end + quote_offset..url.close_end]);
        } else {
            result.push_str("url(");
            result.push_str(&reference);
            result.push(')');
        }
        cursor = url.close_end;
    }
    result.push_str(&source[cursor..]);
    result
}

#[derive(Debug, Clone, Copy)]
struct CssUrlSpan {
    start: usize,
    target_start: usize,
    target_end: usize,
    close_end: usize,
    quote: Option<u8>,
    preserve_wrappers: bool,
}

fn css_url_spans(source: &str) -> Vec<CssUrlSpan> {
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
        if css_function_at(source, cursor, "url") && bytes.get(cursor + 3) == Some(&b'(') {
            if let Some(span) = parse_css_url_span(source, cursor) {
                cursor = span.close_end;
                spans.push(span);
                continue;
            }
        }
        cursor = advance_css_char(source, cursor);
    }
    spans
}

fn parse_css_url_span(source: &str, start: usize) -> Option<CssUrlSpan> {
    let open = start.checked_add(3)?;
    if source.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let (mut cursor, mut preserve_wrappers) = skip_css_space_comments(source, open + 1)?;
    if source
        .as_bytes()
        .get(cursor)
        .is_some_and(|byte| *byte == b'\'' || *byte == b'\"')
    {
        let quote_end = skip_css_string(source, cursor)?;
        let target_start = cursor + 1;
        let target_end = quote_end.checked_sub(1)?;
        let (close, trailing_comments) = skip_css_space_comments(source, quote_end)?;
        preserve_wrappers |= trailing_comments;
        if source.as_bytes().get(close) != Some(&b')') {
            return None;
        }
        return Some(CssUrlSpan {
            start,
            target_start,
            target_end,
            close_end: close + 1,
            quote: Some(source.as_bytes()[cursor]),
            preserve_wrappers,
        });
    }

    let target_start = cursor;
    while cursor < source.len() {
        let byte = source.as_bytes()[cursor];
        if byte == b')' {
            break;
        }
        if byte.is_ascii_whitespace()
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
    let (close, trailing_comments) = skip_css_space_comments(source, cursor)?;
    preserve_wrappers |= trailing_comments;
    if source.as_bytes().get(close) != Some(&b')') {
        return None;
    }
    Some(CssUrlSpan {
        start,
        target_start,
        target_end,
        close_end: close + 1,
        quote: None,
        preserve_wrappers,
    })
}

fn rewrite_css_imports(
    source: &str,
    css_href: &str,
    css_resources: &CssResourceIndex<'_>,
) -> String {
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    for import in css_import_spans(source) {
        let target = &source[import.target_start..import.target_end];
        if is_remote_reference(target) {
            result.push_str(&source[cursor..import.statement_start]);
            cursor = css_import_statement_end(source, import.wrapper_end);
            continue;
        }
        let Some(flow_number) = css_flow_number(css_href, target, css_resources) else {
            continue;
        };
        // Keep @import rather than flattening it: the CSS resource graph,
        // import order, and media/query suffix are all transport contracts,
        // while only the local target wrapper needs a flow address. KindleGen
        // transport uses the canonical unquoted url() wrapper for local CSS.
        result.push_str(&source[cursor..import.wrapper_start]);
        result.push_str("url(");
        result.push_str(&stylesheet_flow_reference(flow_number));
        result.push(')');
        cursor = import.wrapper_end;
    }
    result.push_str(&source[cursor..]);
    result
}

fn css_import_statement_end(source: &str, start: usize) -> usize {
    source[start..]
        .find(';')
        .map_or(source.len(), |offset| start + offset + 1)
}

pub(crate) fn resource_reference(
    base_href: &str,
    target: &str,
    resources: &ResourceIndex<'_>,
) -> Option<String> {
    resources.resource_reference(base_href, target)
}

pub(crate) fn stylesheet_flow_reference(flow_number: u32) -> String {
    format!(
        "kindle:flow:{}?mime=text/css",
        to_base32_fixed(flow_number, 4).expect("CSS flow number fits in four digits")
    )
}

pub(crate) fn css_flow_number(
    base_href: &str,
    target: &str,
    css_resources: &CssResourceIndex<'_>,
) -> Option<u32> {
    let resolved = resolve_path(base_href, target)?;
    css_resources.flow_number(&resolved)
}
