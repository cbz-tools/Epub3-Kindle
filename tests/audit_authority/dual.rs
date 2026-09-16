use epub3_kindle::{Compression, ConvertOptions, convert_file};

use crate::audit_support::epub;
use crate::audit_support::palm::{PalmDb, assert_record_pointer, reconstruct_text};
use crate::audit_support::temp::TempDir;

fn dual(input: Vec<u8>, label: &str) -> Vec<u8> {
    let tmp = TempDir::new(label);
    let src = tmp.write("input.epub", &input);
    let dst = tmp.path().join("output.mobi");
    convert_file(
        &src,
        &dst,
        &ConvertOptions {
            compression: Compression::PalmDoc,
        },
    )
    .expect("Dual MOBI conversion");
    std::fs::read(dst).unwrap()
}

#[test]
fn dual_mobi_has_kf7_boundary_and_kf8_sections_with_consistent_pointer() {
    // REQ: DUAL-001..DUAL-005, FMT-PDB-001..FMT-PDB-003
    let out = dual(epub::minimal_reflowable(), "dual-layout");
    let db = PalmDb::parse(&out).unwrap();
    let kf7 = db.mobi_header(0).unwrap();
    assert!(
        kf7.version < 8,
        "first section of joint MOBI must be legacy/KF7 compatible"
    );
    let kf8_index = kf7
        .exth_u32(121)
        .expect("joint MOBI must identify the KF8 section") as usize;
    assert_record_pointer(&db, kf8_index as u32, "EXTH 121 KF8 boundary", false).unwrap();
    assert!(kf8_index > 0);
    assert_eq!(
        db.record(kf8_index - 1).unwrap(),
        b"BOUNDARY",
        "explicit BOUNDARY must immediately precede KF8 Record 0"
    );
    let kf8 = db.mobi_header(kf8_index).unwrap();
    assert!(kf8.version >= 8, "record after BOUNDARY must be KF8");
}

#[test]
fn dual_mobi_kf7_start_reading_and_text_geometry_are_valid_for_the_stub_itself() {
    // REQ: DUAL-006, DUAL-007, SEM-015
    let out = dual(epub::minimal_reflowable(), "dual-start");
    let db = PalmDb::parse(&out).unwrap();
    let kf7 = db.mobi_header(0).unwrap();
    let start = kf7.exth_u32(116).expect("KF7 Start Reading offset") as usize;
    assert!(
        start < kf7.text_length,
        "KF7 Start Reading must be inside the actual KF7 stub text"
    );
    let stub = reconstruct_text(&db, &kf7).unwrap();
    assert!(start < stub.len());
    assert!(
        String::from_utf8_lossy(&stub).contains("<body"),
        "KF7 compatibility text must be minimally parseable HTML"
    );
}

#[test]
fn dual_mobi_reads_the_same_kf8_semantic_markers_across_structurally_distinct_inputs() {
    // REQ: DUAL-008, SEM-001, SEM-004, SEM-009, CORPUS-004
    let cases = [
        (epub::minimal_reflowable(), "AUTH_MINIMAL_ALPHA", "dual-min"),
        (epub::vertical_japanese(), "AUTH_JA_開始", "dual-ja"),
        (epub::resource_graph(), "AUTH_RESOURCE_TWO", "dual-res"),
        (epub::cover_png(), "AUTH_COVER_BODY", "dual-cover"),
        (epub::large_text(120_000), "AUTH_LARGE_END", "dual-large"),
    ];
    for (input, marker, label) in cases {
        let out = dual(input, label);
        let db = PalmDb::parse(&out).unwrap();
        let kf7 = db.mobi_header(0).unwrap();
        let kf8_index = kf7.exth_u32(121).unwrap() as usize;
        let kf8 = db.mobi_header(kf8_index).unwrap();
        let raw = reconstruct_text(&db, &kf8).unwrap();
        assert!(
            String::from_utf8_lossy(&raw).contains(marker),
            "canonical KF8 reading rendition lost marker {marker}"
        );
    }
}
