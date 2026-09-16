use epub3_kindle::{Compression, ConvertOptions, convert_bytes, convert_file};

use crate::audit_support::epub;
use crate::audit_support::palm::{MobiHeader, PalmDb, reconstruct_text};
use crate::audit_support::temp::TempDir;

fn plain() -> ConvertOptions {
    ConvertOptions {
        compression: Compression::None,
    }
}
fn rawml(out: &[u8]) -> String {
    let db = PalmDb::parse(out).expect("PalmDB");
    let h = db.mobi_header(0).expect("MOBI header");
    String::from_utf8_lossy(&reconstruct_text(&db, &h).expect("reconstruct text")).into_owned()
}

#[derive(Debug, PartialEq, Eq)]
struct Bcp47Language {
    primary: String,
    region: Option<String>,
}

struct LanguageExpectation {
    source: &'static str,
    marker: &'static str,
    locale: u32,
}

fn language_expectation(source: &'static str) -> LanguageExpectation {
    // Independent audit authority values: MOBI stores the main language in
    // the low byte and the dialect in the immediately preceding byte.
    match source {
        "ja" => LanguageExpectation {
            source,
            marker: "AUTH_LANG_ja",
            locale: 0x0000_0011,
        },
        "en-US" => LanguageExpectation {
            source,
            marker: "AUTH_LANG_en-US",
            locale: 0x0000_0409,
        },
        "zh-CN" => LanguageExpectation {
            source,
            marker: "AUTH_LANG_zh-CN",
            locale: 0x0000_0804,
        },
        "zh-TW" => LanguageExpectation {
            source,
            marker: "AUTH_LANG_zh-TW",
            locale: 0x0000_0404,
        },
        "ko" => LanguageExpectation {
            source,
            marker: "AUTH_LANG_ko",
            locale: 0x0000_0012,
        },
        other => panic!("unexpected fixed language variation {other}"),
    }
}

