use super::warnings::WarningCollector;
use crate::{Compression, Error, Result, epub, kf8, kindle};

/// Parse, normalize, and build the common KF8 representation for either public API.
pub(super) fn build_kf8(
    input: &[u8],
    compression: Compression,
) -> Result<(kf8::Kf8Book, WarningCollector)> {
    let mut warnings = WarningCollector::new();
    let book = epub::parse_epub_with_warnings(input, &mut warnings)?;
    let kindle_book = kindle::normalize(book);
    let compression = match compression {
        Compression::None => kf8::TextCompression::None,
        Compression::PalmDoc => kf8::TextCompression::PalmDoc,
    };
    let kf8_book = kf8::build(kindle_book, compression, &mut warnings)
        .map_err(|error| Error::Kf8Build(error.to_string()))?;
    Ok((kf8_book, warnings))
}
