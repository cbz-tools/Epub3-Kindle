use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

#[test]
fn epc005_xpgt_page_template_is_dropped_with_warning_and_body_preserved() {
    let temp = TempDir::new("epc005-xpgt-page-template");
    let input = temp.write("epc005-xpgt.epub", &epub::epc005_xpgt_page_template());
    let output_path = temp.path().join("epc005-xpgt.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-005 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(1),
        "XPGT degradation completes as warning-success: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");
    let warning_lines = stderr
        .lines()
        .filter(|line| line.contains("warning["))
        .collect::<Vec<_>>();
    assert_eq!(
        warning_lines.len(),
        1,
        "multiple XPGT resources and references produce one warning only: {stderr}"
    );
    assert!(
        warning_lines[0].contains("warning[W004]:"),
        "XPGT degradation emits W004: {stderr}"
    );
    assert!(stderr.contains("Adobe XPGT page-template semantics were dropped for Kindle output"));

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert!(
        target.body_text().contains("AUTH_EPC005_XPGT_BODY"),
        "readable chapter content survives"
    );
    for href in [
        "../styles/page-template.xpgt",
        "../styles/alternate-template.xpgt",
    ] {
        assert!(
            target.rawml.contains(href),
            "original XPGT link reference {href} remains in RawML"
        );
    }
    assert!(
        target
            .rawml
            .contains("type=\"application/adobe-page-template+xml\""),
        "the original XPGT link type remains in RawML"
    );
    assert!(
        !target.css.contains("AUTH_EPC005_XPGT_PAYLOAD")
            && !target.css.contains(".xpgt-one")
            && !target.css.contains(".xpgt-two"),
        "XPGT payload is not translated into generated CSS"
    );
    let compact_css = target
        .css
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    assert!(
        compact_css.contains(".ordinary{color:red;}"),
        "ordinary text/css remains in the generated CSS flow: {}",
        target.css
    );

    let resc_hrefs = target
        .resc_resource_hrefs()
        .expect("inspect output RESC resource references");
    assert!(
        resc_hrefs
            .iter()
            .all(|href| !href.to_ascii_lowercase().ends_with(".xpgt")),
        "RESC has no XPGT resource mapping: {resc_hrefs:?}"
    );
    assert_eq!(
        target.embedded_resource_numbers.len(),
        1,
        "the ordinary image is emitted as the only embedded resource"
    );
    let image = target
        .embedded_bytes(target.embedded_resource_numbers[0])
        .expect("resolve ordinary image resource mapping");
    let decoded_image = image::load_from_memory(image).expect("decode ordinary image payload");
    assert_eq!((decoded_image.width(), decoded_image.height()), (1, 1));

    for payload_marker in [
        b"AUTH_EPC005_XPGT_PAYLOAD_ONE".as_slice(),
        b"AUTH_EPC005_XPGT_PAYLOAD_TWO".as_slice(),
    ] {
        assert!(
            !artifact
                .windows(payload_marker.len())
                .any(|window| window == payload_marker),
            "XPGT payload marker {} is absent from the artifact",
            String::from_utf8_lossy(payload_marker)
        );
    }
}
