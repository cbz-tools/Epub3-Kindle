//! Focused closure tests for the currently reopened authority rows.

use std::collections::{BTreeMap, BTreeSet};

use epub3_kindle::{Compression, ConvertOptions, convert_bytes, convert_file};
use image::GenericImageView;

use crate::audit_support::epub;
use crate::audit_support::semantic::{
    NavItem, SourceModel, TargetProjection, decode_embed_number, source_document,
    source_local_resource_edges, source_semantic_tree,
};
use crate::audit_support::temp::TempDir;

fn plain() -> ConvertOptions {
    ConvertOptions {
        compression: Compression::None,
    }
}

fn viewport_resolution(value: &str) -> String {
    let mut width = None;
    let mut height = None;
    for token in value.replace([',', ';'], " ").split_whitespace() {
        let (name, dimension) = token
            .split_once('=')
            .unwrap_or_else(|| panic!("malformed source viewport token {token:?}"));
        match name.to_ascii_lowercase().as_str() {
            "width" => width = Some(dimension.to_owned()),
            "height" => height = Some(dimension.to_owned()),
            other => panic!("unexpected source viewport dimension {other:?}"),
        }
    }
    let width = width.expect("source viewport width");
    let height = height.expect("source viewport height");
    assert!(width.parse::<u32>().is_ok(), "width must be numeric");
    assert!(height.parse::<u32>().is_ok(), "height must be numeric");
    format!("{width}x{height}")
}

fn target(input: &[u8]) -> TargetProjection<'static> {
    let output = convert_bytes(input, &plain()).expect("authority fixture converts");
    TargetProjection::parse(Box::leak(output.into_boxed_slice()))
        .expect("target parses")
        .to_owned()
}

