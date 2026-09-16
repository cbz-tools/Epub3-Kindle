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
    /// Convert one EPUB path, selecting `.azw3` or `.mobi` from the output extension.
    pub fn convert_file(&self, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<()> {
        super::file::convert_file(input, output, self)
    }

    /// Convert one EPUB path and return any warnings collected during conversion.
    pub fn convert_file_with_warnings(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> Result<ConversionOutcome<()>> {
        super::file::convert_file_with_warnings(input, output, self)
    }
}
