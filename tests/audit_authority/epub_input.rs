use epub3_kindle::{
    Compression, ConvertOptions, WarningCode, convert_bytes, convert_bytes_with_warnings,
};

use crate::audit_support::epub;
use crate::audit_support::palm::{PalmDb, reconstruct_text};

fn plain() -> ConvertOptions {
    ConvertOptions {
        compression: Compression::None,
    }
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_rejected_with(input: &[u8], label: &str, expected_reason: &str) {
    let error = match convert_bytes(input, &plain()) {
        Ok(_) => panic!("{label} must be rejected with an explicit failure"),
        Err(error) => error,
    };
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains(&expected_reason.to_ascii_lowercase()),
        "{label} principal failure must mention {expected_reason:?}, got: {error}"
    );
}

#[test]
fn ocf_mimetype_and_container_contract_is_enforced() {
    // REQ: OCF-001, OCF-002
    let good = epub::minimal_reflowable();
    assert!(
        convert_bytes(&good, &plain()).is_ok(),
        "normative OCF baseline must convert"
    );

    assert_rejected_with(
        &epub::mimetype_not_first(),
        "mimetype not first",
        "mimetype",
    );
    assert_rejected_with(
        &epub::wrong_mimetype_payload(),
        "wrong mimetype payload",
        "mimetype",
    );
    assert_rejected_with(
        &epub::compressed_mimetype(),
        "compressed mimetype",
        "mimetype",
    );
}

#[test]
fn ocf_accepts_utf8_paths_and_internal_two_dot_resolution() {
    // REQ: OCF-004, PKG-009, RES-003
    for (label, input, marker) in [
        (
            "non-ASCII UTF-8 OCF path",
            epub::non_ascii_ocf_path(),
            "AUTH_NON_ASCII_OCF_PATH",
        ),
        (
            "internal two-dot path",
            epub::internal_two_dot_paths(),
            "AUTH_INTERNAL_TWO_DOT",
        ),
        (
            "Package Document relative paths",
            epub::package_relative_paths(),
            "AUTH_PACKAGE_BASE",
        ),
    ] {
        let out = convert_bytes(&input, &plain())
            .unwrap_or_else(|error| panic!("{label} must convert: {error}"));
        let db = PalmDb::parse(&out).expect("independent PalmDB parse");
        let header = db.mobi_header(0).expect("MOBI header");
        assert!(
            text(&reconstruct_text(&db, &header).expect("RawML")).contains(marker),
            "{label} marker must survive conversion"
        );
    }
}

#[test]
fn ocf_rejects_unicode_normalization_and_case_fold_collisions() {
    // REQ: OCF-006
    assert_rejected_with(
        &epub::unicode_normalization_collision(),
        "Unicode normalization collision",
        "duplicate/conflicting OCF ZIP entry path",
    );
    assert_rejected_with(
        &epub::case_fold_collision(),
        "case-fold collision",
        "duplicate/conflicting OCF ZIP entry path",
    );
}

#[test]
fn ocf_container_resolution_failures_are_explicit() {
    // REQ: OCF-003
    assert_rejected_with(
        &epub::missing_container_xml(),
        "missing META-INF/container.xml",
        "container.xml",
    );
    assert_rejected_with(
        &epub::malformed_container_xml(),
        "malformed container.xml",
        "container.xml",
    );
    assert_rejected_with(
        &epub::rootfile_missing_package_document(),
        "rootfile missing Package Document",
        "missing EPUB entry",
    );
}

#[test]
fn malformed_zip_and_encrypted_entries_have_isolated_principal_reasons() {
    // REQ: OCF-001, OCF-005, OCF-007, SEC-007, VAL-006
    assert_rejected_with(
        &epub::malformed_truncated_zip(),
        "malformed/truncated ZIP",
        "zip",
    );
    assert_rejected_with(
        &epub::encrypted_zip_entry(),
        "encrypted ZIP entry",
        "encrypted",
    );
}