fn dual_target(
    input: &[u8],
    label: &str,
) -> (TargetProjection<'static>, TargetProjection<'static>) {
    let temp = TempDir::new(label);
    let source = temp.write("input.epub", input);
    let destination = temp.path().join("output.mobi");
    convert_file(&source, &destination, &plain()).expect("Dual fixture converts");
    let output = std::fs::read(destination).expect("Dual output bytes");
    let leaked = Box::leak(output.into_boxed_slice());
    let db = crate::audit_support::palm::PalmDb::parse(leaked).expect("Dual PalmDB");
    let kf7 = db.mobi_header(0).expect("KF7 header");
    let kf8_index = kf7.exth_u32(121).expect("KF8 boundary") as usize;
    (
        TargetProjection::parse_at(leaked, 0).expect("KF7 projection"),
        TargetProjection::parse_at(leaked, kf8_index).expect("KF8 projection"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    tag: String,
    id: Option<String>,
    classes: BTreeSet<String>,
    inline: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct CssRule {
    selector: String,
    declarations: BTreeMap<String, String>,
    order: usize,
}

fn css_rules(source: &str, branch: &str) -> Vec<CssRule> {
    fn collect(source: &str, branch: &str, output: &mut Vec<CssRule>, order: &mut usize) {
        let mut cursor = 0;
        while let Some(relative) = source[cursor..].find('{') {
            let open = cursor + relative;
            let start = source[..open]
                .rfind(['}', '{', ';'])
                .map(|position| position + 1)
                .unwrap_or(0);
            let prelude = source[start..open].trim();
            let Some(close) = matching_brace(source, open) else {
                break;
            };
            let body = &source[open + 1..close];
            if prelude.to_ascii_lowercase().starts_with("@media") {
                let enabled = prelude
                    .to_ascii_lowercase()
                    .contains(&branch.to_ascii_lowercase());
                if enabled {
                    collect(body, branch, output, order);
                }
            } else if !prelude.starts_with('@') {
                let declarations = body
                    .split(';')
                    .filter_map(|part| {
                        let (name, value) = part.split_once(':')?;
                        let name = name.trim().to_ascii_lowercase();
                        let value = value.trim().to_owned();
                        (!name.is_empty() && !value.is_empty()).then_some((name, value))
                    })
                    .collect::<BTreeMap<_, _>>();
                for selector in prelude.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    output.push(CssRule {
                        selector: selector.to_owned(),
                        declarations: declarations.clone(),
                        order: *order,
                    });
                    *order += 1;
                }
            }
            cursor = close + 1;
        }
    }
    let mut output = Vec::new();
    let mut order = 0;
    collect(source, branch, &mut output, &mut order);
    output
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, byte) in source.as_bytes().iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn probe_elements(source: &str) -> Vec<Probe> {
    let mut probes = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("<p") {
        let start = cursor + relative;
        let Some(end_relative) = source[start..].find('>') else {
            break;
        };
        let end = start + end_relative + 1;
        let tag = &source[start..end];
        if source
            .as_bytes()
            .get(start + 2)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b':' || *byte == b'-')
        {
            cursor = end;
            continue;
        }
        let class_value = attribute(tag, "class").unwrap_or_default();
        let inline = attribute(tag, "style")
            .unwrap_or_default()
            .split(';')
            .filter_map(|part| {
                let (name, value) = part.split_once(':')?;
                Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            })
            .collect();
        probes.push(Probe {
            tag: "p".to_owned(),
            id: attribute(tag, "id"),
            classes: class_value
                .split_whitespace()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
            inline,
        });
        cursor = end;
    }
    probes
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    let start = loop {
        let relative = lower[search..].find(&needle)?;
        let position = search + relative;
        let boundary = position == 0
            || lower
                .as_bytes()
                .get(position - 1)
                .is_some_and(u8::is_ascii_whitespace);
        if boundary {
            break position + needle.len();
        }
        search = position + needle.len();
    };
    let quote = tag.as_bytes().get(start).copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let value_start = start + 1;
    let value_end = value_start + tag[value_start..].find(quote as char)?;
    Some(tag[value_start..value_end].to_owned())
}

fn selector_specificity(selector: &str, probe: &Probe) -> Option<(usize, usize, usize)> {
    let selector = selector.trim();
    if selector.contains([' ', '>', '+', '~', ':']) {
        return None;
    }
    let mut tag = None;
    let mut ids = 0;
    let mut classes = 0;
    let mut cursor = 0;
    for part in selector.split(['#', '.']) {
        if part.is_empty() {
            continue;
        }
        let prefix = selector.as_bytes().get(cursor).copied();
        match prefix {
            Some(b'#') => {
                if probe.id.as_deref() == Some(part) {
                    ids += 1;
                } else {
                    return None;
                }
            }
            Some(b'.') => {
                if probe.classes.contains(part) {
                    classes += 1;
                } else {
                    return None;
                }
            }
            _ => tag = Some(part),
        }
        cursor += part.len() + 1;
    }
    if tag.is_some_and(|value| !value.eq_ignore_ascii_case(&probe.tag)) {
        return None;
    }
    Some((ids, classes, usize::from(tag.is_some())))
}

type SelectedCss = BTreeMap<String, ((usize, usize, usize), usize, String)>;

fn effective_css(rules: &[CssRule], probe: &Probe) -> BTreeMap<String, String> {
    let mut selected: SelectedCss = BTreeMap::new();
    for rule in rules {
        let Some(specificity) = selector_specificity(&rule.selector, probe) else {
            continue;
        };
        for (property, value) in &rule.declarations {
            let replace = selected
                .get(property)
                .is_none_or(|(old_specificity, old_order, _)| {
                    specificity > *old_specificity
                        || (specificity == *old_specificity && rule.order > *old_order)
                });
            if replace {
                selected.insert(property.clone(), (specificity, rule.order, value.clone()));
            }
        }
    }
    for (property, value) in &probe.inline {
        selected.insert(property.clone(), ((1000, 0, 0), usize::MAX, value.clone()));
    }
    selected
        .into_iter()
        .map(|(property, (_, _, value))| (property, value))
        .collect()
}

#[test]
fn fixed_layout_exth_126_preserves_explicit_and_derives_common_page_viewport() {
    // REQ: FMT-EXTH-005. Expected values come only from the two fixed source
    // recipes and an independent source-side viewport decoder.
    let explicit_input = epub::fixed_layout();
    let explicit_source = SourceModel::parse(&explicit_input).expect("explicit source model");
    assert_eq!(
        explicit_source.original_resolution.as_deref(),
        Some("1200x800")
    );
    let explicit_target = target(&explicit_input);
    assert_eq!(
        explicit_target.exth_text(126).as_deref(),
        explicit_source.original_resolution.as_deref(),
        "explicit publication resolution must be preserved"
    );

    let inferred_input = epub::fixed_layout_inferred_resolution();
    let inferred_source = SourceModel::parse(&inferred_input).expect("inferred source model");
    assert_eq!(
        inferred_source.original_resolution, None,
        "the inference fixture must not carry Amazon legacy resolution metadata"
    );
    let page_resolutions = inferred_source
        .sections
        .iter()
        .filter(|section| section.fixed)
        .map(|section| viewport_resolution(section.viewport.as_deref().expect("fixed viewport")))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        page_resolutions,
        BTreeSet::from([String::from("1200x800")]),
        "all fixed pages must independently agree before publication inference"
    );
    let inferred_target = target(&inferred_input);
    assert_eq!(
        inferred_target.exth_text(126).as_deref(),
        Some("1200x800"),
        "EXTH 126 must be projected from the common fixed-page viewport"
    );
}

