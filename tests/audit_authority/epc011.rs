use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::{TargetProjection, decode_position_href};
use crate::audit_support::temp::TempDir;

#[test]
fn epc011_valid_body_id_toc_fragments_resolve_to_generated_positions() {
    let temp = TempDir::new("epc011-body-fragment-toc");
    let input = temp.write(
        "epc011-body-fragment-toc.epub",
        &epub::epc011_toc_body_fragment_targets(),
    );
    let output_path = temp.path().join("epc011-body-fragment-toc.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-011 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "valid TOC fragment targets on body ids must convert: {stderr}"
    );
    assert!(stderr.is_empty(), "EPC-011 has no warnings: {stderr}");
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");

    let artifact = std::fs::read(&output_path).expect("read generated AZW3");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    let chapter_one_index = target
        .sections
        .iter()
        .position(|section| {
            section
                .visible_text
                .contains("AUTH_EPC011_BODY_FRAGMENT_CONTENT")
        })
        .expect("first chapter is a generated section");
    let chapter_two_index = target
        .sections
        .iter()
        .position(|section| {
            section
                .visible_text
                .contains("AUTH_EPC011_SECOND_CHAPTER_CONTENT")
        })
        .expect("second chapter is a generated section");
    let chapter_one = &target.sections[chapter_one_index];
    let chapter_two = &target.sections[chapter_two_index];
    assert!(chapter_one.ids.contains("body-target"));
    assert!(chapter_one.ids.contains("section-target"));
    assert!(chapter_two.ids.contains("chapter-two-target"));
    assert!(
        target
            .body_text()
            .contains("AUTH_EPC011_SECTION_FRAGMENT_CONTENT")
    );
    assert!(
        target
            .body_text()
            .contains("AUTH_EPC011_SECOND_CHAPTER_CONTENT")
    );

    let nav = target
        .ncx_entries()
        .expect("decode generated TOC destinations");
    assert_eq!(nav.len(), 3);
    assert_eq!(nav[0].label, "Body fragment");
    assert_eq!(nav[1].label, "Section fragment");
    assert_eq!(nav[2].label, "Second chapter");

    let toc_positions = nav
        .iter()
        .map(|entry| (entry.sequence, entry.offset))
        .collect::<Vec<_>>();
    assert_eq!(toc_positions[0].0 as usize, chapter_one_index);
    assert_eq!(toc_positions[1].0 as usize, chapter_one_index);
    assert_eq!(toc_positions[2].0 as usize, chapter_two_index);

    let link_position = |label: &str, links: &[(String, String)]| {
        let href = &links
            .iter()
            .find(|(_, link_label)| link_label == label)
            .unwrap_or_else(|| panic!("generated link {label:?} exists"))
            .0;
        decode_position_href(href).expect("internal link is rewritten to a Kindle position")
    };
    assert_eq!(
        Some(toc_positions[0]),
        Some(link_position("Body target link", &chapter_one.links)),
        "body-id TOC target agrees with an ordinary internal link to the same fragment"
    );
    assert_eq!(
        Some(toc_positions[1]),
        Some(link_position("Section target link", &chapter_one.links)),
        "paragraph fragment remains a distinct, valid destination"
    );
    assert_eq!(
        Some(toc_positions[2]),
        Some(link_position("Ordinary chapter link", &chapter_one.links)),
        "ordinary cross-chapter navigation still targets its generated section"
    );
    assert_ne!(toc_positions[0], toc_positions[1]);
    assert!(chapter_two.links.iter().any(|(href, label)| {
        label == "Return to first chapter" && decode_position_href(href) == Some(toc_positions[1])
    }));
    assert!(
        !target.rawml.contains("chapter-one.xhtml#body-target")
            && !target.rawml.contains("#body-target"),
        "source href/fragment spelling does not leak into active links"
    );
}