#[test]
fn package_metadata_and_spine_order_survive_conversion() {
    // REQ: PKG-001..PKG-008, SEM-001, SEM-002, SEM-010
    let out = convert_bytes(&epub::metadata_and_spine(), &plain())
        .expect("metadata/spine input converts");
    let db = PalmDb::parse(&out).expect("independent PalmDB parse");
    let h = db.mobi_header(0).expect("KF8 record 0");
    let raw = text(&reconstruct_text(&db, &h).expect("reconstruct RawML"));
    let second = raw.find("AUTH_SPINE_TWO").expect("spine item 2 preserved");
    let first = raw.find("AUTH_SPINE_ONE").expect("spine item 1 preserved");
    assert!(
        second < first,
        "source spine order, not manifest order, must define reading order"
    );
}

#[test]
fn resource_paths_css_imports_shared_images_and_links_remain_resolvable() {
    // REQ: RES-001..RES-007, CONT-004, CSS-001, SEM-007, SEM-009, FMT-RES-001..FMT-RES-003
    let out = convert_bytes(&epub::resource_graph(), &plain()).expect("resource graph converts");
    let db = PalmDb::parse(&out).expect("PalmDB parse");
    let h = db.mobi_header(0).expect("MOBI header");
    let raw = text(&reconstruct_text(&db, &h).expect("RawML"));
    assert!(raw.contains("AUTH_RESOURCE_ONE"));
    assert!(raw.contains("AUTH_RESOURCE_TWO"));
    assert!(
        !raw.contains("../../images/shared.png"),
        "source-relative path must not survive as a dangling Kindle reference"
    );
    assert!(
        !raw.contains("../images/shared.png"),
        "source-relative path must be rewritten"
    );
    assert!(
        out.windows(8).any(|w| w == b"more.css") || out.windows(11).any(|w| w == b"font-weight"),
        "active imported CSS must remain represented"
    );
}

#[test]
fn missing_spine_target_is_rejected() {
    // REQ: PKG-005
    assert!(convert_bytes(&epub::broken_spine(), &plain()).is_err());
}

#[test]
fn foreign_spine_resource_without_required_fallback_is_rejected() {
    // REQ: RES-008, PKG-007
    assert!(convert_bytes(&epub::foreign_without_fallback(), &plain()).is_err());
}

#[test]
fn valid_foreign_resource_fallback_selects_local_content_document() {
    // REQ: PKG-007, RES-002
    let outcome = convert_bytes_with_warnings(&epub::foreign_with_fallback(), &plain())
        .expect("foreign spine resource with a valid fallback converts");
    assert!(
        outcome
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::W002)
    );
    let db = PalmDb::parse(&outcome.value).expect("independent PalmDB parse");
    let header = db.mobi_header(0).expect("MOBI header");
    assert!(
        text(&reconstruct_text(&db, &header).expect("RawML")).contains("AUTH_VALID_FALLBACK"),
        "fallback content document must supply the reading content"
    );
}

#[test]
fn valid_foreign_binary_fallback_replaces_unsupported_resource() {
    // REQ: RES-002, RES-008
    let outcome = convert_bytes_with_warnings(&epub::foreign_binary_with_fallback(), &plain())
        .expect("an unsupported binary with a valid fallback must convert");
    assert!(
        outcome
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::W002)
    );
    assert!(
        outcome
            .value
            .windows(b"AUTH_BINARY_FALLBACK".len())
            .any(|window| window == b"AUTH_BINARY_FALLBACK")
    );
    assert!(
        outcome
            .value
            .windows(b"AUTH_UNSUPPORTED_PRIMARY_IMAGE".len())
            .all(|window| window != b"AUTH_UNSUPPORTED_PRIMARY_IMAGE"),
        "the unsupported primary resource must not be serialized"
    );
    assert!(
        outcome
            .value
            .windows(b"\x89PNG\r\n\x1a\n".len())
            .any(|window| window == b"\x89PNG\r\n\x1a\n"),
        "the usable fallback image must be serialized"
    );
}