#[test]
fn a11y_structure_reconstructs_parent_child_relationships() {
    // REQ: SEM-008.
    let input = epub::accessibility_structure();
    let source = source_semantic_tree(&input, "EPUB/text/a11y.xhtml").expect("source tree");
    let target = target(&input);
    assert_eq!(
        target.semantic_tree(0).unwrap(),
        source,
        "target structural reconstruction must preserve the source parent-child tree"
    );
}

#[test]
fn joint_cover_toc_bodymatter_relationship_is_consistent() {
    // REQ: AMZ-NAV-003.
    let input = epub::cover_png();
    let source = SourceModel::parse(&input).expect("cover source model");
    let target = target(&input);
    let cover = source
        .resources
        .get("EPUB/images/cover.png")
        .expect("source logical cover");
    let cover_offset = target.header.exth_u32(201).expect("EXTH cover offset") as usize;
    let cover_record_index = target
        .header
        .first_resource
        .checked_add(cover_offset as u32)
        .expect("cover resource index") as usize;
    let cover_record = target
        .db
        .record(cover_record_index)
        .expect("logical cover record");
    let source_dimensions = image::load_from_memory(&cover.1)
        .expect("source cover decoder")
        .dimensions();
    let target_dimensions = image::load_from_memory(cover_record)
        .expect("target cover decoder")
        .dimensions();
    assert_eq!(
        target_dimensions, source_dimensions,
        "joint cover route must resolve to the logical cover image"
    );
    let ncx = target.ncx_entries().expect("target NCX");
    assert_eq!(ncx.len(), 1);
    assert_eq!(ncx[0].label, source.toc[0].label);
    assert_eq!(ncx[0].sequence, 1, "TOC must target bodymatter after cover");
    let guide = target.guide_entries().expect("target Guide");
    let bodymatter = source
        .landmarks
        .iter()
        .find(|(kind, _)| kind.eq_ignore_ascii_case("bodymatter"))
        .expect("source bodymatter landmark");
    let bodymatter_sequence = source
        .sections
        .iter()
        .position(|section| section.href == bodymatter.1)
        .expect("bodymatter source section");
    let guide_bodymatter = guide
        .iter()
        .find(|(kind, _, _, _)| kind.eq_ignore_ascii_case("text"))
        .expect("Guide bodymatter route");
    assert_eq!(guide_bodymatter.2 as usize, bodymatter_sequence);
    assert_eq!(ncx[0].sequence as usize, bodymatter_sequence);
    let start_reading = target.header.exth_u32(116).expect("Start Reading");
    assert!(start_reading < target.header.text_length as u32);
    let bodymatter_html = target
        .rawml
        .match_indices("<html")
        .nth(bodymatter_sequence)
        .map(|(offset, _)| offset)
        .expect("bodymatter target section");
    assert!(
        start_reading as usize >= bodymatter_html,
        "Start Reading must enter the same bodymatter route as Guide/TOC"
    );
}

