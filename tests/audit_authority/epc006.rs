use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

#[test]
fn epc006_mathml_reduces_to_ordered_readable_descendant_text() {
    let temp = TempDir::new("epc006-mathml");
    let input = temp.write(
        "epc006-mathml.epub",
        &epub::epc006_mathml_descendant_content(),
    );
    let output_path = temp.path().join("epc006-mathml.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-006 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(1),
        "MathML reduction is a warning-success conversion: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");
    let warning_lines = stderr
        .lines()
        .filter(|line| line.contains("warning["))
        .collect::<Vec<_>>();
    assert_eq!(
        warning_lines.len(),
        1,
        "two MathML roots produce one warning and no other warnings: {stderr}"
    );
    assert!(
        warning_lines[0].contains("warning[W005]:"),
        "MathML emits W005 exactly once: {stderr}"
    );
    assert_eq!(
        stderr.matches("warning[W005]:").count(),
        1,
        "exactly one W005 warning is emitted: {stderr}"
    );

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    let rawml_lower = target.rawml.to_ascii_lowercase();
    assert!(
        !rawml_lower.contains("http://www.w3.org/1998/math/mathml"),
        "MathML namespace is absent from generated RawML"
    );
    for tag in ["<math", "</math", "<mrow", "<mi", "<mo", "<mn", "<mfenced"] {
        assert!(
            !rawml_lower.contains(tag),
            "MathML tag {tag} is absent from generated RawML"
        );
    }

    let body = target.body_text();
    let body_lower = body.to_ascii_lowercase();
    assert!(
        !body_lower.contains("http://www.w3.org/1998/math/mathml"),
        "MathML namespace is absent from generated body text"
    );
    for tag in ["<math", "</math", "<mrow", "<mi", "<mo", "<mn", "<mfenced"] {
        assert!(
            !body_lower.contains(tag),
            "MathML tag {tag} is absent from generated body text"
        );
    }
    assert!(
        body.contains(
            "Before AUTH_EPC006_BEFORE x+1 AUTH_EPC006_BETWEEN xy AUTH_EPC006_AFTER after."
        ),
        "descendant text stays readable and in source order, with surrounding text: {body:?}"
    );
    assert!(
        !body.contains('(') && !body.contains(')'),
        "mfenced open/close attributes do not synthesize punctuation: {body:?}"
    );
    assert!(
        !rawml_lower.contains("kindle:embed:") && !rawml_lower.contains("<img"),
        "generated RawML has no embedded image references"
    );

    assert!(
        target
            .sections
            .iter()
            .all(|section| section.images.is_empty()),
        "fixture produces no image elements in Kindle content"
    );
    assert!(
        target.embedded_resource_numbers.is_empty(),
        "fixture produces no embedded resource references"
    );
    let resource_hrefs = target
        .resc_resource_hrefs()
        .expect("inspect output resource metadata");
    assert!(
        resource_hrefs.iter().all(|href| {
            let href = href.to_ascii_lowercase();
            ![".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp"]
                .iter()
                .any(|extension| href.ends_with(extension))
        }),
        "output resource metadata has no image references: {resource_hrefs:?}"
    );
    let fdst = target
        .header
        .fdst_record
        .expect("generated KF8 has an FDST record") as usize;
    let resc = (0..target.db.record_count())
        .find(|&index| {
            target
                .db
                .record(index)
                .is_ok_and(|record| record.starts_with(b"RESC"))
        })
        .expect("generated KF8 has a RESC record");
    let resc_record = target.db.record(resc).expect("read output RESC record");
    assert!(
        !resc_record
            .windows(b"image/".len())
            .any(|window| window == b"image/"),
        "output RESC metadata has no image media type"
    );
    assert_eq!(
        resc,
        fdst + 1,
        "the resource table has no image payload records"
    );
}
