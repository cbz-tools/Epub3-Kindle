use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::{TargetProjection, decode_position_href};
use crate::audit_support::temp::TempDir;

#[test]
fn epc012_cfi_page_list_is_silently_dropped_and_normal_navigation_survives() {
    let temp = TempDir::new("epc012-cfi-page-list");
    let input = temp.write(
        "epc012-cfi-page-list.epub",
        &epub::epc012_cfi_page_list_entries(),
    );
    let output_path = temp.path().join("epc012-cfi-page-list.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-012 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "CFI page-list entries must not block conversion: {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "CFI page-list is dropped silently: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");

    let artifact = std::fs::read(&output_path).expect("read generated AZW3");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert!(target.body_text().contains("AUTH_EPC012_READABLE_CONTENT"));
    assert!(target.body_text().contains("EPC012_CHAPTER_TARGET"));
    assert!(
        !target.rawml.to_ascii_lowercase().contains("epubcfi"),
        "unsupported CFI references do not remain in active Kindle RawML"
    );

    let chapter_index = target
        .sections
        .iter()
        .position(|section| section.ids.contains("chapter-target"))
        .expect("chapter target is in a generated section");
    let chapter = &target.sections[chapter_index];
    let toc = target.ncx_entries().expect("inspect generated normal TOC");
    assert_eq!(toc.len(), 1);
    assert_eq!(toc[0].label, "Ordinary chapter");
    let toc_position = (toc[0].sequence, toc[0].offset);
    assert_eq!(toc_position.0 as usize, chapter_index);

    let ordinary_link = chapter
        .links
        .iter()
        .find(|(_, label)| label == "Ordinary fragment link")
        .map(|(href, _)| href)
        .expect("ordinary internal fragment link remains active");
    assert_eq!(
        decode_position_href(ordinary_link),
        Some(toc_position),
        "normal TOC and XHTML fragment link target the same generated position"
    );

    let page_list = target
        .sections
        .iter()
        .find(|section| section.visible_text.contains("Page List"))
        .expect("normal page-list target remains represented");
    let page_target = page_list
        .links
        .iter()
        .find(|(_, label)| label == "Ordinary page target")
        .map(|(href, _)| href)
        .expect("ordinary page-list entry survives CFI dropping");
    assert_eq!(
        decode_position_href(page_target),
        Some(toc_position),
        "ordinary page-list href still resolves to its generated fragment"
    );
    assert!(
        !page_list.visible_text.contains("CFI page one"),
        "CFI page-list labels are omitted from active Kindle content"
    );
    assert!(
        !page_list.visible_text.contains("CFI page two"),
        "all CFI-only page-list entries are silently dropped"
    );
}
