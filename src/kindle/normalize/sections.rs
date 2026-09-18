use std::collections::{HashMap, HashSet};

use super::super::{KindleSection, ir::KindleLayoutSemantic};
use crate::book::{Book, ContentDocument, ReadingOrderItem};
use crate::xhtml::scan::{Tag, find_ascii_case_insensitive, tags};

pub(super) const COVER_LANDMARK_MARKER: &str = "kindle:cover-landmark";

pub(super) struct CoverPhase {
    pub(super) cover_hrefs: HashSet<String>,
    pub(super) cover_ids: HashSet<String>,
    pub(super) keep_comic_cover_page: bool,
    pub(super) omitted_cover_hrefs: HashSet<String>,
}

pub(super) fn prepare_cover_phase(book: &Book) -> CoverPhase {
    let has_native_cover_resource = book.metadata.cover.as_deref().is_some_and(|cover_id| {
        book.resources.items.iter().any(|resource| {
            resource.id == cover_id
                && resource
                    .media_type
                    .get(.."image/".len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
        })
    });
    // A cover document can only be suppressed when its destination can be
    // replaced by a package-selected native Kindle cover. Without that
    // resource, keep the source document so its cover links remain resolvable.
    let cover_hrefs = if has_native_cover_resource {
        cover_document_hrefs(book)
    } else {
        HashSet::new()
    };
    let cover_ids = if has_native_cover_resource {
        book.content
            .iter()
            .filter(|content| {
                content.is_cover || cover_hrefs.contains(&document_path(&content.href))
            })
            .map(|content| content.id.clone())
            .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let keep_comic_cover_page = book.metadata.is_fixed_layout
        && book
            .metadata
            .book_type
            .as_deref()
            .is_some_and(|book_type| book_type.trim().eq_ignore_ascii_case("comic"));
    let omitted_cover_hrefs = book
        .reading_order
        .items
        .iter()
        .filter(|item| {
            cover_ids.contains(item.id.as_str()) || cover_hrefs.contains(&document_path(&item.href))
        })
        .map(|item| document_path(&item.href))
        .filter(|href| !href.is_empty())
        .collect::<HashSet<_>>();
    CoverPhase {
        cover_hrefs,
        cover_ids,
        keep_comic_cover_page,
        omitted_cover_hrefs,
    }
}

pub(super) fn normalize_sections(
    content: Vec<ContentDocument>,
    reading_items: Vec<ReadingOrderItem>,
    cover_hrefs: &HashSet<String>,
    cover_ids: &HashSet<String>,
    keep_comic_cover_page: bool,
) -> Vec<KindleSection> {
    let mut content_by_id = content
        .into_iter()
        .map(|content| (content.id.clone(), content))
        .collect::<HashMap<_, _>>();
    reading_items
        .into_iter()
        .filter_map(|item| {
            let content = content_by_id.remove(&item.id)?;
            let is_cover = cover_ids.contains(item.id.as_str())
                || cover_hrefs.contains(&document_path(&item.href));
            if is_cover && !(keep_comic_cover_page && item.linear && content.is_pre_paginated()) {
                return None;
            }
            let layout = if content.is_pre_paginated() {
                KindleLayoutSemantic::PrePaginated
            } else {
                KindleLayoutSemantic::Reflowable
            };
            let source_xhtml = if cover_hrefs.is_empty() {
                content.source_xhtml
            } else {
                match neutralize_cover_links(&content.source_xhtml, &item.href, cover_hrefs) {
                    Some(source) => source,
                    None => content.source_xhtml,
                }
            };
            let is_svg_document = content.media_type.eq_ignore_ascii_case("image/svg+xml");
            Some(KindleSection {
                id: item.id,
                href: item.href,
                source_xhtml,
                is_svg_document,
                referenced_styles: content.referenced_styles,
                dropped_stylesheets: content.dropped_stylesheets,
                page_viewport: content.page_viewport,
                linear: item.linear,
                layout,
                rendition: content.rendition,
                source_properties: content.source_properties,
                source_spine_index: Some(content.source_spine_index),
            })
        })
        .collect()
}

fn cover_document_hrefs(book: &Book) -> HashSet<String> {
    let mut hrefs = book
        .navigation
        .landmarks
        .iter()
        .filter(|landmark| landmark.kind.eq_ignore_ascii_case("cover"))
        .map(|landmark| document_path(&landmark.href))
        .filter(|href| !href.is_empty())
        .collect::<HashSet<_>>();
    hrefs.extend(
        book.content
            .iter()
            .filter(|content| content.is_cover)
            .map(|content| document_path(&content.href)),
    );
    hrefs.retain(|href| !href.is_empty());
    hrefs
}

pub(super) fn document_path(href: &str) -> String {
    let path = href
        .find(['#', '?'])
        .map(|index| &href[..index])
        .unwrap_or(href)
        .replace('\\', "/");
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }
    parts.join("/")
}

fn neutralize_cover_links(
    source: &str,
    section_href: &str,
    cover_hrefs: &HashSet<String>,
) -> Option<String> {
    if cover_hrefs.is_empty() {
        return None;
    }
    let section_path = document_path(section_href);
    let section_directory = section_path
        .rsplit_once('/')
        .map(|(directory, _)| directory)
        .unwrap_or_default();
    let mut nav_stack = Vec::new();
    let mut output: Option<String> = None;
    let mut output_cursor = 0;
    for tag in tags(source) {
        let name = tag.name();
        let closing = source.as_bytes().get(tag.start + 1) == Some(&b'/');
        if name.eq_ignore_ascii_case("nav") {
            if closing {
                nav_stack.pop();
            } else {
                nav_stack.push(has_attribute_token(
                    &source[tag.start..tag.end],
                    "epub:type",
                    "landmarks",
                ));
            }
        }
        let is_cover_landmark = !closing
            && name.eq_ignore_ascii_case("a")
            && nav_stack.iter().any(|is_landmarks| *is_landmarks)
            && has_attribute_token(&source[tag.start..tag.end], "epub:type", "cover");
        for_each_quoted_attribute_range(tag, "href", |value_start, value_end| {
            let target = &source[value_start..value_end];
            if cover_hrefs.contains(&resolve_document_path_from_directory(
                section_path.as_str(),
                section_directory,
                target,
            )) {
                let replacement = if is_cover_landmark {
                    COVER_LANDMARK_MARKER
                } else {
                    "#"
                };
                let result = output.get_or_insert_with(|| String::with_capacity(source.len()));
                result.push_str(&source[output_cursor..value_start]);
                result.push_str(replacement);
                output_cursor = value_end;
            }
        });
    }
    output.map(|mut output| {
        output.push_str(&source[output_cursor..]);
        output
    })
}

/// Visit quoted value ranges for exact, case-insensitive occurrences of an
/// attribute name. The ranges exclude quotes so replacements preserve the
/// original quoting and every other source byte.
fn for_each_quoted_attribute_range(
    tag: Tag<'_>,
    wanted: &str,
    mut visit: impl FnMut(usize, usize),
) {
    let source = tag.source;
    let bytes = source.as_bytes();
    let mut cursor = tag.name_end;
    let limit = tag.end.saturating_sub(1);
    while cursor < limit {
        while cursor < limit && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= limit || bytes[cursor] == b'/' {
            break;
        }
        let name_start = cursor;
        while cursor < limit
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/' | b'>')
        {
            cursor += 1;
        }
        if name_start == cursor {
            cursor += 1;
            continue;
        }
        let name_end = cursor;
        while cursor < limit && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= limit || bytes[cursor] != b'=' {
            // The attribute name itself advanced `cursor`; leave the next
            // loop iteration at the next attribute or the end of this tag.
            continue;
        }
        cursor += 1;
        while cursor < limit && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let Some(&quote @ (b'\'' | b'"')) = bytes.get(cursor) else {
            while cursor < limit && !bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            continue;
        };
        let value_start = cursor + 1;
        let mut value_end = value_start;
        while value_end < limit && bytes[value_end] != quote {
            value_end += 1;
        }
        if value_end >= limit {
            break;
        }
        if source[name_start..name_end].eq_ignore_ascii_case(wanted) {
            visit(value_start, value_end);
        }
        cursor = value_end + 1;
    }
}

fn has_attribute_token(tag: &str, wanted: &str, value: &str) -> bool {
    let Some(start) = find_ascii_case_insensitive(tag, wanted, 0) else {
        return false;
    };
    let mut cursor = start + wanted.len();
    while tag
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor += 1;
    }
    if tag.as_bytes().get(cursor) != Some(&b'=') {
        return false;
    }
    cursor += 1;
    while tag
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor += 1;
    }
    let Some(&quote) = tag.as_bytes().get(cursor) else {
        return false;
    };
    if !matches!(quote, b'"' | b'\'') {
        return false;
    }
    let start = cursor + 1;
    let Some(end) = tag[start..].find(quote as char) else {
        return false;
    };
    tag[start..start + end]
        .split_whitespace()
        .any(|candidate| candidate.eq_ignore_ascii_case(value))
}

fn resolve_document_path_from_directory(
    section_path: &str,
    section_directory: &str,
    target: &str,
) -> String {
    let target_path = target
        .find(['#', '?'])
        .map(|index| &target[..index])
        .unwrap_or(target);
    if target_path.is_empty() {
        return section_path.to_owned();
    }
    document_path(&format!("{section_directory}/{target_path}"))
}
