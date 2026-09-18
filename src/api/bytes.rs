use super::warnings::{ConversionOutcome, WarningCollector};
use crate::{Compression, ConvertOptions, Error, Result, epub, kf8, kindle};

/// Converts an in-memory EPUB into complete KF8/AZW3 bytes.
///
/// The byte API always returns a KF8-only AZW3 representation; it does not
/// produce MOBI. Use [`convert_file`](super::file::convert_file) to select
/// AZW3 or Dual MOBI from an output path extension.
///
/// # Errors
///
/// Returns an error if the input is not a supported EPUB, or if parsing,
/// KF8 construction, or AZW3 container serialization fails.
pub fn convert_bytes(input: &[u8], options: &ConvertOptions) -> Result<Vec<u8>> {
    Ok(convert_bytes_with_warnings(input, options)?.into_value())
}

/// Converts an in-memory EPUB and returns AZW3 bytes with any warnings.
///
/// The returned [`ConversionOutcome`] keeps the output and warnings together;
/// inspect warning details with [`ConversionOutcome::warnings`] and access the
/// bytes with [`ConversionOutcome::value`] or [`ConversionOutcome::into_value`].
/// The byte API always returns KF8-only AZW3 bytes, not MOBI. Use
/// [`convert_file`](super::file::convert_file) for extension-selected file
/// output, including Dual MOBI.
///
/// # Errors
///
/// Returns an error if the input is not a supported EPUB, or if parsing,
/// KF8 construction, or AZW3 container serialization fails.
pub fn convert_bytes_with_warnings(
    input: &[u8],
    options: &ConvertOptions,
) -> Result<ConversionOutcome<Vec<u8>>> {
    let mut warnings = WarningCollector::new();
    let book = epub::parse_epub_with_warnings(input, &mut warnings)?;
    let kindle_book = kindle::normalize(book);
    let compression = match options.compression {
        Compression::None => kf8::TextCompression::None,
        Compression::PalmDoc => kf8::TextCompression::PalmDoc,
    };
    let kf8_book = kf8::build(kindle_book, compression, &mut warnings)
        .map_err(|error| Error::Kf8Build(error.to_string()))?;
    let output = kf8::serialize(kf8_book).map_err(|error| Error::Container(error.to_string()))?;
    Ok(ConversionOutcome::from_collector(output, warnings))
}