#[test]
fn css_graph_order_and_effective_cascade_match_the_source_subset() {
    // REQ: CSS-001, SEM-006, AMZ-CSS-003.
    let input = epub::css_cascade_stress();
    let source = SourceModel::parse(&input).expect("CSS stress source model");
    assert_eq!(
        source
            .css
            .iter()
            .map(|sheet| sheet.href.as_str())
            .collect::<Vec<_>>(),
        vec!["EPUB/styles/base.css", "EPUB/styles/main.css"],
        "source stylesheet order is the expected authority"
    );
    assert_eq!(source.css[1].imports, vec!["EPUB/styles/base.css"]);
    let source_css = source
        .css
        .iter()
        .map(|sheet| sheet.source.as_str())
        .collect::<String>();
    let source_document = source_document(&input, "EPUB/text/ch1.xhtml").expect("source XHTML");
    let source_probes = probe_elements(&source_document);
    assert_eq!(source_probes.len(), 2);
    let target = target(&input);
    assert!(target.rawml.contains("kindle:flow:"));
    assert!(target.css.contains("@import url(kindle:flow:"));
    let target_probes = probe_elements(&target.rawml);
    assert_eq!(target_probes, source_probes);
    let source_rules = css_rules(&source_css, "amzn-kf8");
    let target_rules = css_rules(&target.css, "amzn-kf8");
    let source_effective = source_probes
        .iter()
        .map(|probe| effective_css(&source_rules, probe))
        .collect::<Vec<_>>();
    let target_effective = target_probes
        .iter()
        .map(|probe| effective_css(&target_rules, probe))
        .collect::<Vec<_>>();
    assert_eq!(
        target_effective, source_effective,
        "target effective CSS must preserve graph order, specificity, later-wins, inline style, and KF8 branch semantics"
    );
    assert_eq!(
        target_effective[0].get("color").map(String::as_str),
        Some("black"),
        "inline style must win for the id probe"
    );
    assert_eq!(
        target_effective[1].get("color").map(String::as_str),
        Some("orange"),
        "same-specificity later rule must win for the class probe"
    );
    assert_eq!(
        target_effective[0].get("font-weight").map(String::as_str),
        Some("bold"),
        "amzn-kf8 branch must apply to the KF8 projection"
    );
    assert_ne!(
        target_effective[0].get("font-weight").map(String::as_str),
        Some("normal"),
        "amzn-mobi branch must not override the KF8 projection"
    );
}

#[test]
fn dual_media_branches_are_projected_only_in_the_intended_rendition() {
    // REQ: AMZ-CSS-003.
    let input = epub::media_queries();
    let source = SourceModel::parse(&input).expect("media source model");
    let source_document = source_document(&input, "EPUB/text/ch1.xhtml").expect("source XHTML");
    let source_probe = probe_elements(&source_document)
        .into_iter()
        .next()
        .expect("source media probe");
    let (kf7, kf8) = dual_target(&input, "batch4-media");
    let source_kf8 = effective_css(&css_rules(&source.css[0].source, "amzn-kf8"), &source_probe);
    let source_mobi = effective_css(
        &css_rules(&source.css[0].source, "amzn-mobi"),
        &source_probe,
    );
    let target_probe = probe_elements(&kf8.rawml)
        .into_iter()
        .next()
        .expect("KF8 media probe");
    let target_kf8 = effective_css(&css_rules(&kf8.css, "amzn-kf8"), &target_probe);
    let target_mobi_branch = effective_css(&css_rules(&kf8.css, "amzn-mobi"), &target_probe);
    assert_eq!(target_kf8, source_kf8);
    assert_eq!(target_mobi_branch, source_mobi);
    assert_eq!(
        target_kf8.get("font-weight").map(String::as_str),
        Some("bold")
    );
    assert_eq!(
        target_kf8.get("font-size"),
        None,
        "amzn-mobi font-size must not enter the KF8 effective branch"
    );
    assert!(
        !kf7.rawml.contains("amzn-kf8") && !kf7.rawml.contains("amzn-mobi"),
        "KF7 compatibility stub must not carry KF8 CSS branches"
    );
}

