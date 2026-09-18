use std::path::Path;

use super::warnings::ConversionOutcome;
use crate::Result;

/// Controls the PalmDOC encoding used for KF8 text records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// Store text records without PalmDOC compression (KindleGen `-c0`).
    None,
    /// Use PalmDOC compression (KindleGen `-c1`).
    #[default]
    PalmDoc,
}

/// Options that affect conversion output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConvertOptions {
    /// The text-record compression mode. The default is PalmDOC.
    pub compression: Compression,
}

impl ConvertOptions {
    /// Converts an EPUB file, selecting AZW3 or Dual MOBI from the output extension.
    ///
    /// `.azw3` produces KF8-only AZW3; `.mobi` produces a Dual MOBI with a
    /// minimal KF7 compatibility section and the canonical KF8 reading
    /// rendition. The output is atomically replaced after serialization
    /// succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported input or output extension, input or
    /// output I/O failure, unsupported or invalid EPUB content, or KF8/MOBI
    /// construction and serialization failure.
    pub fn convert_file(&self, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<()> {
        super::file::convert_file(input, output, self)
    }

    /// Converts an EPUB file and returns any warnings collected during conversion.
    ///
    /// `.azw3` produces KF8-only AZW3; `.mobi` produces a Dual MOBI with a
    /// minimal KF7 compatibility section and the canonical KF8 reading
    /// rendition. The outcome value is `()` because output is written to the
    /// destination path; inspect warning details with
    /// [`ConversionOutcome::warnings`].
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported input or output extension, input or
    /// output I/O failure, unsupported or invalid EPUB content, or KF8/MOBI
    /// construction and serialization failure.
    pub fn convert_file_with_warnings(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> Result<ConversionOutcome<()>> {
        super::file::convert_file_with_warnings(input, output, self)
    }
}
