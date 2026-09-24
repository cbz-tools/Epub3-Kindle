use std::io::{Cursor, Read};
use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::palm::PalmDb;
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

fn assert_cli_output_is_structurally_valid(case: &str, input: &[u8], marker: &str) {
    let temp = TempDir::new(case);
    let input_path = temp.write(&format!("{case}.epub"), input);
    let output_path = temp.path().join(format!("{case}.azw3"));
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input_path)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert!(
        cli.status.code().is_some_and(|code| code == 0 || code == 1),
        "{case} must complete conversion, not exit with a fatal rejection: {stderr}"
    );
    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    assert!(!artifact.is_empty(), "{case} artifact is non-empty");

    let palm = PalmDb::parse(&artifact).expect("independent PalmDB structural validation");
    assert!(palm.record_count() > 1, "AZW3 contains PalmDB records");
    let target = TargetProjection::parse(&artifact).expect("independent KF8 output inspection");
    assert!(
        target.body_text().contains(marker),
        "readable source content survives {case}: {}",
        target.body_text()
    );
}

fn first_zip_entry_name(input: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(input)).expect("synthetic EPUB ZIP parses");
    archive
        .by_index(0)
        .expect("synthetic EPUB has a first entry")
        .name()
        .to_owned()
}

#[test]
fn epc015_nonfirst_mimetype_container_converts_to_structurally_valid_output() {
    let input = epub::mimetype_not_first();
    assert_ne!(
        first_zip_entry_name(&input),
        "mimetype",
        "the fixed synthetic EPUB places another entry first"
    );
    assert_cli_output_is_structurally_valid("epc015-nonfirst-mimetype", &input, "AUTH_VALID_OCF");
}

#[test]
fn epc016_compressed_mimetype_container_converts_to_structurally_valid_output() {
    let input = epub::compressed_mimetype();
    let mut archive = zip::ZipArchive::new(Cursor::new(&input)).expect("synthetic EPUB ZIP parses");
    let mimetype = archive
        .by_name("mimetype")
        .expect("synthetic EPUB contains mimetype");
    assert_eq!(
        mimetype.compression(),
        zip::CompressionMethod::Deflated,
        "the fixed synthetic EPUB compresses mimetype"
    );
    drop(mimetype);
    assert_cli_output_is_structurally_valid("epc016-compressed-mimetype", &input, "AUTH_VALID_OCF");
}

#[test]
fn epc017_opf_package_version_2_is_not_an_immediate_rejection() {
    let input = epub::package_version_2();
    let mut archive = zip::ZipArchive::new(Cursor::new(&input)).expect("synthetic EPUB ZIP parses");
    let mut opf = String::new();
    archive
        .by_name("EPUB/package.opf")
        .expect("synthetic EPUB contains its Package Document")
        .read_to_string(&mut opf)
        .expect("synthetic Package Document is readable");
    assert!(opf.contains("version=\"2.0\""));

    assert_cli_output_is_structurally_valid("epc017-package-version-2", &input, "AUTH_VALID_OCF");
}
