use std::collections::HashSet;

use super::super::{KindleLandmark, KindleNavigationItem, KindleSection, ir::KindleLayoutSemantic};
use super::sections::{COVER_LANDMARK_MARKER, document_path};
use crate::book::{NavigationItem, NavigationLandmark, Resource, plain_display_text};

pub(super) fn normalize_navigation(
    items: Vec<NavigationItem>,
    mut page_list: Vec<NavigationItem>,
    resources: &[Resource],
    sections: &mut Vec<KindleSection>,
    cover_hrefs: &HashSet<String>,
    omitted_cover_hrefs: &HashSet<String>,
) -> (Vec<KindleNavigationItem>, Option<String>) {
    let navigation = prune_navigation(items, cover_hrefs)
        .into_iter()
        .map(KindleNavigationItem::from)
        .collect::<Vec<_>>();
    let suppressed_cover_hrefs = omitted_cover_hrefs
        .iter()
        .filter(|cover_href| {
            !sections
                .iter()
                .any(|section| document_path(&section.href) == cover_href.as_str())
        })
        .cloned()
        .collect::<HashSet<_>>();
    redirect_suppressed_cover_targets(&mut page_list, &suppressed_cover_hrefs);
    let page_list = page_list
        .into_iter()
        .map(KindleNavigationItem::from)
        .collect::<Vec<_>>();
    // A manifest nav resource is a navigation source, not visible content, when
    // it is absent from the spine. When it is in the spine, preserve its source
    // position and linear semantics.
    let navigation_resource = resources.iter().find(|resource| {
        (resource
            .media_type
            .eq_ignore_ascii_case("application/xhtml+xml")
            || resource.media_type.eq_ignore_ascii_case("text/html"))
            && resource
                .properties
                .iter()
                .any(|property| property.eq_ignore_ascii_case("nav"))
    });
    let toc_href = if let Some(resource) = navigation_resource {
        if sections.iter().any(|section| section.id == resource.id) {
            Some(resource.href.clone())
        } else {
            None
        }
    } else if navigation.is_empty() {
        None
    } else {
        let href = synthetic_toc_href(sections, resources);
        sections.insert(
            0,
            KindleSection {
                id: "__kindle_toc".to_owned(),
                href: href.clone(),
                source_xhtml: render_synthetic_toc(&navigation),
                is_svg_document: false,
                referenced_styles: Vec::new(),
                dropped_stylesheets: Vec::new(),
                page_viewport: None,
                linear: true,
                layout: KindleLayoutSemantic::Reflowable,
                rendition: Default::default(),
                source_properties: Vec::new(),
                source_spine_index: None,
            },
        );
        Some(href)
    };
    if !page_list.is_empty() {
        let href = synthetic_page_list_href(sections, resources);
        let insert_at = usize::from(toc_href.is_some()).min(sections.len());
        sections.insert(
            insert_at,
            KindleSection {
                id: "__kindle_page_list".to_owned(),
                href,
                source_xhtml: render_synthetic_page_list(&page_list),
                is_svg_document: false,
                referenced_styles: Vec::new(),
                dropped_stylesheets: Vec::new(),
                page_viewport: None,
                linear: true,
                layout: KindleLayoutSemantic::Reflowable,
                rendition: Default::default(),
                source_properties: Vec::new(),
                source_spine_index: None,
            },
        );
    }
    (navigation, toc_href)
}

fn redirect_suppressed_cover_targets(
    items: &mut [NavigationItem],
    suppressed_cover_hrefs: &HashSet<String>,
) {
    for item in items {
        if suppressed_cover_hrefs.contains(&document_path(&item.href)) {
            item.href = COVER_LANDMARK_MARKER.to_owned();
        }
        redirect_suppressed_cover_targets(&mut item.children, suppressed_cover_hrefs);
    }
}

