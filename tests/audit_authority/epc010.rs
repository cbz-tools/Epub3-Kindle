use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

#[test]
fn epc010_invalid_utf8_stylesheets_are_dropped_without_losing_valid_content() {
    let temp = TempDir::new("epc010-invalid-utf8-css");
    let input = temp.write(
        "epc010-invalid-utf8-css.epub",
        &epub::epc010_invalid_utf8_stylesheets(),
    );
    let output_path = temp.path().join("epc010-invalid-utf8-css.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-010 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(1),
        "dropping undecodable presentation CSS completes as warning-success: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");
    let warning_lines = stderr
        .lines()
        .filter(|line| line.contains("warning["))
        .collect::<Vec<_>>();
    assert_eq!(
        warning_lines.len(),
        1,
        "multiple invalid CSS files emit a single warning: {stderr}"
    );
    assert!(
        warning_lines[0].contains("warning[W004]:"),
        "non-UTF-8 CSS degradation emits W004: {stderr}"
    );
    assert!(stderr.contains("a stylesheet with invalid UTF-8 was dropped for Kindle output"));

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert!(
        target.body_text().contains("AUTH_EPC010_READABLE_BODY"),
        "readable XHTML body survives"
    );
    let compact_css = target
        .css
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    assert!(
        compact_css.contains(".good{font-weight:bold;}"),
        "the valid stylesheet remains in the generated KF8 CSS flow: {}",
        target.css
    );
    for marker in [
        "AUTH_EPC010_INVALID_ONE_PAYLOAD",
        "AUTH_EPC010_INVALID_TWO_PAYLOAD",
        ".invalid-one",
        ".invalid-two",
    ] {
        assert!(
            !target.css.contains(marker) && !target.rawml.contains(marker),
            "invalid CSS content is not transported: {marker}"
        );
    }
    assert!(
        target.rawml.contains("kindle:flow:"),
        "the valid stylesheet reference is projected to a Kindle CSS flow"
    );
    assert!(
        !stderr.contains("does not resolve to a generated document"),
        "the explicitly dropped stylesheet references do not become unresolved internal links: {stderr}"
    );
}