#[test]
fn remote_resource_is_never_silently_fetched_or_dropped() {
    // REQ: RES-006, SEC-003, SEM-007
    let input = epub::remote_resource();
    match convert_bytes(&input, &plain()) {
        Err(error) => {
            let message = error.to_string().to_ascii_lowercase();
            assert!(
                message.contains("remote") || message.contains("external"),
                "remote-only dependency failure must be explicit and remote-related: {error}"
            );
            assert!(
                !message.contains("escapes the epub root") && !message.contains("path escape"),
                "remote fixture must not fail as a path error: {error}"
            );
        }
        Ok(out) => {
            let db = PalmDb::parse(&out).expect("PalmDB parse");
            let h = db.mobi_header(0).expect("MOBI header");
            let raw = text(&reconstruct_text(&db, &h).expect("RawML"));
            assert!(
                raw.contains("https://example.invalid/never-fetch.png"),
                "if remote references are accepted they must not silently disappear"
            );
        }
    }
}

#[test]
fn scripting_input_is_sanitized_with_a_warning_and_readable_text_preserved() {
    // REQ: CONT-009, SEC-001, SEC-002
    let outcome = convert_bytes_with_warnings(&epub::scripting(), &plain())
        .expect("scripting semantics must be safely degraded");
    let db = PalmDb::parse(&outcome.value).expect("independent PalmDB parse");
    let header = db.mobi_header(0).expect("MOBI header");
    let raw = text(&reconstruct_text(&db, &header).expect("RawML"));
    assert!(
        outcome
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::W001)
    );
    assert!(raw.contains("AUTH_SCRIPT"));
    assert!(!raw.contains("document.body.dataset"));
    assert!(!raw.contains("onload=\"alert"));
}

#[test]
fn kindle_unsupported_css_that_changes_visible_content_warns_and_preserves_text() {
    // REQ: CSS-007, CSS-008, AMZ-CSS-002, SEM-006
    let outcome = convert_bytes_with_warnings(&epub::unsupported_css(), &plain())
        .expect("unsupported but processable CSS must degrade safely");
    let db = PalmDb::parse(&outcome.value).expect("independent PalmDB parse");
    let header = db.mobi_header(0).expect("MOBI header");
    let raw = text(&reconstruct_text(&db, &header).expect("RawML"));
    assert!(
        outcome
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::W004)
    );
    assert!(raw.contains("AUTH_BAD_CSS"));
}

#[test]
fn malformed_xhtml_and_navigation_contract_violations_fail_explicitly() {
    // REQ: CONT-001, NAV-001, NAV-002
    assert_rejected_with(&epub::malformed_xhtml(), "malformed XHTML", "xml");
    assert!(convert_bytes(&epub::missing_navigation(), &plain()).is_err());
    assert!(convert_bytes(&epub::duplicate_toc_navigation(), &plain()).is_err());
}

#[test]
fn unsupported_media_overlay_warns_while_container_path_escape_remains_an_error() {
    // REQ: CONT-010, SEC-004, SEC-007
    let outcome = convert_bytes_with_warnings(&epub::media_overlay(), &plain())
        .expect("media overlay playback must be safely degraded");
    let db = PalmDb::parse(&outcome.value).expect("independent PalmDB parse");
    let header = db.mobi_header(0).expect("MOBI header");
    let raw = text(&reconstruct_text(&db, &header).expect("RawML"));
    assert!(
        outcome
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::W003)
    );
    assert!(raw.contains("AUTH_MEDIA_OVERLAY"));
    assert_rejected_with(
        &epub::path_traversal_resource(),
        "path escape",
        "escapes the EPUB root",
    );
}

#[test]
fn recoverable_viewport_degradation_converts_with_w004() {
    // REQ: AMZ-CSS-004
    let outcome =
        convert_bytes_with_warnings(&epub::fixed_layout_with_degraded_viewport(), &plain())
            .expect("a recoverable viewport variant must convert");
    assert!(!outcome.value.is_empty());
    assert!(
        outcome
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::W004)
    );
}

