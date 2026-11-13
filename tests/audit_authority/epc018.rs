use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::palm::{
    PalmDb, decode_indx_rows, parse_indx, parse_trailer, text_record_indices,
};
use crate::audit_support::semantic::{
    NavItem, SourceModel, TargetProjection, decode_position_href,
};
use crate::audit_support::temp::TempDir;

fn opening_tags(source: &str) -> Vec<&str> {
    source
        .split('<')
        .skip(1)
        .filter_map(|part| part.find('>').map(|end| &part[..=end]))
        .collect()
}

fn tag_name(tag: &str) -> &str {
    tag.trim_start_matches('/')
        .split(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
        .next()
        .unwrap_or_default()
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn tags_named<'a>(source: &'a str, name: &str) -> Vec<&'a str> {
    opening_tags(source)
        .into_iter()
        .filter(|tag| tag_name(tag).eq_ignore_ascii_case(name))
        .collect()
}

fn external_reference(href: &str) -> bool {
    let lower = href.trim_start().to_ascii_lowercase();
    ["http:", "https:", "mailto:", "tel:", "//", "data:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

fn flatten_nav(items: &[NavItem], labels: &mut Vec<String>) {
    for item in items {
        labels.push(item.label.clone());
        flatten_nav(&item.children, labels);
    }
}

fn read_entry(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> String {
    let mut content = String::new();
    archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("EPUB entry {name} exists"))
        .read_to_string(&mut content)
        .unwrap_or_else(|_| panic!("EPUB entry {name} is readable"));
    content
}

fn fragment_sequence_ranges(
    target: &TargetProjection<'_>,
) -> std::collections::BTreeMap<u32, (u32, u32)> {
    let coordinate = if target.header.version >= 8 && target.header.record_index > 0 {
        "section-relative"
    } else {
        "global"
    };
    let main_index = target
        .header
        .global_record_index(
            target.header.index_record.expect("KF8 FRAG INDX pointer"),
            coordinate,
        )
        .expect("FRAG pointer resolves to a PalmDB record");
    let main = parse_indx(target.db.record(main_index).expect("FRAG main record"))
        .expect("FRAG main INDX framing");
    let mut ranges = std::collections::BTreeMap::new();
    for detail_number in 0..main.detail_count {
        let detail_record = target
            .db
            .record(main_index + 1 + detail_number)
            .expect("FRAG detail record");
        let mut detail = parse_indx(detail_record).expect("FRAG detail INDX framing");
        detail.tagx = main.tagx.clone();
        for row in decode_indx_rows(detail_record, &detail).expect("FRAG detail rows") {
            let file_number = *row
                .values
                .get(&3)
                .and_then(|values| values.first())
                .expect("FRAG row has owning document number");
            let sequence = *row
                .values
                .get(&4)
                .and_then(|values| values.first())
                .expect("FRAG row has navigation sequence");
            let length = *row
                .values
                .get(&6)
                .and_then(|values| values.get(1))
                .expect("FRAG tag 6 has payload length");
            assert!(ranges.insert(sequence, (file_number, length)).is_none());
        }
    }
    assert_eq!(ranges.len(), main.entry_count);
    ranges
}

fn project_kf8(artifact: &[u8], dual_mobi: bool) -> TargetProjection<'_> {
    if dual_mobi {
        let palm = PalmDb::parse(artifact).expect("independent Dual MOBI PalmDB");
        let kf8_index =
            palm.mobi_header(0)
                .expect("Dual MOBI KF7 header")
                .exth_u32(121)
                .expect("Dual MOBI EXTH 121 identifies the KF8 section") as usize;
        TargetProjection::parse_at(artifact, kf8_index)
            .expect("independent Dual MOBI KF8 projection")
    } else {
        TargetProjection::parse(artifact).expect("independent KF8 projection")
    }
}

fn run_external_epub(input: &PathBuf, format: &str, extension: &str) {
    let source_bytes = std::fs::read(input).expect("external EPUB is readable");
    let source = SourceModel::parse(&source_bytes).expect("independent source EPUB projection");
    assert!(
        source
            .sections
            .iter()
            .any(|section| !section.visible_text.trim().is_empty()),
        "source EPUB has readable spine content"
    );
    let release_output_env = if format == "Dual MOBI" {
        "EPUB3_KINDLE_RELEASE_DUAL_OUTPUT"
    } else {
        "EPUB3_KINDLE_RELEASE_KF8_OUTPUT"
    };
    if let Ok(release_path) = std::env::var(release_output_env) {
        let release_artifact =
            std::fs::read(&release_path).expect("release CLI output is readable");
        verify_external_artifact(&source_bytes, &source, &release_artifact, format);
    }

    let temp = TempDir::new(&format!("epc018-external-{format}"));
    let output = temp.path().join(format!("external.{extension}"));
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(input)
        .arg("-o")
        .arg(&output)
        .arg("-verbose")
        .output()
        .expect("external EPUB CLI process starts");
    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert!(
        matches!(cli.status.code(), Some(0 | 1)),
        "external EPUB does not fail conversion: {stderr}"
    );
    assert!(output.is_file(), "the CLI writes {format} output");

    let artifact = std::fs::read(&output).expect("CLI writes Kindle output");
    verify_external_artifact(&source_bytes, &source, &artifact, format);
}

fn verify_external_artifact(
    source_bytes: &[u8],
    source: &SourceModel,
    artifact: &[u8],
    format: &str,
) {
    let palm = PalmDb::parse(artifact).expect("independent PalmDB validation");
    assert!(palm.record_count() > 1, "PalmDB contains records");
    let first_header = palm
        .mobi_header(0)
        .expect("PalmDB begins with MOBI Record 0");
    for record_index in text_record_indices(&palm, &first_header).expect("MOBI text record range") {
        let record = palm.record(record_index).expect("MOBI text record exists");
        parse_trailer(record, first_header.extra_record_data_flags).unwrap_or_else(|error| {
            panic!(
                "independent text trailer parse failed at record {record_index}, flags {:#06x}, record bytes {}: {error}",
                first_header.extra_record_data_flags,
                record.len()
            )
        });
    }
    let target = if format == "Dual MOBI" {
        let kf8_index = first_header
            .exth_u32(121)
            .expect("Dual MOBI EXTH 121 identifies the KF8 section")
            as usize;
        TargetProjection::parse_at(artifact, kf8_index)
    } else {
        TargetProjection::parse(artifact)
    }
    .expect("independent KF8 output projection");
    let fragment_ranges = fragment_sequence_ranges(&target);
    assert!(
        target.header.version >= 8,
        "KF8 reading rendition is present"
    );

    if format == "Dual MOBI" {
        assert!(
            first_header.version < 8,
            "Dual MOBI starts with KF7 compatibility data"
        );
        let kf8_index = first_header
            .exth_u32(121)
            .expect("Dual MOBI EXTH 121 identifies the KF8 section")
            as usize;
        assert!(kf8_index > 0, "KF8 section follows the KF7 section");
        assert_eq!(palm.record(kf8_index - 1).unwrap(), b"BOUNDARY");
        assert_eq!(
            palm.mobi_header(kf8_index).unwrap().version,
            target.header.version,
            "independent MOBI section parsing identifies the KF8 rendition"
        );
        assert_eq!(target.header.record_index, kf8_index);
    } else {
        assert_eq!(first_header.version, target.header.version);
    }

    let text_sections = source
        .sections
        .iter()
        .filter(|section| !section.visible_text.trim().is_empty())
        .collect::<Vec<_>>();
    assert_eq!(target.sections.len(), text_sections.len());
    let mut source_docs = zip::ZipArchive::new(Cursor::new(source_bytes))
        .expect("source EPUB ZIP opens for link verification");
    for (source_section, target_section) in text_sections.iter().zip(&target.sections) {
        let source_section = *source_section;
        let source_xhtml = read_entry(&mut source_docs, &source_section.href);
        let source_anchor_tags = tags_named(&source_xhtml, "a")
            .into_iter()
            .filter(|tag| !tag.starts_with('/'))
            .filter(|tag| attribute(tag, "href").is_some())
            .collect::<Vec<_>>();
        assert_eq!(
            target_section.visible_text, source_section.visible_text,
            "complete readable text survives in {}",
            source_section.href
        );
        assert!(
            source_section
                .ids
                .iter()
                .all(|id| target_section.ids.contains(id)),
            "all source fragment IDs survive in {}",
            source_section.href
        );
        assert_eq!(target_section.links.len(), source_section.links.len());
        assert_eq!(source_anchor_tags.len(), source_section.links.len());
        for (link_index, ((href, label), (target_href, target_label))) in source_section
            .links
            .iter()
            .zip(&target_section.links)
            .enumerate()
        {
            let raw_href = attribute(source_anchor_tags[link_index], "href")
                .expect("source anchor has its hyperlink target");
            assert_eq!(target_label, label, "link labels survive");
            if !external_reference(&raw_href) {
                let expected_file_number = text_sections
                    .iter()
                    .position(|section| section.href == *href)
                    .unwrap_or_else(|| {
                        panic!(
                            "internal source target {href:?} from {} names a non-readable spine document",
                            source_section.href
                        )
                    });
                let (sequence, offset) = decode_position_href(target_href)
                    .expect("internal link is represented by a Kindle position");
                let (actual_file_number, fragment_length) = fragment_ranges
                    .get(&sequence)
                    .copied()
                    .expect("internal link sequence exists in the independent FRAG index");
                assert_eq!(
                    actual_file_number as usize, expected_file_number,
                    "internal link points to the source document named by its href"
                );
                assert!(
                    offset < fragment_length,
                    "internal link offset is inside its target FRAG payload"
                );
                assert!(
                    offset < target.header.text_length as u32,
                    "internal link offset is within KF8 text"
                );
                if let Some((_, fragment)) = raw_href.split_once('#') {
                    assert!(
                        fragment.is_empty()
                            || target.sections[expected_file_number].ids.contains(fragment),
                        "internal fragment {fragment:?} exists in the expected output document"
                    );
                }
            }
        }
    }

    let mut expected_toc = Vec::new();
    flatten_nav(&source.toc, &mut expected_toc);
    let actual_toc = target
        .ncx_entries()
        .expect("independent NCX reconstruction")
        .into_iter()
        .map(|entry| entry.label)
        .collect::<Vec<_>>();
    assert_eq!(
        actual_toc, expected_toc,
        "Kindle TOC preserves source labels and order"
    );

    let mut archive = zip::ZipArchive::new(Cursor::new(source_bytes))
        .expect("source EPUB ZIP opens independently");
    let container = read_entry(&mut archive, "META-INF/container.xml");
    let rootfile = tags_named(&container, "rootfile")
        .into_iter()
        .find_map(|tag| attribute(tag, "full-path"))
        .expect("OCF container names a Package Document");
    let opf = read_entry(&mut archive, &rootfile);
    let cover_item = tags_named(&opf, "item").into_iter().find(|tag| {
        attribute(tag, "properties")
            .is_some_and(|value| value.split_whitespace().any(|token| token == "cover-image"))
    });
    if let Some(cover_item) = cover_item {
        let cover_href = attribute(cover_item, "href").expect("logical cover has an href");
        let cover_path = resolve_package_resource(&rootfile, &cover_href);
        let cover_source = source
            .resources
            .get(&cover_path)
            .expect("logical cover is a packaged manifest resource");
        let source_cover = image::load_from_memory(&cover_source.1)
            .expect("source logical cover is a decodable image");
        let cover_offset = target
            .header
            .exth_u32(201)
            .expect("KF8 EXTH 201 identifies the logical cover")
            as usize;
        let cover_bytes = target
            .embedded_bytes(cover_offset + 1)
            .expect("EXTH cover offset resolves inside the image resource pool");
        let cover_record_index = target
            .image_records()
            .into_iter()
            .find(|&record_index| {
                target
                    .db
                    .record(record_index)
                    .is_ok_and(|record| record == cover_bytes)
            })
            .expect("EXTH cover image is present in the PalmDB image records");
        let target_cover = image::load_from_memory(cover_bytes)
            .expect("converted logical cover is a decodable image");
        assert_eq!(
            (target_cover.width(), target_cover.height()),
            (source_cover.width(), source_cover.height()),
            "logical cover dimensions survive conversion"
        );
        assert!(target.image_records().contains(&cover_record_index));
    }
}

fn resolve_package_resource(package_path: &str, href: &str) -> String {
    let path = href.split(['#', '?']).next().unwrap_or(href);
    let base = package_path
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let mut parts = base
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

#[test]
fn epc018_link_rel_resource_variants_and_valid_a_area_links_convert_in_both_formats() {
    let input_bytes = epub::epc018_link_target_case("both", "target.xhtml#destination");
    let source = SourceModel::parse(&input_bytes).expect("independent source fixture projection");
    assert_eq!(source.sections[0].links.len(), 1);

    for (format, extension) in [("KF8", "azw3"), ("Dual MOBI", "mobi")] {
        let temp = TempDir::new(&format!("epc018-links-{format}"));
        let input = temp.write("epc018-links.epub", &input_bytes);
        let output = temp.path().join(format!("epc018-links.{extension}"));
        let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
            .arg(input)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("EPC-018 CLI process starts");
        let stderr = String::from_utf8_lossy(&cli.stderr);
        assert_eq!(
            cli.status.code(),
            Some(0),
            "{format} conversion is clean: {stderr}"
        );
        assert!(stderr.is_empty(), "{format} emits no diagnostics: {stderr}");
        let artifact = std::fs::read(&output).expect("CLI writes the Kindle artifact");
        let palm = PalmDb::parse(&artifact).expect("independent PalmDB structure is valid");
        let first_header = palm.mobi_header(0).unwrap();
        let target = if format == "Dual MOBI" {
            let kf8_index = first_header
                .exth_u32(121)
                .expect("Dual MOBI EXTH 121 identifies the KF8 section")
                as usize;
            TargetProjection::parse_at(&artifact, kf8_index)
        } else {
            TargetProjection::parse(&artifact)
        }
        .expect("independent KF8 content parses");
        assert!(
            target.body_text().contains("AUTH_EPC018_DESTINATION"),
            "source destination text survives in {format}: {}",
            target.body_text()
        );
        assert_eq!(target.sections.len(), 2);

        let anchors = tags_named(&target.rawml, "a")
            .into_iter()
            .filter_map(|tag| attribute(tag, "href"))
            .collect::<Vec<_>>();
        let areas = tags_named(&target.rawml, "area")
            .into_iter()
            .filter_map(|tag| attribute(tag, "href"))
            .collect::<Vec<_>>();
        assert_eq!(anchors.len(), 1, "one source anchor survives");
        assert_eq!(areas.len(), 1, "one source area survives");
        assert!(anchors[0].starts_with("kindle:pos:fid:"));
        assert!(areas[0].starts_with("kindle:pos:fid:"));
        assert_eq!(anchors[0], areas[0], "a and area point to the same target");
        assert!(target.sections[1].ids.contains("destination"));
        let (sequence, offset) = decode_position_href(&anchors[0])
            .expect("valid local a/area links use a Kindle position");
        let (file_number, fragment_length) = fragment_sequence_ranges(&target)
            .get(&sequence)
            .copied()
            .expect("valid local link points into the independent FRAG index");
        assert_eq!(file_number, 1, "a/area target the second spine document");
        assert!(
            offset < fragment_length,
            "target position is inside its FRAG payload"
        );

        let links = tags_named(&target.rawml, "link");
        let icon_hrefs = links
            .iter()
            .filter(|tag| {
                attribute(tag, "rel").is_some_and(|rel| {
                    rel.split_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("icon"))
                })
            })
            .filter_map(|tag| attribute(tag, "href"))
            .collect::<Vec<_>>();
        assert_eq!(
            icon_hrefs.len(),
            2,
            "both icon rel patterns remain resource links"
        );
        assert!(icon_hrefs.iter().any(|href| href == "../images/icon.png"));
        assert!(
            icon_hrefs
                .iter()
                .any(|href| href == "../images/alternate-icon.png")
        );
        assert!(
            icon_hrefs
                .iter()
                .all(|href| !href.starts_with("kindle:pos:"))
        );

        let stylesheet_hrefs = links
            .iter()
            .filter(|tag| {
                attribute(tag, "rel").is_some_and(|rel| {
                    rel.split_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                })
            })
            .filter_map(|tag| attribute(tag, "href"))
            .collect::<Vec<_>>();
        assert_eq!(
            stylesheet_hrefs.len(),
            2,
            "single and multi-token CSS rel forms survive"
        );
        assert!(
            stylesheet_hrefs
                .iter()
                .all(|href| href.starts_with("kindle:flow:"))
        );
        assert!(target.css.contains("link-normal-style-marker"));
        assert!(target.css.contains("link-alternate-style-marker"));
    }
}

#[test]
fn epc018_invalid_a_area_document_and_fragment_targets_remain_fatal_in_both_formats() {
    let cases = [
        (
            "a-off-spine-document",
            "a",
            "../text/off-spine.xhtml#off-spine",
            "internal link target does not resolve to a generated document",
        ),
        (
            "area-off-spine-document",
            "area",
            "../text/off-spine.xhtml#off-spine",
            "internal link target does not resolve to a generated document",
        ),
        (
            "a-missing-fragment",
            "a",
            "target.xhtml#missing-fragment",
            "internal link fragment does not resolve in generated document",
        ),
        (
            "area-missing-fragment",
            "area",
            "target.xhtml#missing-fragment",
            "internal link fragment does not resolve in generated document",
        ),
    ];

    for (case, element, href, expected_error) in cases {
        for (format, extension) in [("KF8", "azw3"), ("Dual MOBI", "mobi")] {
            let temp = TempDir::new(&format!("epc018-{case}-{format}"));
            let input = temp.write(
                "epc018-invalid-link.epub",
                &epub::epc018_link_target_case(element, href),
            );
            let output = temp.path().join(format!("invalid.{extension}"));
            let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
                .arg(input)
                .arg("-o")
                .arg(&output)
                .output()
                .expect("EPC-018 invalid-link CLI process starts");
            let stderr = String::from_utf8_lossy(&cli.stderr);
            assert_eq!(
                cli.status.code(),
                Some(2),
                "{case} remains fatal for {format}: {stderr}"
            );
            assert!(
                stderr.contains(expected_error),
                "strict target validation remains: {stderr}"
            );
            assert!(
                !output.exists(),
                "invalid {element} destination does not leave partial {format} output"
            );
        }
    }
}

#[test]
fn epc018_pls_links_remain_on_the_dedicated_degradation_path_in_both_formats() {
    for (format, extension, dual_mobi) in [("KF8", "azw3", false), ("Dual MOBI", "mobi", true)] {
        let temp = TempDir::new(&format!("epc018-pls-{format}"));
        let input = temp.write(
            "epc004-pronunciation.epub",
            &epub::epc004_pronunciation_lexicon(),
        );
        let output = temp.path().join(format!("epc018-pls.{extension}"));
        let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
            .arg(input)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("EPC-018 PLS CLI process starts");
        let stderr = String::from_utf8_lossy(&cli.stderr);
        assert_eq!(
            cli.status.code(),
            Some(1),
            "{format} PLS warning-success: {stderr}"
        );
        assert_eq!(
            stderr.matches("warning[W002]:").count(),
            1,
            "{format}: {stderr}"
        );
        assert!(
            stderr.contains("PLS pronunciation lexicon semantics were dropped for Kindle output")
        );
        let artifact = std::fs::read(&output).expect("PLS conversion writes output");
        let target = project_kf8(&artifact, dual_mobi);
        assert!(
            target
                .body_text()
                .contains("AUTH_EPC004_PRONUNCIATION_BODY")
        );
        assert!(target.rawml.contains(
            "<link rel=\"pronunciation\" type=\"application/pls+xml\" href=\"../lexicon/en.pls\"/>"
        ));
        assert!(target.rawml.contains(
            "<link rel=\"alternate pronunciation\" type=\"application/pls+xml\" href=\"../lexicon/de.pls\"/>"
        ));
        assert!(target.rawml.contains("href=\"kindle:pos:fid:"));
        assert!(
            target
                .resc_resource_hrefs()
                .expect("inspect PLS RESC references")
                .iter()
                .all(|href| !href.ends_with(".pls"))
        );
        assert!(
            !artifact
                .windows(b"AUTH_EPC004_PLS_PAYLOAD".len())
                .any(|window| window == b"AUTH_EPC004_PLS_PAYLOAD")
        );
    }
}

#[test]
fn epc018_xpgt_links_remain_on_the_dedicated_degradation_path_in_both_formats() {
    for (format, extension, dual_mobi) in [("KF8", "azw3", false), ("Dual MOBI", "mobi", true)] {
        let temp = TempDir::new(&format!("epc018-xpgt-{format}"));
        let input = temp.write("epc005-xpgt.epub", &epub::epc005_xpgt_page_template());
        let output = temp.path().join(format!("epc018-xpgt.{extension}"));
        let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
            .arg(input)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("EPC-018 XPGT CLI process starts");
        let stderr = String::from_utf8_lossy(&cli.stderr);
        assert_eq!(
            cli.status.code(),
            Some(1),
            "{format} XPGT warning-success: {stderr}"
        );
        assert_eq!(
            stderr.matches("warning[W004]:").count(),
            1,
            "{format}: {stderr}"
        );
        assert!(
            stderr.contains("Adobe XPGT page-template semantics were dropped for Kindle output")
        );
        let artifact = std::fs::read(&output).expect("XPGT conversion writes output");
        let target = project_kf8(&artifact, dual_mobi);
        assert!(target.body_text().contains("AUTH_EPC005_XPGT_BODY"));
        for href in [
            "../styles/page-template.xpgt",
            "../styles/alternate-template.xpgt",
        ] {
            assert!(target.rawml.contains(href), "{format} retains {href}");
        }
        assert!(!target.css.contains("AUTH_EPC005_XPGT_PAYLOAD"));
        let compact_css = target
            .css
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        assert!(compact_css.contains(".ordinary{color:red;}"));
        assert!(
            target
                .resc_resource_hrefs()
                .expect("inspect XPGT RESC references")
                .iter()
                .all(|href| !href.to_ascii_lowercase().ends_with(".xpgt"))
        );
        assert!(
            !artifact
                .windows(b"AUTH_EPC005_XPGT_PAYLOAD_ONE".len())
                .any(|window| window == b"AUTH_EPC005_XPGT_PAYLOAD_ONE")
        );
    }
}

#[test]
#[ignore = "optional external-EPUB check; set EPUB3_KINDLE_EXTERNAL_EPUB to a local file"]
fn epc018_optional_external_epub_converts_in_both_formats() {
    let input = PathBuf::from(
        std::env::var_os("EPUB3_KINDLE_EXTERNAL_EPUB")
            .expect("set EPUB3_KINDLE_EXTERNAL_EPUB to a local external EPUB path"),
    );
    assert!(
        input.is_file(),
        "the explicitly selected external EPUB exists"
    );
    run_external_epub(&input, "KF8", "azw3");
    run_external_epub(&input, "Dual MOBI", "mobi");
}