fn embedded_numbers(source: &str) -> Vec<usize> {
    source
        .split("kindle:embed:")
        .skip(1)
        .filter_map(|value| value.split(['?', '"', '\'', ')', ' ']).next())
        .filter_map(decode_embed_number)
        .collect()
}

fn target_resource_matches(
    target: &TargetProjection<'_>,
    media_type: &str,
    source_bytes: &[u8],
    number: usize,
) -> bool {
    if media_type.starts_with("font/") {
        return target.font_records.iter().any(|record| {
            target
                .decompressed_font(*record)
                .is_ok_and(|bytes| bytes == source_bytes)
        });
    }
    let Ok(emitted) = target.embedded_bytes(number) else {
        return false;
    };
    if media_type.starts_with("image/") {
        let Ok(source_image) = image::load_from_memory(source_bytes) else {
            return false;
        };
        let Ok(target_image) = image::load_from_memory(emitted) else {
            return false;
        };
        source_image.dimensions() == target_image.dimensions()
    } else {
        emitted == source_bytes
    }
}

#[test]
fn all_local_xhtml_svg_css_and_font_edges_resolve_to_target_resources() {
    // REQ: RES-007, CONT-004.
    for (label, input) in [
        ("resource", epub::resource_graph()),
        ("svg", epub::svg_reference_graph()),
        ("cover", epub::cover_png()),
        ("font", epub::embedded_font(false)),
    ] {
        let source = SourceModel::parse(&input).expect("source resource model");
        let edges = source_local_resource_edges(&input).expect("source local edge inventory");
        assert!(
            !edges.is_empty(),
            "{label} fixture must have local resource edges"
        );
        let target = target(&input);
        let target_refs = format!("{}{}", target.rawml, target.css);
        for edge in &edges {
            let (media_type, bytes) = source.resources.get(&edge.target).unwrap_or_else(|| {
                panic!(
                    "{label} edge target is not a manifest resource: {}",
                    edge.target
                )
            });
            if media_type.eq_ignore_ascii_case("text/css") {
                assert!(
                    target_refs.contains("kindle:flow:"),
                    "{label} stylesheet edge {} -> {} must resolve to a target flow",
                    edge.owner,
                    edge.target
                );
            } else {
                let matching = embedded_numbers(&target_refs)
                    .into_iter()
                    .any(|number| target_resource_matches(&target, media_type, bytes, number));
                assert!(
                    matching,
                    "{label} local edge {} -> {} must resolve to an emitted target resource",
                    edge.owner, edge.target
                );
            }
        }
        for number in embedded_numbers(&target_refs) {
            assert!(
                target.embedded_bytes(number).is_ok()
                    || target
                        .font_records
                        .iter()
                        .any(|record| target.decompressed_font(*record).is_ok()),
                "{label} emitted embed {number} must resolve to a bounded resource"
            );
        }
    }
}

