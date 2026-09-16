use super::warnings::{ConversionOutcome, WarningCollector};
use crate::{Compression, ConvertOptions, Error, Result, epub, kf8, kindle};

/// Convert an in-memory EPUB into a complete KF8/AZW3 byte vector.
pub fn convert_bytes(input: &[u8], options: &ConvertOptions) -> Result<Vec<u8>> {
    Ok(convert_bytes_with_warnings(input, options)?.into_value())
}

/// Convert an in-memory EPUB and return the output together with any warnings.
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
    let kf8_book =
        kf8::build(kindle_book, compression).map_err(|error| Error::Kf8Build(error.to_string()))?;
    let output = kf8::serialize(kf8_book).map_err(|error| Error::Container(error.to_string()))?;
    Ok(ConversionOutcome::from_collector(output, warnings))
}
