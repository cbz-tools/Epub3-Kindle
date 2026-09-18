use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::{TargetProjection, decode_position_href};
use crate::audit_support::temp::TempDir;

#[test]
fn epc001_canonical_internal_and_navigation_targets_preserve_fragments() {
    let input = epub::epc001_canonical_document_targets();
    let temp = TempDir::new("epc001-canonical-targets");
    let input_path = temp.write("epc001-canonical-targets.epub", &input);
    let output_path = temp.path().join("epc001-canonical-targets.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input_path)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-001 CLI process starts");
    assert_eq!(
        cli.status.code(),
        Some(0),
        "canonical internal links must convert successfully: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(
        cli.stderr.is_empty(),
        "EPC-001 fixture must convert without warnings: {}",
        String::from_utf8_lossy(&cli.stderr)
    );

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert_eq!(
        target.sections.len(),
        2,
        "both spine documents are generated"
    );
    assert!(target.sections[1].ids.contains("piv"));
    assert!(target.sections[1].ids.contains("later"));
    assert!(
        !target.rawml.to_ascii_lowercase().contains("xhtml/xhtml/"),
        "generated content must not retain a doubled XHTML path"
    );

    let navigation = target
        .ncx_entries()
        .expect("independently decode generated TOC targets");
    assert_eq!(
        navigation.len(),
        2,
        "both target fragments remain in navigation"
    );
    assert_eq!(navigation[0].label, "Target pivot");
    assert_eq!(navigation[1].label, "Later target");
    assert_eq!(target.sections[0].links.len(), 2);
    assert_eq!(target.sections[1].links.len(), 1);
    assert_eq!(target.sections[1].links[0].1, "Return to pivot");

    let internal_targets = target.sections[0]
        .links
        .iter()
        .map(|(href, _)| decode_position_href(href).expect("internal link uses Kindle position"))
        .collect::<Vec<_>>();
    for (internal, nav) in internal_targets.iter().zip(&navigation) {
        assert_eq!(
            internal.0 as usize, 1,
            "link resolves to the target spine section"
        );
        assert_eq!(
            *internal,
            (nav.sequence, nav.offset),
            "XHTML and navigation targets resolve to the same generated fragment"
        );
    }
    assert_ne!(
        internal_targets[0], internal_targets[1],
        "#piv and #later retain distinct fragment destinations"
    );
    assert!(
        internal_targets[0].1 < internal_targets[1].1,
        "fragment positions retain document order"
    );
    let fragment_only_target = decode_position_href(&target.sections[1].links[0].0)
        .expect("fragment-only link uses a Kindle position");
    assert_eq!(
        fragment_only_target, internal_targets[0],
        "#piv retains its fragment and resolves within the current document"
    );
}