fn bcp47_primary_region(tag: &str) -> Bcp47Language {
    let mut parts = tag.trim().split('-');
    let primary = parts.next().unwrap_or_default().to_ascii_lowercase();
    let region = parts
        .find(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
        .map(|part| part.to_ascii_uppercase());
    Bcp47Language { primary, region }
}

fn assert_language_metadata(header: &MobiHeader<'_>, expected: &LanguageExpectation) {
    assert_eq!(
        header.locale, expected.locale,
        "MOBI Header locale must preserve the externally defined main language/dialect for {}",
        expected.source
    );
    if let Some(target) = header.exth_text(524) {
        let source = bcp47_primary_region(expected.source);
        let target = bcp47_primary_region(&target);
        assert_eq!(
            target.primary, source.primary,
            "EXTH 524 primary language contradicts source {}",
            expected.source
        );
        if let (Some(source_region), Some(target_region)) = (source.region, target.region) {
            assert_eq!(
                target_region, source_region,
                "EXTH 524 region contradicts source {}",
                expected.source
            );
        }
    }
}

fn dual_language_output(input: &[u8], label: &str) -> Vec<u8> {
    let tmp = TempDir::new(label);
    let source = tmp.write("input.epub", input);
    let destination = tmp.path().join("output.mobi");
    convert_file(
        &source,
        &destination,
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .expect("Dual language conversion");
    std::fs::read(destination).expect("read Dual language output")
}

#[test]
fn publication_language_is_preserved_in_target_metadata_without_contradiction() {
    // REQ: SEM-016
    for (source, input) in epub::publication_languages() {
        let expected = language_expectation(source);

        let azw3 = convert_bytes(&input, &plain()).expect("AZW3 language conversion");
        let azw3_db = PalmDb::parse(&azw3).expect("AZW3 PalmDB");
        let azw3_header = azw3_db.mobi_header(0).expect("AZW3 MOBI header");
        assert_language_metadata(&azw3_header, &expected);
        assert!(
            String::from_utf8_lossy(
                &reconstruct_text(&azw3_db, &azw3_header).expect("AZW3 reading text")
            )
            .contains(expected.marker),
            "AZW3 reading content must remain associated with source language {source}"
        );

        let dual = dual_language_output(&input, &format!("language-{source}"));
        let dual_db = PalmDb::parse(&dual).expect("Dual PalmDB");
        let kf7 = dual_db.mobi_header(0).expect("Dual KF7 MOBI header");
        assert_language_metadata(&kf7, &expected);
        let kf8_index = kf7.exth_u32(121).expect("Dual MOBI KF8 boundary") as usize;
        let kf8 = dual_db
            .mobi_header(kf8_index)
            .expect("Dual KF8 MOBI header");
        assert_language_metadata(&kf8, &expected);
        assert_eq!(
            kf7.locale, kf8.locale,
            "Dual KF7 and KF8 language locales must agree for {source}"
        );
        assert!(
            String::from_utf8_lossy(
                &reconstruct_text(&dual_db, &kf8).expect("Dual KF8 reading text")
            )
            .contains(expected.marker),
            "Dual KF8 reading content must remain associated with source language {source}"
        );
    }
}

#[test]
fn nested_navigation_labels_targets_and_internal_links_survive() {
    // REQ: NAV-001..NAV-008, AMZ-NAV-001..AMZ-NAV-004, SEM-003, SEM-007
    let out =
        convert_bytes(&epub::nested_navigation(), &plain()).expect("nested navigation converts");
    let raw = rawml(&out);
    for marker in [
        "Part A",
        "Nested A.1",
        "Part B",
        "AUTH_LINK_TO_B",
        "AUTH_LINK_BACK",
    ] {
        assert!(raw.contains(marker), "missing semantic marker {marker}");
    }
}

#[test]
fn vertical_japanese_ruby_tcy_emphasis_and_unicode_are_preserved() {
    // REQ: CSS-003..CSS-006, LAYOUT-007, AMZ-CSS-001, AMZ-CSS-002, SEM-004, SEM-005, SEM-006
    let out =
        convert_bytes(&epub::vertical_japanese(), &plain()).expect("vertical Japanese converts");
    let raw = rawml(&out);
    for marker in [
        "AUTH_JA_開始",
        "漢",
        "かん",
        "字",
        "じ",
        "12",
        "圏点",
        "終端",
    ] {
        assert!(
            raw.contains(marker),
            "vertical semantic marker missing: {marker}"
        );
    }
    assert!(
        out.windows(b"vertical-rl".len())
            .any(|w| w == b"vertical-rl"),
        "vertical writing declaration must remain represented"
    );
}

#[test]
fn unicode_scalars_are_not_normalized_substituted_or_dropped() {
    // REQ: CONT-006, SEM-001, SEM-014
    let out = convert_bytes(&epub::unicode_stress(), &plain()).expect("Unicode stress converts");
    let raw = rawml(&out);
    for marker in [
        "葛󠄀",
        "辻󠄀",
        "が",
        "き゚",
        "😀",
        "𠮷野家",
        "Café",
        "한국어",
        "简体中文",
        "—―…",
    ] {
        assert!(
            raw.contains(marker),
            "Unicode marker changed or lost: {marker}"
        );
    }
}

#[test]
fn fixed_layout_metadata_and_page_order_are_preserved() {
    // REQ: LAYOUT-001..LAYOUT-006, SEM-011
    let out = convert_bytes(&epub::fixed_layout(), &plain()).expect("fixed layout converts");
    let db = PalmDb::parse(&out).expect("PalmDB");
    let h = db.mobi_header(0).expect("MOBI header");
    let rawml = reconstruct_text(&db, &h).expect("RawML");
    let raw = String::from_utf8_lossy(&rawml);
    let p1 = raw.find("AUTH_FIXED_PAGE_1").expect("page 1");
    let p2 = raw.find("AUTH_FIXED_PAGE_2").expect("page 2");
    assert!(p1 < p2, "fixed page order must follow spine");
    assert_eq!(
        h.exth_text(122).as_deref(),
        Some("true"),
        "KF8 fixed-layout signal expected"
    );
    assert!(
        h.exth_text(126).is_some(),
        "original resolution metadata must be emitted for fixed layout"
    );
}

#[test]
fn mixed_layout_keeps_reflowable_and_pre_paginated_sections_distinct() {
    // REQ: LAYOUT-002, LAYOUT-003, LAYOUT-008, SEM-012, SEM-013
    let out = convert_bytes(&epub::mixed_layout(), &plain()).expect("mixed layout converts");
    let raw = rawml(&out);
    assert!(raw.contains("AUTH_MIX_REFLOW"));
    assert!(raw.contains("AUTH_MIX_FIXED"));
    assert!(
        !out.windows(b"html.kf8-layout".len())
            .any(|w| w == b"html.kf8-layout"),
        "converter must not invent a book-wide fallback writing/layout class"
    );
}

#[test]
fn accessibility_structure_and_alternative_text_remain_in_reading_content() {
    // REQ: AMZ-A11Y-001..AMZ-A11Y-003, SEM-002, SEM-008
    let out =
        convert_bytes(&epub::accessibility_structure(), &plain()).expect("a11y structure converts");
    let raw = rawml(&out);
    for marker in [
        "AUTH_A11Y_HEADING",
        "AUTH_A11Y_CAPTION",
        "Meaningful alternative",
        "Header",
        "Descriptive internal target",
    ] {
        assert!(
            raw.contains(marker),
            "accessibility semantic missing: {marker}"
        );
    }
}

#[test]
fn ordinary_reflowable_input_stays_reflowable_without_invented_fixed_layout_or_body_typography() {
    // REQ: LAYOUT-001, LAYOUT-009, AMZ-REFLOW-001, AMZ-REFLOW-002
    let out =
        convert_bytes(&epub::minimal_reflowable(), &plain()).expect("minimal reflowable converts");
    let db = PalmDb::parse(&out).expect("PalmDB");
    let h = db.mobi_header(0).expect("MOBI header");
    let rawml = reconstruct_text(&db, &h).expect("RawML");
    let raw = String::from_utf8_lossy(&rawml);
    assert!(raw.contains("AUTH_MINIMAL_ALPHA"));
    assert_ne!(
        h.exth_text(122).as_deref(),
        Some("true"),
        "reflowable input must not be advertised as fixed layout"
    );
    assert!(
        !out.windows(b"html.kf8-layout".len())
            .any(|w| w == b"html.kf8-layout"),
        "converter must not invent a fallback fixed/layout class"
    );
    assert!(
        !out.windows(b"position:absolute".len())
            .any(|w| w == b"position:absolute"),
        "ordinary body must not be forced into fixed positioning"
    );
}

#[test]
fn embedded_font_affects_only_source_selected_content_and_does_not_replace_text_semantics() {
    // REQ: FONT-006
    let out =
        convert_bytes(&epub::embedded_font(false), &plain()).expect("embedded font input converts");
    let raw = rawml(&out);
    assert!(raw.contains("AUTH_FONT_TEXT"));
    assert!(
        out.windows(b"AuthorityAudit".len())
            .any(|w| w == b"AuthorityAudit"),
        "source-declared font family remains represented"
    );
}
