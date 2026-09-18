use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::{TargetProjection, decode_embed_number, decode_position_href};
use crate::audit_support::temp::TempDir;

#[test]
fn epc013_suppressed_cover_navigation_uses_native_cover_destination() {
    let temp = TempDir::new("epc013-suppressed-cover-navigation");
    let input = temp.write(
        "epc013-suppressed-cover-navigation.epub",
        &epub::epc013_suppressed_cover_navigation(),
    );
    let output_path = temp.path().join("epc013-suppressed-cover-navigation.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-013 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "suppressed cover navigation must convert successfully: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");
    assert!(
        stderr.is_empty(),
        "fixture converts without warnings: {stderr}"
    );

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert_eq!(
        target.sections.len(),
        3,
        "nav, page list, and chapter are generated"
    );
    assert!(
        !target
            .body_text()
            .contains("AUTH_EPC013_SUPPRESSED_COVER_XHTML_BODY"),
        "suppressed cover XHTML body is absent"
    );
    assert!(target.body_text().contains("AUTH_EPC013_CHAPTER_BODY"));
    assert!(
        !target.rawml.contains("text/cover-page.xhtml") && !target.rawml.contains("#cover-target"),
        "suppressed cover path and source fragment are absent from active RawML"
    );

    let cover_offset = target
        .header
        .exth_u32(201)
        .expect("EXTH 201 identifies the native cover resource") as usize;
    let cover_record_index = target.header.first_resource as usize + cover_offset;
    assert!(
        target.image_records().contains(&cover_record_index),
        "EXTH 201 resolves to the emitted cover image record"
    );
    let cover_bytes = target
        .db
        .record(cover_record_index)
        .expect("EXTH 201 cover image record");
    assert_eq!(
        image::guess_format(cover_bytes).expect("cover resource has an image format"),
        image::ImageFormat::Jpeg,
        "PNG cover is emitted as the existing native JPEG cover resource"
    );
    image::load_from_memory(cover_bytes).expect("native cover image decodes");

    let page_list = target
        .sections
        .iter()
        .find(|section| section.visible_text.contains("Page List"))
        .expect("synthetic page-list section is generated");
    let nav_section = target
        .sections
        .iter()
        .find(|section| {
            section
                .links
                .iter()
                .any(|(_, label)| label == "Cover destination")
        })
        .expect("source navigation document remains in its reading position");
    let chapter_index = target
        .sections
        .iter()
        .position(|section| section.visible_text.contains("AUTH_EPC013_CHAPTER_BODY"))
        .expect("ordinary chapter section is generated");
    let chapter = &target.sections[chapter_index];
    let page_list_cover = page_list
        .links
        .iter()
        .find(|(_, label)| label == "Cover page")
        .map(|(href, _)| href)
        .expect("page-list cover link is emitted");
    let nav_cover_landmark = nav_section
        .links
        .iter()
        .find(|(_, label)| label == "Cover destination")
        .map(|(href, _)| href)
        .expect("nav landmarks cover link is emitted");
    assert_eq!(
        page_list_cover, nav_cover_landmark,
        "page-list and navigation landmark resolve to the same native resource reference"
    );
    assert!(
        page_list_cover.starts_with("kindle:embed:"),
        "cover href resolves through the native Kindle resource reference"
    );
    let cover_embed = page_list_cover
        .strip_prefix("kindle:embed:")
        .and_then(|href| href.split('?').next())
        .and_then(decode_embed_number)
        .expect("cover href carries a native resource number");
    assert_eq!(
        target
            .resource_bytes(cover_embed)
            .expect("cover embed resolves"),
        cover_bytes,
        "page-list and landmark hrefs resolve to the resource selected by EXTH 201"
    );

    let ordinary_page_target = page_list
        .links
        .iter()
        .find(|(_, label)| label == "Ordinary page target")
        .map(|(href, _)| href)
        .expect("ordinary page-list link is emitted");
    let ordinary_page_position = decode_position_href(ordinary_page_target)
        .expect("ordinary fragment remains a Kindle position link");
    assert_eq!(
        ordinary_page_position.0 as usize, chapter_index,
        "ordinary target resolves to the generated chapter section"
    );
    assert!(chapter.ids.contains("ordinary"));
    let ordinary_body_link = chapter
        .links
        .iter()
        .find(|(_, label)| label == "Ordinary fragment link")
        .map(|(href, _)| href)
        .expect("ordinary body fragment link is emitted");
    assert_eq!(
        decode_position_href(ordinary_body_link),
        Some(ordinary_page_position),
        "ordinary internal fragment link still resolves to the same generated target"
    );

    let toc = target
        .ncx_entries()
        .expect("inspect generated TOC after existing cover pruning");
    assert_eq!(toc.len(), 1, "cover TOC entry remains pruned");
    assert_eq!(toc[0].label, "Ordinary chapter");
}

#[test]
fn epc013_without_native_cover_resource_keeps_source_cover_destination() {
    let temp = TempDir::new("epc013-source-cover-without-native-resource");
    let input = temp.write(
        "epc013-source-cover-without-native-resource.epub",
        &epub::epc013_cover_navigation_without_native_cover_resource(),
    );
    let output_path = temp
        .path()
        .join("epc013-source-cover-without-native-resource.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-013 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "source cover remains a generated document destination when no native cover exists: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "fixture converts without warnings: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");

    let artifact = std::fs::read(&output_path).expect("read generated AZW3");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert!(target.body_text().contains("AUTH_EPC013_SOURCE_COVER"));
    assert!(target.body_text().contains("AUTH_EPC013_ORDINARY_CHAPTER"));
    assert!(
        target.header.exth_u32(201).is_none() && target.header.exth_u32(202).is_none(),
        "no native cover or thumbnail offsets are fabricated"
    );
    assert!(
        target.image_records().is_empty(),
        "the fixture has no native cover image resource"
    );

    let cover_index = target
        .sections
        .iter()
        .position(|section| section.visible_text.contains("AUTH_EPC013_SOURCE_COVER"))
        .expect("the source cover remains a generated reading section");
    assert!(target.sections[cover_index].ids.contains("cover-target"));
    let nav_section = target
        .sections
        .iter()
        .find(|section| {
            section
                .links
                .iter()
                .any(|(_, label)| label == "Cover destination")
        })
        .expect("the source landmarks navigation remains readable");
    let landmark_position = nav_section
        .links
        .iter()
        .find(|(_, label)| label == "Cover destination")
        .and_then(|(href, _)| decode_position_href(href))
        .expect("cover landmark resolves to a generated position");
    assert_eq!(
        landmark_position.0 as usize, cover_index,
        "cover landmark targets the retained source cover section"
    );

    let page_list = target
        .sections
        .iter()
        .find(|section| section.visible_text.contains("Page List"))
        .expect("the page-list projection remains present");
    let page_list_cover_position = page_list
        .links
        .iter()
        .find(|(_, label)| label == "Cover page")
        .and_then(|(href, _)| decode_position_href(href))
        .expect("page-list cover target resolves to a generated position");
    assert_eq!(page_list_cover_position, landmark_position);

    let toc = target
        .ncx_entries()
        .expect("inspect normal TOC destinations");
    let cover_toc = toc
        .iter()
        .find(|entry| entry.label == "Cover contents entry")
        .expect("cover TOC entry is retained");
    assert_eq!(
        (cover_toc.sequence, cover_toc.offset),
        landmark_position,
        "normal TOC and cover landmark share the source cover destination"
    );
    assert!(toc.iter().any(|entry| entry.label == "Ordinary chapter"));
    assert!(
        !target.rawml.contains("text/cover-page.xhtml#cover-target")
            && !target.rawml.contains("kindle:cover-landmark"),
        "the source href is rewritten to a valid position and no native-cover marker remains"
    );
}
