use epub3_kindle::{Compression, ConvertOptions, convert_bytes, convert_file};

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
            compression: Compression::None,
        },
    )
    .expect("Dual MOBI conversion");
    std::fs::read(dst).expect("read Dual MOBI")
}

#[test]
fn palmdb_record_table_and_mobi0_are_self_consistent() {
    // REQ: FMT-PDB-001..FMT-PDB-005, FMT-MOBI-001..FMT-MOBI-005, FMT-EXTH-001..FMT-EXTH-003
    let out = convert_bytes(
        &epub::minimal_reflowable(),
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .unwrap();
    let db = PalmDb::parse(&out).expect("PalmDB geometry");
    assert!(db.record_count() > 2);
    let h = db.mobi_header(0).expect("MOBI record 0");
    assert_eq!(h.encoding, 65001, "KF8 text encoding must be UTF-8");
    assert!(h.version >= 8, "AZW3 path must contain a KF8 MOBI header");
    assert!(
        h.record_size == 4096 || h.record_size == 0,
        "PalmDOC text record size must be coherent"
    );
    assert_record_pointer(&db, h.first_resource, "first resource", true)
        .expect("first-resource pointer");
    if let Some(indx) = h.indx_record {
        assert_record_pointer(&db, indx, "INDX", true).unwrap();
    }
}

#[test]
fn palmdoc_none_and_compressed_modes_reconstruct_identical_text() {
    // REQ: FMT-PDOC-001..FMT-PDOC-005
    let input = epub::large_text(180_000);
    let none = convert_bytes(
        &input,
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .unwrap();
    let compressed = convert_bytes(
        &input,
        &ConvertOptions {
            compression: Compression::PalmDoc,
        },
    )
    .unwrap();
    let db0 = PalmDb::parse(&none).unwrap();
    let h0 = db0.mobi_header(0).unwrap();
    let db1 = PalmDb::parse(&compressed).unwrap();
    let h1 = db1.mobi_header(0).unwrap();
    assert_eq!(h0.compression, 1);
    assert_eq!(h1.compression, 2);
    let t0 = reconstruct_text(&db0, &h0).unwrap();
    let t1 = reconstruct_text(&db1, &h1).unwrap();
    assert_eq!(
        t0, t1,
        "compression choice must not alter the logical text stream"
    );
    assert!(String::from_utf8_lossy(&t1).contains("AUTH_LARGE_END"));
}

#[test]
fn all_observable_record_pointers_are_bounded_by_actual_palmdb_count() {
    // REQ: FMT-PDB-002, FMT-MOBI-004, FMT-EXTH-002, VAL-005
    let out = convert_bytes(
        &epub::large_text(250_000),
        &ConvertOptions {
            compression: Compression::PalmDoc,
        },
    )
    .unwrap();
    let db = PalmDb::parse(&out).unwrap();
    let h = db.mobi_header(0).unwrap();
    assert_record_pointer(&db, h.first_resource, "first resource", true).unwrap();
    if let Some(indx) = h.indx_record {
        assert_record_pointer(&db, indx, "INDX", true).unwrap();
    }
    for exth in [121u32, 201, 202] {
        if let Some(v) = h.exth_u32(exth) {
            if exth == 121 {
                assert_record_pointer(&db, v, "KF8 boundary target", false).unwrap();
            }
            // Cover offsets are relative to first_resource and checked in cover test.
        }
    }
}

#[test]
fn cover_and_thumbnail_metadata_resolve_to_real_image_records() {
    // REQ: AMZ-COVER-001..AMZ-COVER-002, FMT-RES-001, FMT-COVER-001..FMT-COVER-002, SEM-009
    let out = convert_bytes(
        &epub::cover_png(),
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .unwrap();
    let db = PalmDb::parse(&out).unwrap();
    let h = db.mobi_header(0).unwrap();
    let first = h.first_resource as usize;
    assert!(first < db.record_count());
    for (ty, label) in [(201u32, "cover"), (202u32, "thumbnail")] {
        let rel = h
            .exth_u32(ty)
            .unwrap_or_else(|| panic!("missing EXTH {ty} {label}")) as usize;
        let idx = first.checked_add(rel).expect("resource index overflow");
        assert!(
            idx < db.record_count(),
            "{label} resource index outside PalmDB"
        );
        let record = db.record(idx).unwrap();
        assert!(
            record.starts_with(&[0xff, 0xd8, 0xff]),
            "{label} must resolve to Kindle-compatible JPEG bytes"
        );
    }
}

#[test]
fn embedded_font_is_emitted_as_a_font_resource_for_plain_and_idpf_obfuscated_input() {
    // REQ: FONT-001..FONT-006, SEM-010
    for (index, input) in [epub::embedded_font(false), epub::embedded_font(true)]
        .into_iter()
        .enumerate()
    {
        let out = dual(input, &format!("font-{index}"));
        let db = PalmDb::parse(&out).unwrap();
        let mut font_records = 0usize;
        for i in 0..db.record_count() {
            let rec = db.record(i).unwrap();
            if rec.starts_with(b"FONT") {
                font_records += 1;
            }
        }
        assert!(
            font_records >= 1,
            "CSS-referenced embedded font must resolve to an emitted FONT resource"
        );
    }
}

#[test]
fn auxiliary_kf8_records_that_are_emitted_have_in_range_geometry() {
    // REQ: FMT-FDST-001..FMT-FDST-003, FMT-INDX-001..FMT-INDX-003, FMT-AUX-001..FMT-AUX-003
    let out = convert_bytes(
        &epub::large_text(300_000),
        &ConvertOptions {
            compression: Compression::PalmDoc,
        },
    )
    .unwrap();
    let db = PalmDb::parse(&out).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..db.record_count() {
        let rec = db.record(i).unwrap();
        for magic in [b"FDST".as_slice(), b"INDX", b"FCIS", b"FLIS", b"DATP"] {
            if rec.starts_with(magic) {
                seen.insert(String::from_utf8_lossy(magic).to_string());
            }
        }
    }
    assert!(
        seen.contains("FDST"),
        "KF8 flow map is required for converter output with multiple flows/resources"
    );
    assert!(
        seen.contains("INDX"),
        "KF8 index records must be present for navigable output"
    );
    assert!(
        seen.contains("FCIS") && seen.contains("FLIS"),
        "control records must be physically present when advertised"
    );
    // DATP exact contents are deliberately not asserted: no authoritative exact-parity contract is available.
}

#[test]
fn kf8_auxiliary_and_index_pointers_resolve_to_expected_record_families() {
    // REQ: FMT-FDST-001, FMT-INDX-001..FMT-INDX-003, FMT-SKEL-001, FMT-FRAG-001, FMT-NCX-001, FMT-GUIDE-001, FMT-AUX-003
    let out = convert_bytes(
        &epub::large_text(220_000),
        &ConvertOptions {
            compression: Compression::PalmDoc,
        },
    )
    .unwrap();
    let db = PalmDb::parse(&out).unwrap();
    let h = db.mobi_header(0).unwrap();
    for (ptr, magic, label) in [
        (h.fdst_record, b"FDST".as_slice(), "FDST"),
        (h.ncx_record, b"INDX".as_slice(), "NCX INDX"),
        (h.fragment_index, b"INDX".as_slice(), "fragment INDX"),
        (h.skeleton_index, b"INDX".as_slice(), "skeleton INDX"),
        (h.guide_index, b"INDX".as_slice(), "guide INDX"),
    ] {
        if let Some(ptr) = ptr {
            assert_record_pointer(&db, ptr, label, false).unwrap();
            assert!(
                db.record(ptr as usize).unwrap().starts_with(magic),
                "{label} pointer does not resolve to expected record signature"
            );
        }
    }
    if let Some(ptr) = h.datp_index {
        assert_record_pointer(&db, ptr, "DATP", false).unwrap();
        assert!(
            db.record(ptr as usize).unwrap().starts_with(b"DATP"),
            "DATP pointer, if present, must resolve to DATP"
        );
    }
}

#[test]
fn writing_mode_and_page_progression_metadata_do_not_contradict_source_semantics() {
    // REQ: FMT-META-525, FMT-META-527
    let vertical = convert_bytes(
        &epub::vertical_japanese(),
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .expect("vertical converts");
    let vdb = PalmDb::parse(&vertical).expect("PalmDB");
    let vh = vdb.mobi_header(0).expect("MOBI");
    if let Some(mode) = vh.exth_text(525) {
        assert!(
            mode.contains("vertical"),
            "emitted writing-mode metadata contradicts vertical source: {mode}"
        );
    }

    let rtl = convert_bytes(
        &epub::rtl_progression(),
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .expect("rtl progression converts");
    let rdb = PalmDb::parse(&rtl).expect("PalmDB");
    let rh = rdb.mobi_header(0).expect("MOBI");
    if let Some(progression) = rh.exth_text(527) {
        assert_eq!(
            progression, "rtl",
            "emitted page-progression metadata contradicts source"
        );
    }
}

#[test]
fn emitted_resc_record_is_structurally_bounded_and_reflects_fixed_layout_source() {
    // REQ: FMT-RESC-001
    let out = convert_bytes(
        &epub::fixed_layout(),
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .expect("fixed layout converts");
    let db = PalmDb::parse(&out).expect("PalmDB");
    let mut found = None;
    for i in 0..db.record_count() {
        let record = db.record(i).expect("record");
        if record.starts_with(b"RESC") {
            found = Some(record);
            break;
        }
    }
    let resc = found.expect("fixed-layout output must expose a RESC record when the format requires resource/spine projection");
    assert!(
        resc.len() >= 16,
        "RESC record must contain its framing prefix"
    );
    let printable = String::from_utf8_lossy(resc);
    assert!(
        printable.contains("pre-paginated")
            || printable.contains("orientation")
            || printable.contains("spine"),
        "RESC payload must carry source-derived fixed-layout/resource semantics rather than an unrelated payload"
    );
}
