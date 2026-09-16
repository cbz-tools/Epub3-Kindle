use epub3_kindle::{
    Compression, ConversionOutcome, ConvertOptions, WarningCode, convert_bytes_with_warnings,
};

#[test]
fn conversion_api_exposes_empty_warnings_for_clean_success() {
    let input = include_bytes!("fixtures/public-api-and-cli/source.epub");
    let outcome = convert_bytes_with_warnings(
        input,
        &ConvertOptions {
            compression: Compression::None,
        },
    )
    .expect("existing API fixture converts");

    assert!(!outcome.value().is_empty());
    assert!(outcome.warnings().is_empty());
}

#[test]
fn conversion_outcome_preserves_one_warning() {
    let outcome = ConversionOutcome::new("converted")
        .with_warnings([(WarningCode::W002, "rich content fallback used")]);

    assert_eq!(*outcome.value(), "converted");
    assert_eq!(outcome.warnings().len(), 1);
    assert_eq!(outcome.warnings()[0].code, WarningCode::W002);
}

#[test]
fn conversion_outcome_preserves_multiple_warnings_in_order() {
    let outcome = ConversionOutcome::new(()).with_warnings([
        (WarningCode::W001, "interactive feature degraded"),
        (WarningCode::W006, "publishing guidance exceeded"),
    ]);

    assert_eq!(outcome.warnings().len(), 2);
    assert_eq!(outcome.warnings()[0].code, WarningCode::W001);
    assert_eq!(outcome.warnings()[1].code, WarningCode::W006);
}
