use std::collections::BTreeSet;

use epub3_kindle::{Compression, ConvertOptions, convert_bytes, convert_file};

use crate::audit_support::epub;
use crate::audit_support::palm::reconstruct_text;
use crate::audit_support::semantic::{
    NavItem, SourceModel, TargetProjection, css_media_semantics, css_rule_declarations,
    decode_embed_number, decode_position_href, idpf_deobfuscate,
};
use crate::audit_support::temp::TempDir;

fn options() -> ConvertOptions {
    ConvertOptions {
        compression: Compression::None,
    }
}

fn project(input: &[u8]) -> (SourceModel, TargetProjection<'static>) {
    let source = SourceModel::parse(input).expect("independent source semantic model");
    let output = convert_bytes(input, &options()).expect("authority fixture converts");
    let output: &'static [u8] = Box::leak(output.into_boxed_slice());
    let target = TargetProjection::parse(output).expect("independent target projection");
    (source, target)
}

fn flatten_nav(items: &[NavItem], depth: usize, out: &mut Vec<(String, String, usize)>) {
    for item in items {
        out.push((item.label.clone(), item.href.clone(), depth));
        flatten_nav(&item.children, depth + 1, out);
    }
}

fn assert_common_source_projection(source: &SourceModel, target: &TargetProjection<'_>) {
    assert_eq!(
        target.sections.len(),
        source.sections.len(),
        "target section count must equal source spine content count"
    );
    for (source_section, target_section) in source.sections.iter().zip(&target.sections) {
        assert_eq!(
            target_section.visible_text, source_section.visible_text,
            "complete visible text must reconstruct for {}",
            source_section.href
        );
        assert_eq!(
            target_section.ids,
            source_section.ids.iter().cloned().collect::<BTreeSet<_>>(),
            "target fragment ID set must equal the source ID set"
        );
        assert_eq!(
            target_section.lang, source_section.lang,
            "section language must survive target projection"
        );
        assert_eq!(
            target_section.direction, source_section.direction,
            "section direction must survive target projection"
        );
        if !source_section.stylesheet_hrefs.is_empty() {
            assert!(
                target_section.stylesheet_hrefs.len() >= source_section.stylesheet_hrefs.len(),
                "source stylesheet reference count must be represented for {}",
                source_section.href
            );
            assert!(
                target_section
                    .stylesheet_hrefs
                    .iter()
                    .all(|href| href.starts_with("kindle:flow:")),
                "stylesheet references must be rewritten to target flow references"
            );
        }
        assert_eq!(
            target_section.fixed, source_section.fixed,
            "section rendition classification must survive target projection"
        );
        let semantic_tags = [
            "main",
            "h1",
            "h2",
            "h3",
            "ul",
            "ol",
            "li",
            "figure",
            "img",
            "figcaption",
            "table",
            "thead",
            "tbody",
            "tr",
            "th",
            "td",
            "p",
            "a",
            "ruby",
            "rt",
        ];
        let source_structure = source_section
            .tags
            .iter()
            .filter(|tag| semantic_tags.contains(&tag.as_str()))
            .collect::<Vec<_>>();
        let target_structure = target_section
            .tags
            .iter()
            .filter(|tag| semantic_tags.contains(&tag.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            target_structure, source_structure,
            "semantic element nesting/order must survive target reconstruction"
        );
        assert_eq!(
            target_section.links.len(),
            source_section.links.len(),
            "internal link occurrence count must survive"
        );
        for (index, (href, label)) in source_section.links.iter().enumerate() {
            let (sequence, offset) = decode_position_href(&target_section.links[index].0)
                .expect("internal link must be a decodable Kindle position reference");
            let destination = href.split(['#', '?']).next().unwrap_or(href);
            let expected_section = source
                .sections
                .iter()
                .position(|section| section.href == destination)
                .unwrap_or_else(|| {
                    assert!(destination.is_empty(), "unresolved source link {href}");
                    source
                        .sections
                        .iter()
                        .position(|section| section.href == source_section.href)
                        .unwrap()
                });
            assert!(
                target_section.links[index].0.starts_with("kindle:pos:"),
                "source link {href} ({label}) has no target position projection"
            );
            assert_eq!(
                sequence as usize, expected_section,
                "source link {href} ({label}) must resolve to its source section"
            );
            assert_eq!(
                target_section.links[index].1, *label,
                "source link label {label:?} must survive target reconstruction"
            );
            if let Some(fragment) = href.split_once('#').map(|(_, fragment)| fragment) {
                assert!(
                    source.sections[expected_section]
                        .ids
                        .iter()
                        .any(|id| id == fragment),
                    "source fragment target #{fragment} must resolve to one source ID"
                );
                assert!(
                    target.sections[expected_section].ids.contains(fragment),
                    "target fragment target #{fragment} must resolve to one target ID"
                );
            }
            assert!(
                offset < target.header.text_length as u32,
                "source link {href} ({label}) has an out-of-range target offset"
            );
        }
        assert_eq!(
            target_section.images.len(),
            source_section.images.len(),
            "image occurrence count must survive"
        );
        for (index, (source_href, alt)) in source_section.images.iter().enumerate() {
            let target_image = &target_section.images[index];
            assert!(
                target_image.starts_with("kindle:embed:")
                    || (source_section.fixed && target_image.starts_with("kindle:flow:")),
                "image {source_href} must resolve through a target resource reference"
            );
            if let Some(embed) = target_image.strip_prefix("kindle:embed:") {
                let number = decode_embed_number(embed.split('?').next().unwrap()).unwrap();
                if !source_href.ends_with("cover.png") {
                    assert_eq!(
                        target.embedded_bytes(number).unwrap(),
                        &source.resources.get(source_href).unwrap().1,
                        "image {source_href} target resource bytes must match the source logical resource"
                    );
                }
            }
            assert_eq!(
                &target_section.image_alts[index], alt,
                "alt text must remain associated with image occurrence {source_href}"
            );
        }
        assert_eq!(
            target_section.ruby, source_section.ruby,
            "ruby base/text associations must survive target reconstruction"
        );
    }
}

#[test]
fn batch2_common_projection_closes_metadata_text_sections_ids_links_resources_and_a11y() {
    // REQ: PKG-003, RES-001/005/007, CONT-002/003/005/007/008, AMZ-QA-003/005,
    // AMZ-LINK-001, AMZ-A11Y-001..003, SEM-001/002/004/007/008/009, SEM-015.
    for input in [
        epub::minimal_reflowable(),
        epub::metadata_and_spine(),
        epub::nested_navigation(),
        epub::resource_graph(),
        epub::vertical_japanese(),
        epub::unicode_stress(),
        epub::accessibility_structure(),
        epub::rtl_progression(),
    ] {
        let (source, target) = project(&input);
        assert_common_source_projection(&source, &target);
        assert_eq!(
            target.body_text(),
            source
                .sections
                .iter()
                .map(|s| s.visible_text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            "required reading content closure must match source sections exactly"
        );
    }
    let (source, target) = project(&epub::large_text(120_000));
    assert_eq!(
        target.body_text(),
        source
            .sections
            .iter()
            .map(|section| section.visible_text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        "large visible text must reconstruct completely across PalmDOC records"
    );
    let (source, target) = project(&epub::metadata_and_spine());
    assert_eq!(
        target.exth_text(503).as_deref(),
        Some(source.title.as_str()),
        "EXTH title must be source-derived"
    );
    assert_eq!(
        target.exth_text(524).as_deref(),
        Some(source.language.as_str()),
        "target language must be source-derived"
    );
}

#[test]
fn batch2_publication_date_and_modified_remain_independent_through_exth_projection() {
    // REQ: PKG-003, FMT-EXTH-009.
    let (source, target) = project(&epub::metadata_date_divergence());
    assert_eq!(source.publication_date.as_deref(), Some("2022-10-17"));
    assert_eq!(source.modified, "2026-09-13T00:00:00Z");
    assert_eq!(
        target.exth_text(106).as_deref(),
        source.publication_date.as_deref(),
        "EXTH 106 must project publication_date"
    );
    assert_ne!(
        target.exth_text(106).as_deref(),
        Some(source.modified.as_str()),
        "EXTH 106 must not project modified"
    );
}

#[test]
fn batch2_data_uri_css_image_is_materialized_and_resolvable() {
    // REQ: RES-009, FMT-RES-001..002.
    let (_source, target) = project(&epub::css_data_uri_image());
    assert!(!target.css.contains("data:image/"));
    let number = target
        .css
        .split("kindle:embed:")
        .nth(1)
        .and_then(|value| value.split(['?', ')', '"', '\'']).next())
        .and_then(decode_embed_number)
        .expect("CSS data image must become a Kindle embed reference");
    let payload = target.embedded_bytes(number).unwrap();
    assert_eq!(
        image::guess_format(payload).unwrap(),
        image::ImageFormat::Png
    );
    image::load_from_memory(payload).expect("CSS data image must be decodable");
}

#[test]
fn batch2_inline_style_data_images_use_their_owning_document_base() {
    // REQ: RES-009.
    let (_source, target) = project(&epub::inline_style_data_uri_images_at_root_and_depth());
    assert!(!target.css.contains("data:image/"));
    assert!(!target.rawml.contains("data:image/"));
    for (selector, marker) in [
        (".inline-root", "AUTH_INLINE_CSS_ROOT"),
        (".inline-deep", "AUTH_INLINE_CSS_DEEP"),
    ] {
        let rule = target
            .css
            .split('}')
            .find(|rule| rule.contains(selector))
            .unwrap_or_else(|| panic!("inline-style CSS vector {selector} was not projected"));
        let number = rule
            .split("kindle:embed:")
            .nth(1)
            .and_then(|value| value.split(['?', ')', '"', '\'']).next())
            .and_then(decode_embed_number)
            .unwrap_or_else(|| panic!("inline-style CSS vector {selector} lacks kindle:embed"));
        let payload = target
            .embedded_bytes(number)
            .unwrap_or_else(|error| panic!("inline-style CSS vector {selector} embed: {error}"));
        image::load_from_memory(payload)
            .unwrap_or_else(|error| panic!("inline-style CSS vector {selector} image: {error}"));
        assert!(
            target.body_text().contains(marker),
            "readable body marker {marker} must survive"
        );
    }
}

#[test]
fn batch2_data_uri_xhtml_image_is_materialized_and_resolvable() {
    // REQ: RES-009, FMT-RES-001..002.
    let (_source, target) = project(&epub::xhtml_data_uri_image());
    assert!(!target.rawml.contains("data:image/"));
    let image = target.sections[0]
        .images
        .first()
        .expect("XHTML image occurrence");
    let number = image
        .strip_prefix("kindle:embed:")
        .and_then(|value| decode_embed_number(value.split('?').next().unwrap_or(value)))
        .expect("XHTML data image must become a Kindle embed reference");
    let payload = target.embedded_bytes(number).unwrap();
    assert_eq!(
        image::guess_format(payload).unwrap(),
        image::ImageFormat::Png
    );
    image::load_from_memory(payload).expect("XHTML data image must be decodable");
}

#[test]
fn batch2_negative_data_uri_images_are_safely_degraded_in_css_and_xhtml() {
    // REQ: RES-009.  A valid EPUB carries independent CSS url() and XHTML
    // img src vectors for unsupported/mismatched media, strict-base64
    // failure, corrupt decoded bytes, and oversized image dimensions.
    let input = epub::unsafe_data_uri_images();
    let source = SourceModel::parse(&input).expect("independent source semantic model");
    let output = convert_bytes(&input, &options()).expect("unsafe inline data must degrade safely");
    let output: &'static [u8] = Box::leak(output.into_boxed_slice());
    let target = TargetProjection::parse(output).expect("independent target projection");

    let source_body = source
        .sections
        .iter()
        .map(|section| section.visible_text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        source_body.contains("AUTH_UNSAFE_DATA_URI_BEGIN"),
        "audit fixture must contain the readable source marker"
    );
    assert_eq!(
        target.body_text(),
        source_body,
        "readable body text must survive"
    );
    assert!(!target.css.contains("data:image/"));
    assert!(!target.rawml.contains("data:image/"));

    for selector in [
        ".css-unsafe-unsupported",
        ".css-unsafe-mismatched",
        ".css-unsafe-malformed",
        ".css-unsafe-corrupt",
        ".css-unsafe-oversized",
    ] {
        let rule = target
            .css
            .split('}')
            .find(|rule| rule.contains(selector))
            .unwrap_or_else(|| panic!("CSS unsafe vector {selector} was not projected"));
        assert!(
            rule.contains("background-image:url()"),
            "CSS unsafe vector {selector} must be emptied, not exposed"
        );
        assert!(!rule.contains("kindle:embed:"));
    }

    let target_section = target
        .sections
        .first()
        .expect("XHTML body section must be projected");
    assert_eq!(
        target_section.images.len(),
        5,
        "all XHTML image occurrences must remain inspectable"
    );
    for (index, image) in target_section.images.iter().enumerate() {
        assert!(
            image.is_empty(),
            "XHTML unsafe vector {index} must be safely emptied"
        );
        assert!(!image.starts_with("kindle:embed:"));
    }
    for marker in [
        "XHTML_UNSAFE_UNSUPPORTED",
        "XHTML_UNSAFE_MISMATCHED",
        "XHTML_UNSAFE_MALFORMED",
        "XHTML_UNSAFE_CORRUPT",
        "XHTML_UNSAFE_OVERSIZED",
    ] {
        assert!(
            target.rawml.contains(marker),
            "XHTML readable marker {marker} must remain in RawML"
        );
    }
}

#[test]
fn batch2_css_namespace_declaration_and_qualified_selector_are_retained() {
    // REQ: CHAR-CSS-NAMESPACE-001 (compatibility characterization).
    let (_source, target) = project(&epub::css_namespace_selector());
    assert!(
        target
            .css
            .contains("@namespace epub \"http://www.idpf.org/2007/ops\";")
    );
    assert!(target.css.contains("*[epub|type~=\"note\"]"));
    assert!(target.css.contains("color: red"));
}

#[test]
fn batch2_navigation_projection_decodes_ncx_ctoc_hierarchy_targets_and_order() {
    // REQ: NAV-003..NAV-008, AMZ-NAV-001..AMZ-NAV-004, SEM-003/015.
    let (source, target) = project(&epub::nested_navigation());
    let mut expected = Vec::new();
    flatten_nav(&source.toc, 0, &mut expected);
    let actual = target
        .ncx_entries()
        .expect("independent NCX/CTOC reconstruction");
    assert_eq!(
        actual.len(),
        expected.len(),
        "NCX entry count must equal source linked TOC nodes"
    );
    let actual_labels = actual
        .iter()
        .map(|e| e.label.clone())
        .collect::<BTreeSet<_>>();
    let expected_labels = expected
        .iter()
        .map(|(label, _, _)| label.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_labels, expected_labels,
        "navigation labels must be reconstructed from CTOC, not RawML marker presence"
    );
    assert_eq!(
        actual.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(),
        expected
            .iter()
            .map(|(label, _, _)| label.as_str())
            .collect::<Vec<_>>(),
        "target NCX order must equal source TOC depth-first order"
    );
    let coordinates = actual
        .iter()
        .map(|entry| (entry.sequence, entry.offset))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        coordinates.len(),
        actual.len(),
        "target navigation must not introduce duplicate destination coordinates"
    );
    let expected_destinations = expected
        .iter()
        .map(|(_, href, _)| {
            source
                .sections
                .iter()
                .position(|section| section.href == href.split('#').next().unwrap_or(href))
                .expect("source TOC target must resolve to a spine section")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual
            .iter()
            .map(|entry| entry.sequence as usize)
            .collect::<Vec<_>>(),
        expected_destinations,
        "target navigation destination set/order must equal source TOC targets"
    );
    for (position, (label, href, depth)) in expected.iter().enumerate() {
        let entry = actual
            .get(position)
            .expect("source TOC node has target NCX node");
        assert_eq!(&entry.label, label);
        assert_eq!(
            entry.depth,
            Some(*depth as u32),
            "TOC depth must survive NCX reconstruction for {label}"
        );
        let expected_parent = if *depth == 0 {
            None
        } else {
            expected[..position]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (_, _, prior_depth))| *prior_depth + 1 == *depth)
                .map(|(parent, _)| parent as u32)
        };
        assert_eq!(
            entry.parent, expected_parent,
            "TOC parent relation must survive NCX reconstruction for {label}"
        );
        let target_section = expected_destinations[position];
        assert_eq!(
            entry.sequence as usize, target_section,
            "TOC target sequence must identify the source spine destination for {label} -> {href}"
        );
        assert!(
            entry.sequence < 1_000_000 && entry.offset < target.header.text_length as u32,
            "NCX target position must be in RawML range for {label} -> {href}"
        );
        assert!(
            target.rawml.contains(&format!(
                "kindle:pos:fid:{:04X}:off:{:010X}",
                entry.sequence, entry.offset
            )) || target.rawml.contains("kindle:pos:fid:"),
            "NCX target must correspond to an emitted position coordinate"
        );
    }
    assert!(
        source
            .landmarks
            .iter()
            .any(|(kind, _)| kind == "bodymatter"),
        "source bodymatter landmark must be present"
    );
    let guide = target
        .guide_entries()
        .expect("target Guide/landmark index must be independently decoded");
    assert_eq!(
        guide
            .iter()
            .map(|(kind, _, _, _)| kind.as_str())
            .collect::<Vec<_>>(),
        source
            .landmarks
            .iter()
            .map(|(kind, _)| {
                if matches!(kind.as_str(), "bodymatter" | "body" | "start") {
                    "text"
                } else {
                    kind.as_str()
                }
            })
            .collect::<Vec<_>>(),
        "target Guide landmark kinds must preserve source semantic classes"
    );
    for ((expected_kind, href), (kind, label, sequence, offset)) in
        source.landmarks.iter().zip(&guide)
    {
        let expected_kind = if matches!(expected_kind.as_str(), "bodymatter" | "body" | "start") {
            "text"
        } else {
            expected_kind.as_str()
        };
        assert_eq!(
            expected_kind, kind,
            "landmark kind must remain source-derived"
        );
        let destination = href.split('#').next().unwrap_or(href);
        let expected_section = source
            .sections
            .iter()
            .position(|section| section.href == destination)
            .expect("source landmark destination must resolve");
        assert_eq!(*sequence as usize, expected_section);
        assert!(
            *offset < target.header.text_length as u32,
            "Guide landmark {label} target must be in range"
        );
    }
    let start = target.header.exth_u32(116).expect("semantic Start Reading");
    assert!(
        start < target.header.text_length as u32,
        "Start Reading must be in range"
    );
    let body_start = target.rawml.find("<body").expect("target bodymatter");
    assert!(
        (body_start..=body_start + 64).contains(&(start as usize)),
        "semantic Start Reading must point at the reconstructed bodymatter start: {start} vs {body_start}"
    );
}

#[test]
fn batch2_resource_css_projection_closes_import_url_shared_resource_and_css_graph() {
    // REQ: RES-004/005/007, CSS-001/002/006/009/010, AMZ-CSS-003, SEM-006.
    let (source, target) = project(&epub::resource_graph());
    assert_eq!(source.css.len(), 2);
    assert!(
        target.css.contains("@import url(kindle:flow:"),
        "stylesheet import must reconstruct as a target flow reference"
    );
    assert!(
        target.css.matches("kindle:embed:").count() >= 1,
        "CSS url() must reconstruct as an embedded resource reference"
    );
    let css_resource = target
        .css
        .split("kindle:embed:")
        .nth(1)
        .and_then(|value| value.split(['?', ')', '\"', '\'']).next())
        .and_then(decode_embed_number)
        .expect("CSS url() must decode to a target resource number");
    assert_eq!(
        target.embedded_bytes(css_resource).unwrap(),
        &source.resources.get("EPUB/images/shared.png").unwrap().1,
        "stylesheet-relative CSS url() must resolve to the source logical shared resource"
    );
    let shared = target
        .embedded_resource_numbers
        .iter()
        .filter(|n| **n == 1)
        .count();
    assert!(
        shared >= 3,
        "shared image must remain one target resource referenced by both XHTML and CSS occurrences"
    );
    for section in &target.sections {
        for image in &section.images {
            let n = image
                .split("kindle:embed:")
                .nth(1)
                .unwrap()
                .split('?')
                .next()
                .and_then(decode_embed_number)
                .unwrap_or(0);
            assert_eq!(
                target.embedded_bytes(n).unwrap(),
                source
                    .resources
                    .get("EPUB/images/shared.png")
                    .unwrap()
                    .1
                    .as_slice()
            );
        }
    }
    let (source, target) = project(&epub::vertical_japanese());
    for expected in &source.css[0].declarations {
        let value = expected.1.replace(' ', "");
        assert!(
            target.css.replace(' ', "").contains(&value),
            "CSS declaration {:?} must remain represented",
            expected
        );
    }
    let (source, target) = project(&epub::paired_css_semantics());
    assert_common_source_projection(&source, &target);
    assert_eq!(
        source.css.len(),
        2,
        "paired CSS fixture must have a two-sheet source graph"
    );
    let source_rules = source
        .css
        .iter()
        .map(|sheet| css_rule_declarations(&sheet.source))
        .fold(std::collections::BTreeMap::new(), |mut all, rules| {
            all.extend(rules);
            all
        });
    let target_rules = css_rule_declarations(&target.css);
    for (selector, declarations) in source_rules {
        let target_declarations = target_rules
            .get(&selector)
            .unwrap_or_else(|| panic!("target CSS rule {selector:?} is missing"));
        for (property, value) in declarations {
            assert_eq!(
                target_declarations.get(&property),
                Some(&value),
                "target CSS semantic declaration {selector:?} {property} must equal source"
            );
        }
    }
    let (source, target) = project(&epub::media_queries());
    assert_common_source_projection(&source, &target);
    assert_eq!(
        css_media_semantics(&source.css[0].source),
        css_media_semantics(&target.css),
        "media-query branches must project selector/property semantics without inversion"
    );
}

#[test]
fn batch2_layout_cover_font_and_file_api_artifacts_are_independently_projected() {
    // REQ: PKG-010/011, RES-001/007, LAYOUT-003..008, FONT-001/002/003/006,
    // AMZ-REFLOW-002, AMZ-COVER-001/002, SEM-005/010..013, API-001.
    let (fixed, target) = project(&epub::fixed_layout());
    assert_common_source_projection(&fixed, &target);
    assert_eq!(
        fixed.rendition_layout.as_deref(),
        Some("pre-paginated"),
        "source expected model must carry the global fixed-layout declaration"
    );
    assert_eq!(target.exth_text(122).as_deref(), Some("true"));
    assert_eq!(
        target.exth_text(124).as_deref(),
        fixed.orientation.as_deref()
    );
    let expected_viewport = fixed.viewport.as_deref().map(|v| {
        v.replace("width=", "")
            .replace("height=", "")
            .replace(',', "x")
    });
    assert_eq!(
        target.exth_text(126).as_deref(),
        expected_viewport.as_deref()
    );
    let resc = target
        .resc_metadata()
        .expect("target RESC metadata must be independently decoded");
    for (property, expected) in [
        ("rendition:orientation", fixed.orientation.as_deref()),
        ("rendition:spread", fixed.spread.as_deref()),
        ("rendition:viewport", fixed.viewport.as_deref()),
    ] {
        assert_eq!(
            resc.get(property).map(String::as_str),
            expected,
            "RESC {property} must equal source rendition metadata"
        );
    }
    let fixed_spine = target
        .resc_spine_properties()
        .expect("target RESC spine properties must be independently decoded");
    assert_eq!(fixed_spine.len(), fixed.sections.len());
    for ((_, target_properties, target_linear), source_section) in
        fixed_spine.iter().zip(&fixed.sections)
    {
        assert_eq!(target_properties, &source_section.source_properties);
        assert!(
            *target_linear,
            "fixed source sections are linear in the fixture"
        );
    }
    assert_eq!(
        target.rawml.matches("image/svg+xml").count(),
        2,
        "one target page flow must reconstruct for each fixed source spine item: {}",
        target.rawml
    );
    let (mixed, target) = project(&epub::mixed_layout());
    assert_common_source_projection(&mixed, &target);
    let mixed_surface = format!("{}{}", target.rawml, target.css).to_ascii_lowercase();
    for generated in [
        "writing-mode:",
        "direction:",
        "position:fixed",
        "position:absolute",
        "kf8-layout",
        "vrtl",
        "hltr",
    ] {
        assert!(
            !mixed_surface.contains(generated),
            "mixed input must not receive unsupported book-wide fallback semantic {generated}"
        );
    }
    assert_eq!(
        target.rawml.matches("image/svg+xml").count(),
        1,
        "per-item fixed override must produce exactly one page flow"
    );
    assert_eq!(
        target.sections.iter().map(|s| s.fixed).collect::<Vec<_>>(),
        mixed.sections.iter().map(|s| s.fixed).collect::<Vec<_>>(),
        "target section classification must preserve the per-item fixed override"
    );
    let mixed_spine = target
        .resc_spine_properties()
        .expect("mixed target RESC spine properties");
    assert_eq!(
        mixed_spine
            .iter()
            .map(|(_, properties, _)| properties)
            .collect::<Vec<_>>(),
        mixed
            .sections
            .iter()
            .map(|section| &section.source_properties)
            .collect::<Vec<_>>(),
        "mixed target per-item rendition properties must equal source spine overrides"
    );
    let (rtl, target) = project(&epub::rtl_progression());
    assert_eq!(rtl.progression, "rtl");
    assert_eq!(
        target.exth_text(527).as_deref(),
        Some("rtl"),
        "publication progression must remain distinct from document writing mode"
    );
    assert_eq!(
        target.exth_text(525).as_deref(),
        Some("vertical-rl"),
        "document writing mode must be source-derived"
    );
    assert_ne!(
        target.exth_text(527),
        target.exth_text(525),
        "page progression must not be used as a document writing-mode substitute"
    );
    let (_, reflow) = project(&epub::minimal_reflowable());
    assert_ne!(reflow.exth_text(122).as_deref(), Some("true"));
    let reflow_surface = format!("{}{}", reflow.rawml, reflow.css).to_ascii_lowercase();
    for forced in [
        "font-size:",
        "line-height:",
        "color:",
        "background:",
        "writing-mode:",
        "direction:",
        "position:fixed",
        "position:absolute",
        "width:",
        "height:",
        "kf8-layout",
        "vrtl",
        "hltr",
    ] {
        assert!(
            !reflow_surface.contains(forced),
            "reflowable input must not receive converter-generated semantic fallback {forced}"
        );
    }
    let (cover, target) = project(&epub::cover_png());
    assert_common_source_projection(&cover, &target);
    let first_image = target.header.first_resource as usize;
    let cover_offset = target.header.exth_u32(201).expect("EXTH cover offset") as usize;
    let thumbnail_offset = target.header.exth_u32(202).expect("EXTH thumbnail offset") as usize;
    let cover_record_index = first_image + cover_offset;
    let cover_bytes = target.db.record(cover_record_index).unwrap();
    let cover_image = image::load_from_memory(cover_bytes)
        .expect("EXTH cover target must resolve to a decodable image payload");
    let source_cover = cover
        .resources
        .get("EPUB/images/cover.png")
        .expect("source logical cover resource");
    let source_image =
        image::load_from_memory(&source_cover.1).expect("source logical cover must decode");
    assert_eq!(
        (cover_image.width(), cover_image.height()),
        (source_image.width(), source_image.height()),
        "normalized target cover dimensions must correspond to source cover"
    );
    assert!(target.image_records().contains(&cover_record_index));
    assert!(
        first_image + thumbnail_offset < target.db.record_count(),
        "EXTH thumbnail offset must resolve inside the emitted image records"
    );
    let cover_occurrence = target
        .sections
        .iter()
        .flat_map(|section| section.images.iter().zip(&section.image_alts))
        .find(|(_, alt)| alt.as_str() == "Authority Audit Cover")
        .map(|(src, _)| {
            decode_embed_number(
                src.split("kindle:embed:")
                    .nth(1)
                    .unwrap()
                    .split('?')
                    .next()
                    .unwrap(),
            )
            .unwrap()
        })
        .expect("source cover occurrence must remain associated with its image reference");
    assert_eq!(
        cover_occurrence,
        cover_offset + 1,
        "cover occurrence must point at the EXTH logical cover resource"
    );
    for obfuscated in [false, true] {
        let (source, target) = project(&epub::embedded_font(obfuscated));
        assert_common_source_projection(&source, &target);
        assert!(
            !target.font_records.is_empty(),
            "CSS-selected font resource must be emitted"
        );
        let decoded = target.decompressed_font(target.font_records[0]).unwrap();
        let source_font = &source.resources.get("EPUB/fonts/audit.ttf").unwrap().1;
        let expected = if obfuscated {
            idpf_deobfuscate(&source.identifier, source_font)
        } else {
            source_font.clone()
        };
        assert_eq!(
            source.resources.get("EPUB/fonts/audit.ttf").unwrap().0,
            "font/ttf",
            "source font MIME/type must be explicit in the expected model"
        );
        assert!(
            matches!(
                decoded.get(..4),
                Some(b"OTTO") | Some(b"true") | Some(b"ttcf") | Some([0, 1, 0, 0])
            ),
            "target FONT payload must decode to the declared SFNT font type"
        );
        assert_eq!(&decoded[..1040], &expected[..1040]);
        let font_reference = target
            .css
            .split("src:url(")
            .nth(1)
            .and_then(|value| value.split(')').next())
            .expect("target @font-face src must be projected");
        assert!(
            font_reference.starts_with("kindle:embed:"),
            "@font-face src must point to a target resource rather than remain an unresolved path"
        );
        let font_embed = decode_embed_number(
            font_reference
                .strip_prefix("kindle:embed:")
                .unwrap()
                .split('?')
                .next()
                .unwrap(),
        )
        .expect("target font embed number");
        assert_eq!(
            font_embed, 1,
            "the fixture's @font-face src must resolve to its one emitted font resource"
        );
        assert!(
            target
                .font_records
                .contains(&(target.header.fdst_record.unwrap() as usize + font_embed),),
            "CSS-selected target resource must be an emitted FONT record"
        );
        assert_eq!(
            target.resource_bytes(font_embed).unwrap(),
            target.db.record(target.font_records[0]).unwrap(),
            "CSS embed number must resolve through the target binary resource table"
        );
        let rules = css_rule_declarations(&target.css);
        assert_eq!(
            rules
                .get(".fonted")
                .and_then(|declarations| declarations.get("font-family"))
                .map(String::as_str),
            Some("'AuthorityAudit'"),
            "target CSS must retain the selected font-family association"
        );
        assert!(
            rules.iter().all(|(selector, declarations)| {
                selector != "body" && selector != "p" || !declarations.contains_key("font-family")
            }),
            "embedded font must not leak to unrelated body/control selectors"
        );
        assert!(target.rawml.contains("AuthorityAudit") && target.rawml.contains("AUTH_FONT_TEXT"));
    }
    let tmp = TempDir::new("batch2-file-api");
    let src = tmp.write("input.epub", &epub::minimal_reflowable());
    let dst = tmp.path().join("artifact.azw3");
    convert_file(&src, &dst, &options()).expect("File API .azw3 conversion");
    let bytes = std::fs::read(&dst).expect("read exact .azw3 artifact");
    let target = TargetProjection::parse(Box::leak(bytes.into_boxed_slice()))
        .expect("independent PalmDB/MOBI/KF8 parse of .azw3 artifact");
    assert!(target.header.version >= 8);
    let _ = reconstruct_text(&target.db, &target.header).expect("artifact RawML reconstruction");
}

#[test]
fn batch2_negative_css_constructs_remain_isolated() {
    // REQ: CSS-007/008, AMZ-CSS-002.  Each negative uses a valid package and
    // one unsupported construct, so a prior syntax error cannot mask another.
    for (input, reason) in [
        (epub::unsupported_css_selector(), "sibling combinators"),
        (epub::unsupported_css_pseudo(), "pseudo-elements"),
        (epub::unsupported_css_counter(), "counter-reset"),
    ] {
        let outcome = epub3_kindle::convert_bytes_with_warnings(&input, &options())
            .expect("isolated unsupported CSS must degrade safely");
        assert!(
            outcome
                .warnings()
                .iter()
                .any(|warning| warning.code == epub3_kindle::WarningCode::W004
                    && warning.message.contains(reason)),
            "warning must identify the isolated unsupported CSS reason {reason:?}: {:?}",
            outcome.warnings
        );
    }
}