#[test]
fn resc_values_and_absent_resource_declarations_agree_with_actual_graphs() {
    // REQ: FMT-RESC-001.
    let fixed_input = epub::fixed_layout();
    let fixed_source = SourceModel::parse(&fixed_input).expect("fixed source model");
    let fixed_target = target(&fixed_input);
    let fixed_resc = fixed_target.resc_metadata().expect("fixed RESC metadata");
    for (property, expected) in [
        ("rendition:orientation", fixed_source.orientation.as_deref()),
        ("rendition:spread", fixed_source.spread.as_deref()),
        ("rendition:viewport", fixed_source.viewport.as_deref()),
    ] {
        assert_eq!(
            fixed_resc.get(property).map(String::as_str),
            expected,
            "fixed RESC value {property} must equal source expected semantics"
        );
    }
    assert_eq!(
        fixed_target.resc_resource_hrefs().unwrap(),
        Vec::<String>::new(),
        "RESC must not invent a resource graph absent from its emitted XML"
    );

    let cover_input = epub::cover_png();
    let cover_source = SourceModel::parse(&cover_input).expect("cover source model");
    let cover_target = target(&cover_input);
    assert!(cover_target.resc_resource_hrefs().unwrap().is_empty());
    let cover_bytes = cover_source
        .resources
        .get("EPUB/images/cover.png")
        .expect("cover bytes");
    let cover_offset = cover_target
        .header
        .exth_u32(201)
        .expect("EXTH cover offset") as usize;
    let cover_record_index = cover_target
        .header
        .first_resource
        .checked_add(cover_offset as u32)
        .expect("cover resource index") as usize;
    let cover_record = cover_target
        .db
        .record(cover_record_index)
        .expect("logical cover record");
    let source_cover = image::load_from_memory(&cover_bytes.1).expect("source cover decoder");
    let target_cover = image::load_from_memory(cover_record).expect("target cover decoder");
    assert_eq!(
        (target_cover.width(), target_cover.height()),
        (source_cover.width(), source_cover.height()),
        "EXTH 201 must resolve to the logical cover image"
    );

    let font_input = epub::embedded_font(false);
    let font_source = SourceModel::parse(&font_input).expect("font source model");
    let font_target = target(&font_input);
    assert!(font_target.resc_resource_hrefs().unwrap().is_empty());
    let font_bytes = font_source
        .resources
        .get("EPUB/fonts/audit.ttf")
        .expect("font bytes");
    assert!(
        font_target.font_records.iter().any(|record| {
            font_target
                .decompressed_font(*record)
                .is_ok_and(|bytes| bytes == font_bytes.1)
        }),
        "actual FONT record must remain reachable through the independent CSS/resource graph"
    );
}

fn flatten_labels(items: &[NavItem], output: &mut Vec<String>) {
    for item in items {
        output.push(item.label.clone());
        flatten_labels(&item.children, output);
    }
}

#[test]
fn large_navigation_crosses_indx_detail_records_and_reconstructs_all_rows() {
    // REQ: FMT-INDX-003.
    let input = epub::large_navigation_index();
    let source = SourceModel::parse(&input).expect("large navigation source model");
    let target = target(&input);
    let main_index = target.header.ncx_record.expect("NCX pointer") as usize;
    let main_record = target.db.record(main_index).expect("NCX main record");
    let main = crate::audit_support::palm::parse_indx(main_record).expect("NCX INDX main");
    assert!(
        main.detail_count > 1,
        "large fixture must cross multiple INDX detail records"
    );
    assert_eq!(main.detail_count, main.row_offsets.len());
    let mut decoded_rows = Vec::new();
    for detail_number in 0..main.detail_count {
        let detail_index = main_index + 1 + detail_number;
        let detail_record = target.db.record(detail_index).expect("NCX detail record");
        let mut detail =
            crate::audit_support::palm::parse_indx(detail_record).expect("NCX detail framing");
        detail.tagx = main.tagx.clone();
        let rows = crate::audit_support::palm::decode_indx_rows(detail_record, &detail)
            .expect("NCX detail rows");
        assert!(!rows.is_empty());
        assert!(detail.idxt_offset < detail_record.len());
        decoded_rows.extend(rows);
    }
    assert_eq!(decoded_rows.len(), main.entry_count);
    let mut expected_labels = Vec::new();
    flatten_labels(&source.toc, &mut expected_labels);
    let target_entries = target.ncx_entries().expect("reconstructed NCX rows");
    assert_eq!(target_entries.len(), expected_labels.len());
    assert_eq!(
        target_entries
            .iter()
            .map(|entry| entry.label.clone())
            .collect::<Vec<_>>(),
        expected_labels,
        "all detail-block rows must reconstruct the source navigation labels in order"
    );
    assert!(
        target_entries
            .iter()
            .any(|entry| entry.label.contains("Large navigation label 298-19")),
        "the final boundary-crossing navigation row must be decoded"
    );
}
