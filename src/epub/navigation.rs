//! Parse NCX and EPUB 3 navigation into common navigation semantics.
//!
//! This module absorbs source-format differences and canonicalizes navigation
//! targets; KF8 INDX/CTOC serialization remains in `kf8::ncx`.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::opf::{ManifestItem, attr, local_name};
use super::package::resolve_href;
use crate::book::{
    Navigation, NavigationGroup, NavigationItem, NavigationLandmark, plain_display_text,
};
use crate::error::{Error, Result};
use crate::xhtml::path::{is_external_reference, normalize_path, resolve_path};
pub(super) fn parse_ncx(xml: &[u8]) -> Result<Navigation> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut navigation = Navigation::default();
    let mut stack: Vec<NavigationItem> = Vec::new();
    let mut current_text = None;
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(event) => {
                let name = local_name(event.name().as_ref());
                if name == "navpoint" {
                    stack.push(NavigationItem::default());
                }
                if name == "text" {
                    current_text = Some(String::new());
                }
                if name == "img" {
                    if let (Some(current_text), Some(alt)) =
                        (current_text.as_mut(), attr(&event, "alt"))
                    {
                        current_text.push_str(&alt);
                    }
                }
                if name == "content" {
                    if let Some(item) = stack.last_mut() {
                        item.href = attr(&event, "src").unwrap_or_default();
                    }
                }
            }
            Event::Empty(event) if local_name(event.name().as_ref()) == "content" => {
                if let Some(item) = stack.last_mut() {
                    item.href = attr(&event, "src").unwrap_or_default();
                }
            }
            Event::Empty(event) if local_name(event.name().as_ref()) == "img" => {
                if let (Some(current_text), Some(alt)) =
                    (current_text.as_mut(), attr(&event, "alt"))
                {
                    current_text.push_str(&alt);
                }
            }
            Event::Text(event) => {
                if let Some(current_text) = current_text.as_mut() {
                    current_text.push_str(
                        &event
                            .unescape()
                            .map_err(|error| Error::Xml(error.to_string()))?,
                    );
                }
            }
            Event::CData(event) => {
                if let Some(current_text) = current_text.as_mut() {
                    current_text.push_str(&String::from_utf8_lossy(event.as_ref()));
                }
            }
            Event::End(event) => {
                let name = local_name(event.name().as_ref());
                if name == "text" {
                    if let Some(item) = stack.last_mut() {
                        item.label =
                            plain_display_text(current_text.take().as_deref().unwrap_or_default());
                    }
                }
                if name == "navpoint" {
                    if let Some(item) = stack.pop() {
                        if let Some(parent) = stack.last_mut() {
                            parent.children.push(item);
                        } else {
                            navigation.items.push(item);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(navigation)
}

pub(super) fn parse_nav_xhtml(xml: &[u8]) -> Result<Navigation> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut navigation = Navigation::default();
    let mut current_anchor: Option<(NavigationItem, Option<String>)> = None;
    let mut current_unlinked_span: Option<String> = None;
    let mut list_items: Vec<(NavigationItem, Option<String>)> = Vec::new();
    let mut nav_stack: Vec<Option<String>> = Vec::new();
    let mut toc_nav_count = 0usize;
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(event) if local_name(event.name().as_ref()) == "nav" => {
                if attr(&event, "type").is_some_and(|kind| has_token(&kind, "toc")) {
                    toc_nav_count += 1;
                }
                nav_stack.push(nav_kind(&event));
            }
            Event::Empty(event) if local_name(event.name().as_ref()) == "nav" => {
                if attr(&event, "type").is_some_and(|kind| has_token(&kind, "toc")) {
                    toc_nav_count += 1;
                }
            }
            Event::Start(event) if local_name(event.name().as_ref()) == "li" => {
                if !nav_stack.is_empty() {
                    list_items.push((NavigationItem::default(), None));
                }
            }
            Event::Start(event) if local_name(event.name().as_ref()) == "a" => {
                let kind = attr(&event, "type").or_else(|| attr(&event, "role"));
                current_anchor = Some((
                    NavigationItem {
                        href: attr(&event, "href").unwrap_or_default(),
                        ..NavigationItem::default()
                    },
                    kind,
                ));
            }
            Event::Start(event)
                if local_name(event.name().as_ref()) == "span"
                    && current_anchor.is_none()
                    && !list_items.is_empty() =>
            {
                current_unlinked_span = Some(String::new());
            }
            Event::Text(event) => {
                if let Some((item, _)) = current_anchor.as_mut() {
                    item.label.push_str(
                        &event
                            .unescape()
                            .map_err(|error| Error::Xml(error.to_string()))?,
                    );
                } else if let Some(heading) = current_unlinked_span.as_mut() {
                    heading.push_str(
                        &event
                            .unescape()
                            .map_err(|error| Error::Xml(error.to_string()))?,
                    );
                }
            }
            Event::Empty(event) if local_name(event.name().as_ref()) == "img" => {
                if let Some((item, _)) = current_anchor.as_mut() {
                    if let Some(alt) = attr(&event, "alt") {
                        item.label.push_str(&alt);
                    }
                }
            }
            Event::CData(event) => {
                if let Some((item, _)) = current_anchor.as_mut() {
                    item.label
                        .push_str(&String::from_utf8_lossy(event.as_ref()));
                } else if let Some(heading) = current_unlinked_span.as_mut() {
                    heading.push_str(&String::from_utf8_lossy(event.as_ref()));
                }
            }
            Event::End(event) if local_name(event.name().as_ref()) == "span" => {
                if let Some(heading) = current_unlinked_span.take() {
                    if let Some((item, _)) = list_items.last_mut() {
                        item.label = plain_display_text(&heading);
                    }
                }
            }
            Event::End(event) if local_name(event.name().as_ref()) == "a" => {
                if let Some((mut item, anchor_kind)) = current_anchor.take() {
                    item.label = plain_display_text(&item.label);
                    if let Some((list_item, list_anchor_kind)) = list_items.last_mut() {
                        list_item.href = item.href;
                        list_item.label = item.label;
                        *list_anchor_kind = anchor_kind;
                    } else {
                        append_nav_item(
                            &mut navigation,
                            item,
                            anchor_kind,
                            nav_stack.last().and_then(Clone::clone),
                        );
                    }
                }
            }
            Event::End(event) if local_name(event.name().as_ref()) == "li" => {
                if let Some((item, anchor_kind)) = list_items.pop() {
                    if let Some((parent, _)) = list_items.last_mut() {
                        parent.children.push(item);
                    } else {
                        append_nav_item(
                            &mut navigation,
                            item,
                            anchor_kind,
                            nav_stack.last().and_then(Clone::clone),
                        );
                    }
                }
            }
            Event::End(event) if local_name(event.name().as_ref()) == "nav" => {
                nav_stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    match toc_nav_count {
        0 => Err(Error::InvalidEpub(
            "navigation document has no epub:type=toc nav".to_owned(),
        )),
        1 => Ok(navigation),
        count => Err(Error::InvalidEpub(format!(
            "navigation document contains {count} epub:type=toc nav elements; exactly one is required"
        ))),
    }
}

fn append_nav_item(
    navigation: &mut Navigation,
    item: NavigationItem,
    anchor_kind: Option<String>,
    nav_kind: Option<String>,
) {
    let nav_kind = nav_kind.unwrap_or_default();
    if has_token(&nav_kind, "landmarks") {
        let kind = anchor_kind.unwrap_or_default();
        navigation.landmarks.push(NavigationLandmark {
            kind: normalize_landmark_kind(&kind),
            label: item.label,
            href: item.href,
        });
    } else if has_token(&nav_kind, "page-list") || has_token(&nav_kind, "page_list") {
        navigation.page_list.push(item);
    } else if nav_kind.is_empty() || has_token(&nav_kind, "toc") {
        navigation.items.push(item);
    } else {
        let group = navigation
            .custom
            .iter_mut()
            .find(|group| group.kind.eq_ignore_ascii_case(&nav_kind));
        if let Some(group) = group {
            group.items.push(item);
        } else {
            navigation.custom.push(NavigationGroup {
                kind: nav_kind,
                items: vec![item],
            });
        }
    }
}

fn nav_kind(event: &BytesStart<'_>) -> Option<String> {
    attr(event, "type").or_else(|| attr(event, "role"))
}

pub(super) fn has_token(value: &str, token: &str) -> bool {
    value
        .split_whitespace()
        .any(|part| part.eq_ignore_ascii_case(token))
}

fn normalize_landmark_kind(value: &str) -> String {
    if has_token(value, "cover") {
        return "cover".to_owned();
    }
    value
        .split_whitespace()
        .find(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                "text" | "body" | "bodymatter" | "start" | "titlepage" | "title-page" | "toc"
            )
        })
        .map(|part| match part.to_ascii_lowercase().as_str() {
            "bodymatter" | "body" => "text".to_owned(),
            "title-page" => "titlepage".to_owned(),
            value => value.to_owned(),
        })
        .unwrap_or_default()
}

pub(super) fn canonicalize_navigation(
    navigation: &mut Navigation,
    navigation_path: &str,
    opf_base: &Path,
    manifest: &[ManifestItem],
    direct_svg_spine_ids: &HashSet<String>,
) -> Result<()> {
    let navigation_base = Path::new(navigation_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let mut manifest_by_resolved_path = HashMap::with_capacity(manifest.len());
    let mut manifest_by_href = HashMap::with_capacity(manifest.len());
    for candidate in manifest.iter().filter(|candidate| {
        candidate
            .media_type
            .eq_ignore_ascii_case("application/xhtml+xml")
            || candidate.media_type.eq_ignore_ascii_case("text/html")
            || direct_svg_spine_ids.contains(&candidate.id)
    }) {
        let resolved_path = resolve_href(opf_base, &candidate.href);
        // Preserve the first declaration for duplicate and empty resolved
        // paths. Empty is a valid key because resolve_href returns it for
        // external hrefs.
        manifest_by_resolved_path
            .entry(resolved_path)
            .or_insert_with(|| candidate.href.clone());
        if !is_external_reference(&candidate.href) {
            if let Some(manifest_spelling) = normalize_path(&candidate.href) {
                // Keep the Book-coordinate spelling separately so a repeated
                // canonicalization pass does not resolve it against nav again.
                manifest_by_href
                    .entry(manifest_spelling)
                    .or_insert_with(|| candidate.href.clone());
            }
        }
    }
    for item in &mut navigation.items {
        canonicalize_navigation_item(
            item,
            navigation_base,
            &manifest_by_resolved_path,
            &manifest_by_href,
        )?;
    }
    drop_cfi_page_list_items(&mut navigation.page_list);
    for item in &mut navigation.page_list {
        canonicalize_navigation_item(
            item,
            navigation_base,
            &manifest_by_resolved_path,
            &manifest_by_href,
        )?;
    }
    for landmark in &mut navigation.landmarks {
        let (target_path, suffix) = split_link_suffix(&landmark.href);
        if !target_path.is_empty() {
            validate_local_navigation_target(target_path, navigation_base)?;
            landmark.href = format!(
                "{}{}",
                canonical_navigation_target(
                    target_path,
                    navigation_base,
                    &manifest_by_resolved_path,
                    &manifest_by_href,
                ),
                suffix
            );
        }
    }
    for group in &mut navigation.custom {
        for item in &mut group.items {
            canonicalize_navigation_item(
                item,
                navigation_base,
                &manifest_by_resolved_path,
                &manifest_by_href,
            )?;
        }
    }
    Ok(())
}

fn drop_cfi_page_list_items(items: &mut Vec<NavigationItem>) {
    let mut retained = Vec::with_capacity(items.len());
    for mut item in std::mem::take(items) {
        drop_cfi_page_list_items(&mut item.children);
        if is_epub_cfi_page_list_target(&item.href) {
            // Keep any ordinary nested page targets while dropping only the
            // unsupported CFI destination itself.
            retained.append(&mut item.children);
        } else {
            retained.push(item);
        }
    }
    *items = retained;
}

fn is_epub_cfi_page_list_target(href: &str) -> bool {
    let Some((_, fragment)) = href.split_once('#') else {
        return false;
    };
    let fragment = fragment.split('?').next().unwrap_or(fragment);
    fragment
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("epubcfi("))
        && fragment.ends_with(')')
}

fn canonicalize_navigation_item(
    item: &mut NavigationItem,
    navigation_base: &Path,
    manifest_by_resolved_path: &HashMap<String, String>,
    manifest_by_href: &HashMap<String, String>,
) -> Result<()> {
    let (target_path, suffix) = split_link_suffix(&item.href);
    if !target_path.is_empty() {
        validate_local_navigation_target(target_path, navigation_base)?;
        item.href = format!(
            "{}{}",
            canonical_navigation_target(
                target_path,
                navigation_base,
                manifest_by_resolved_path,
                manifest_by_href,
            ),
            suffix
        );
    }
    for child in &mut item.children {
        canonicalize_navigation_item(
            child,
            navigation_base,
            manifest_by_resolved_path,
            manifest_by_href,
        )?;
    }
    Ok(())
}

fn canonical_navigation_target(
    target_path: &str,
    navigation_base: &Path,
    manifest_by_resolved_path: &HashMap<String, String>,
    manifest_by_href: &HashMap<String, String>,
) -> String {
    // Navigation hrefs may already be in package-document coordinates. When
    // that path names a manifest document, or is already in manifest spelling,
    // preserve it instead of resolving it against the navigation document.
    if let Some(target) = normalize_path(target_path) {
        if let Some(manifest_href) = manifest_by_resolved_path
            .get(&target)
            .or_else(|| manifest_by_href.get(&target))
        {
            return manifest_href.clone();
        }
    }
    let resolved_target = resolve_href(navigation_base, target_path);
    manifest_by_resolved_path
        .get(&resolved_target)
        .cloned()
        .unwrap_or(resolved_target)
}

fn validate_local_navigation_target(target: &str, navigation_base: &Path) -> Result<()> {
    if is_external_reference(target) {
        return Ok(());
    }
    let base = format!("{}/", navigation_base.to_string_lossy());
    if resolve_path(&base, target).is_none() {
        return Err(Error::InvalidEpub(format!(
            "navigation target {target} escapes the EPUB root"
        )));
    }
    Ok(())
}

pub(super) fn split_link_suffix(href: &str) -> (&str, &str) {
    href.find(['#', '?'])
        .map(|index| (&href[..index], &href[index..]))
        .unwrap_or((href, ""))
}
