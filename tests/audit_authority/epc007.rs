use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::palm::{parse_fdst, reconstruct_text_raw};
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

#[test]
fn epc007_explicit_xhtml_viewport_creates_fixed_layout_page_flows() {
    let temp = TempDir::new("epc007-fixed-layout-viewport");
    let input = temp.write(
        "epc007-viewport.epub",
        &epub::epc007_viewport_fixed_layout_with_blank_page(),
    );
    let output_path = temp.path().join("epc007-viewport.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-007 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "explicit viewport fixed layout converts without warnings: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");
    assert!(
        !stderr.contains("warning["),
        "valid explicit viewport produces no warning: {stderr}"
    );

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert_eq!(
        target.exth_text(122).as_deref(),
        Some("true"),
        "fixed-layout metadata is projected"
    );
    assert_eq!(
        target.exth_text(126).as_deref(),
        Some("1200x1577"),
        "original resolution comes from the explicit XHTML viewport"
    );
    assert!(
        target.body_text().contains("AUTH_EPC007_PAGE_TWO"),
        "ordinary fixed-page body content remains in the output"
    );
    assert!(
        target.rawml.contains("kindle:flow:0001?mime=image/svg+xml")
            && target.rawml.contains("kindle:flow:0002?mime=image/svg+xml"),
        "both fixed-layout sections reference generated page presentation flows"
    );
    assert_eq!(
        target.embedded_resource_numbers.len(),
        1,
        "the illustrated page image is transported as an embedded resource"
    );

    let fdst_pointer = target.header.fdst_record.expect("KF8 has an FDST pointer");
    let coordinate = if target.header.version >= 8 && target.header.record_index > 0 {
        "section-relative"
    } else {
        "global"
    };
    let fdst_index = target
        .header
        .global_record_index(fdst_pointer, coordinate)
        .expect("resolve the KF8 FDST record");
    let fdst = parse_fdst(
        target
            .db
            .record(fdst_index)
            .expect("read the KF8 FDST record"),
    )
    .expect("parse the KF8 flow table");
    assert_eq!(fdst.flow_count, 3, "one main flow and two page flows exist");
    let raw = reconstruct_text_raw(&target.db, &target.header)
        .expect("reconstruct active KF8 content and page flows");
    let (blank_start, blank_end) = fdst.ranges[1];
    let blank_flow = std::str::from_utf8(&raw[blank_start as usize..blank_end as usize])
        .expect("blank page flow is UTF-8");
    assert!(
        blank_flow.contains(r#"width="1200" height="1577" viewBox="0 0 1200 1577""#),
        "blank page presentation uses only its explicit XHTML viewport: {blank_flow}"
    );
    let (illustrated_start, illustrated_end) = fdst.ranges[2];
    let illustrated_flow =
        std::str::from_utf8(&raw[illustrated_start as usize..illustrated_end as usize])
            .expect("illustrated page flow is UTF-8");
    assert!(
        illustrated_flow.contains("<image") && illustrated_flow.contains("kindle:embed:"),
        "existing image-backed page presentation is preserved: {illustrated_flow}"
    );
}