#[test]
fn font_encryption_and_zero_byte_font_negative_boundaries_are_explicit() {
    // REQ: FONT-004, FONT-005, AMZ-QA-004
    assert!(convert_bytes(&epub::invalid_font_encryption(), &plain()).is_err());
    assert!(convert_bytes(&epub::zero_byte_font(), &plain()).is_err());
}

#[test]
fn kindle_media_queries_are_preserved_without_inverting_kf8_and_mobi_branches() {
    // REQ: CSS-009, AMZ-CSS-003
    let out = convert_bytes(&epub::media_queries(), &plain())
        .expect("supported Kindle media-query input converts");
    assert!(out.windows(b"amzn-kf8".len()).any(|w| w == b"amzn-kf8"));
    assert!(out.windows(b"amzn-mobi".len()).any(|w| w == b"amzn-mobi"));
    assert!(
        out.windows(b"AUTH_MEDIA_QUERY".len())
            .any(|w| w == b"AUTH_MEDIA_QUERY")
    );
}

#[test]
fn unsupported_scrolled_layout_is_not_silently_treated_as_equivalent_paginated_layout() {
    // REQ: LAYOUT-010
    assert!(convert_bytes(&epub::unsupported_layout_flow(), &plain()).is_err());
}

#[test]
fn ocf_rejects_unsupported_compression_and_duplicate_paths() {
    // REQ: OCF-005, OCF-006, SEC-004, SEC-007
    assert_rejected_with(
        &epub::unsupported_zip_compression(),
        "unsupported ZIP compression",
        "compression",
    );
    assert_rejected_with(
        &epub::duplicate_container_path(),
        "duplicate ZIP entry path",
        "duplicate/conflicting OCF ZIP entry path",
    );
}

#[test]
fn malformed_package_is_rejected_and_metadata_extensions_do_not_replace_required_semantics() {
    // REQ: PKG-001, PKG-002, PKG-012
    assert_rejected_with(
        &epub::malformed_package(),
        "malformed Package Document",
        "xml",
    );
    assert_rejected_with(
        &epub::unsupported_package_version(),
        "unsupported Package Document version",
        "package version",
    );
    assert_rejected_with(
        &epub::unsupported_package_namespace(),
        "unsupported Package Document namespace",
        "package namespace",
    );
    assert_rejected_with(
        &epub::invalid_unique_identifier(),
        "unresolved package unique identifier",
        "unique-identifier",
    );
    let out = convert_bytes(&epub::metadata_extension(), &plain())
        .expect("unknown optional metadata must not corrupt required publication semantics");
    let db = PalmDb::parse(&out).expect("PalmDB parse");
    let h = db.mobi_header(0).expect("MOBI header");
    let raw = text(&reconstruct_text(&db, &h).expect("RawML"));
    assert!(raw.contains("AUTH_METADATA_EXTENSION_BODY"));
    assert_rejected_with(
        &epub::unsupported_layout_flow(),
        "unsupported required rendition flow",
        "flow",
    );
}

#[test]
fn manifest_ids_are_unique_and_manifest_coherence_is_checked() {
    // REQ: PKG-004
    let valid = convert_bytes(&epub::resource_graph(), &plain())
        .expect("coherent manifest href/media-type/properties must convert");
    assert!(!valid.is_empty(), "coherent manifest must produce output");
    assert_rejected_with(
        &epub::duplicate_manifest_id(),
        "duplicate manifest ID",
        "duplicate manifest item id",
    );
}

#[test]
fn non_linear_spine_item_does_not_reorder_linear_reading_sequence() {
    // REQ: PKG-008
    let out = convert_bytes(&epub::linear_semantics(), &plain())
        .expect("linear/non-linear spine input converts");
    let db = PalmDb::parse(&out).expect("PalmDB parse");
    let h = db.mobi_header(0).expect("MOBI header");
    let raw = text(&reconstruct_text(&db, &h).expect("RawML"));
    let one = raw
        .find("AUTH_LINEAR_ONE")
        .expect("first linear section retained");
    let two = raw
        .find("AUTH_LINEAR_TWO")
        .expect("second linear section retained");
    assert!(
        one < two,
        "linear=yes reading sequence must preserve spine order"
    );
}
