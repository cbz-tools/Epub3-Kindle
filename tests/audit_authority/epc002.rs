use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

#[test]
fn epc002_prefixed_opf_namespace_parses_manifest_spine_and_body() {
    let temp = TempDir::new("epc002-prefixed-opf");
    let prefixed = convert_and_inspect(
        &temp,
        "epc002-prefixed-opf",
        &epub::epc002_prefixed_opf_package_namespace(),
    );
    assert_eq!(
        prefixed.sections.len(),
        1,
        "the prefixed spine item is generated"
    );
    assert!(
        prefixed
            .body_text()
            .contains("AUTH_EPC002_PREFIXED_OPF_BODY")
    );
    let spine = prefixed
        .resc_spine_properties()
        .expect("inspect generated spine derived from the prefixed manifest");
    assert_eq!(spine.len(), 1);
    assert!(spine[0].2, "the prefixed spine item remains linear");
    let navigation = prefixed
        .ncx_entries()
        .expect("inspect generated navigation from the prefixed manifest");
    assert_eq!(navigation.len(), 1);
    assert_eq!(navigation[0].label, "Prefixed chapter");
    assert_eq!(navigation[0].sequence as usize, 0);

    let default_namespace = convert_and_inspect(
        &temp,
        "epc002-default-namespace",
        &epub::minimal_reflowable(),
    );
    assert_eq!(default_namespace.sections.len(), 1);
    assert!(default_namespace.body_text().contains("AUTH_MINIMAL_ALPHA"));
}

fn convert_and_inspect(temp: &TempDir, label: &str, input: &[u8]) -> TargetProjection<'static> {
    let input_path = temp.write(&format!("{label}.epub"), input);
    let output_path = temp.path().join(format!("{label}.azw3"));
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input_path)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-002 CLI process starts");
    assert_eq!(
        cli.status.code(),
        Some(0),
        "{label} must convert successfully: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(
        cli.stderr.is_empty(),
        "{label} must convert without warnings: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(Box::leak(artifact.into_boxed_slice()))
        .expect("inspect generated Kindle content");
    assert_eq!(
        target.sections.len(),
        1,
        "one manifest spine document is present"
    );
    target
}
