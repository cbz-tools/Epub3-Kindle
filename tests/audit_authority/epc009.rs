use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

#[test]
fn epc009_inline_style_cdata_delimiters_are_removed_and_css_survives() {
    let temp = TempDir::new("epc009-inline-style-cdata");
    let input = temp.write(
        "epc009-inline-style-cdata.epub",
        &epub::epc009_inline_style_cdata(),
    );
    let output_path = temp.path().join("epc009-inline-style-cdata.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-009 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "inline style CDATA must convert successfully: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");
    assert!(
        stderr.is_empty(),
        "fixture converts without warnings: {stderr}"
    );
    assert!(
        !stderr.contains("malformed CSS: unterminated CSS statement"),
        "CDATA delimiters must not reach CSS parsing: {stderr}"
    );

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert!(target.body_text().contains("AUTH_EPC009_CDATA_BODY"));
    assert!(target.body_text().contains("AUTH_EPC009_ORDINARY_BODY"));

    let generated = format!("{}{}", target.rawml, target.css);
    assert!(!generated.contains("<![CDATA["));
    assert!(!generated.contains("]]>"));
    let compact_css = target
        .css
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    assert!(
        compact_css.contains("body{margin:0;}"),
        "CDATA CSS rule survives: {}",
        target.css
    );
    assert!(
        compact_css.contains(".example{font-weight:bold;}"),
        "CDATA CSS declaration survives: {}",
        target.css
    );
    assert!(
        compact_css.contains(".ordinary{color:red;}"),
        "ordinary non-CDATA inline style remains handled: {}",
        target.css
    );
}
