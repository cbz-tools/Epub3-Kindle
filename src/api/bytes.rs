use super::conversion::build_kf8;
use super::warnings::ConversionOutcome;
use crate::{ConvertOptions, Error, OutputFormat, Result, mobi};

/// Converts EPUB bytes to AZW3 or MOBI bytes.
///
/// `options.format` selects the output representation. Its default is AZW3;
/// compression defaults to PalmDOC. Use [`convert_with_warnings`] to retain
/// non-fatal conversion warnings with the output.
///
/// # Errors
///
/// Returns [`Error::InvalidEpub`](crate::common::Error::InvalidEpub), an
/// unsupported-input or unsupported-EPUB error, or an error from EPUB parsing,
/// KF8 construction, or AZW3/MOBI serialization.
pub fn convert(input: &[u8], options: &ConvertOptions) -> Result<Vec<u8>> {
    Ok(convert_with_warnings(input, options)?.into_value())
}

/// Converts EPUB bytes and returns the output with any collected warnings.
///
/// The input is an EPUB byte slice. The output is AZW3 when `options.format` is
/// [`OutputFormat::Azw3`] and Dual MOBI when it is [`OutputFormat::Mobi`]. The
/// default format is AZW3 and the default text compression is PalmDOC. On
/// success, the outcome contains the output bytes and any collected
/// non-fatal warnings.
///
/// # Errors
///
/// Returns an invalid-input or unsupported-EPUB error, or an error from EPUB
/// parsing, KF8 construction, or AZW3/MOBI serialization.
pub fn convert_with_warnings(
    input: &[u8],
    options: &ConvertOptions,
) -> Result<ConversionOutcome<Vec<u8>>> {
    let (kf8_book, warnings) = build_kf8(input, options.compression)?;
    let mut output = Vec::new();
    match options.format {
        OutputFormat::Azw3 => kf8_book
            .serialize_to_writer(&mut output, "<memory>")
            .map_err(|error| Error::Container(error.to_string()))?,
        OutputFormat::Mobi => mobi::serialize_to_writer(kf8_book, &mut output, "<memory>")
            .map_err(|error| Error::Container(error.to_string()))?,
    }
    Ok(ConversionOutcome::from_collector(output, warnings))
}