pub(super) fn normalize_landmarks(
    source_landmarks: Vec<NavigationLandmark>,
    sections: &[KindleSection],
    toc_href: Option<&str>,
    omitted_cover_hrefs: &HashSet<String>,
) -> Vec<KindleLandmark> {
    let fallback_body_href = sections
        .iter()
        .find(|section| {
            section.linear
                && toc_href.is_none_or(|toc| document_path(&section.href) != document_path(toc))
        })
        .map(|section| section.href.clone());
    let mut landmarks = source_landmarks
        .into_iter()
        .filter(|landmark| !landmark.kind.eq_ignore_ascii_case("cover"))
        .map(|mut landmark| {
            let target_path = document_path(&landmark.href);
            let target_is_omitted_cover = !target_path.is_empty()
                && omitted_cover_hrefs.contains(&target_path)
                && !sections
                    .iter()
                    .any(|section| document_path(&section.href) == target_path);
            if is_bodymatter_landmark(&landmark.kind) && target_is_omitted_cover {
                if let Some(fallback_body_href) = fallback_body_href.as_ref() {
                    landmark.href = fallback_body_href.clone();
                }
            }
            landmark
        })
        .map(KindleLandmark::from)
        .collect::<Vec<_>>();
    if !landmarks
        .iter()
        .any(|landmark| is_bodymatter_landmark(&landmark.kind))
    {
        if let Some(body_section) = sections
            .iter()
            .find(|section| section.linear && Some(section.href.as_str()) != toc_href)
        {
            landmarks.push(KindleLandmark {
                kind: "text".to_owned(),
                label: "本文".to_owned(),
                href: body_section.href.clone(),
            });
        }
    }
    if let Some(href) = toc_href {
        if !landmarks
            .iter()
            .any(|landmark| landmark.kind.eq_ignore_ascii_case("toc"))
        {
            landmarks.push(KindleLandmark {
                kind: "toc".to_owned(),
                label: "目次".to_owned(),
                href: href.to_owned(),
            });
        }
    }
    landmarks
}

fn is_bodymatter_landmark(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "text" | "body" | "bodymatter" | "start"
    )
}

fn prune_navigation(
    items: Vec<NavigationItem>,
    cover_hrefs: &HashSet<String>,
) -> Vec<NavigationItem> {
    let mut result = Vec::new();
    for item in items {
        let NavigationItem {
            label,
            href,
            children: item_children,
        } = item;
        let children = prune_navigation(item_children, cover_hrefs);
        if cover_hrefs.contains(&document_path(&href)) {
            result.extend(children);
        } else {
            result.push(NavigationItem {
                label,
                href,
                children,
            });
        }
    }
    result
}

fn synthetic_toc_href(sections: &[KindleSection], resources: &[crate::book::Resource]) -> String {
    let mut index = 0usize;
    loop {
        let href = if index == 0 {
            "__kindle_toc.xhtml".to_owned()
        } else {
            format!("__kindle_toc-{index}.xhtml")
        };
        if sections.iter().all(|section| section.href != href)
            && resources.iter().all(|resource| resource.href != href)
        {
            return href;
        }
        index += 1;
    }
}

fn synthetic_page_list_href(
    sections: &[KindleSection],
    resources: &[crate::book::Resource],
) -> String {
    let mut index = 0usize;
    loop {
        let href = if index == 0 {
            "__kindle_page_list.xhtml".to_owned()
        } else {
            format!("__kindle_page_list-{index}.xhtml")
        };
        if sections.iter().all(|section| section.href != href)
            && resources.iter().all(|resource| resource.href != href)
        {
            return href;
        }
        index += 1;
    }
}

fn render_synthetic_toc(items: &[KindleNavigationItem]) -> String {
    let mut output = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" class="hltr">
<head><title>目　次</title></head>
<body class="p-toc"><nav epub:type="toc" id="toc"><h1>目　次</h1><ol>"#,
    );
    render_synthetic_toc_items(items, &mut output);
    output.push_str("</ol></nav></body></html>");
    output
}

fn render_synthetic_page_list(items: &[KindleNavigationItem]) -> String {
    let mut output = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Page List</title></head><body><nav epub:type="page-list"><h1>Page List</h1><ol>"#,
    );
    render_synthetic_toc_items(items, &mut output);
    output.push_str("</ol></nav></body></html>");
    output
}

fn render_synthetic_toc_items(items: &[KindleNavigationItem], output: &mut String) {
    for item in items {
        output.push_str("<li>");
        if item.href.is_empty() {
            push_html_text(output, &item.label);
        } else {
            output.push_str("<a href=\"");
            push_html_attribute(output, &item.href);
            output.push_str("\">");
            push_html_text(output, &item.label);
            output.push_str("</a>");
        }
        if !item.children.is_empty() {
            output.push_str("<ol>");
            render_synthetic_toc_items(&item.children, output);
            output.push_str("</ol>");
        }
        output.push_str("</li>");
    }
}

fn push_html_text(output: &mut String, value: &str) {
    push_html_escaped(output, value, false);
}

fn push_html_attribute(output: &mut String, value: &str) {
    push_html_escaped(output, value, true);
}

fn push_html_escaped(output: &mut String, value: &str, attribute: bool) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' if attribute => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

impl From<NavigationLandmark> for KindleLandmark {
    fn from(landmark: NavigationLandmark) -> Self {
        Self {
            kind: landmark.kind,
            label: plain_display_text(&landmark.label),
            href: landmark.href,
        }
    }
}

impl From<NavigationItem> for KindleNavigationItem {
    fn from(item: NavigationItem) -> Self {
        Self {
            label: plain_display_text(&item.label),
            href: item.href,
            children: item.children.into_iter().map(Self::from).collect(),
        }
    }
}
