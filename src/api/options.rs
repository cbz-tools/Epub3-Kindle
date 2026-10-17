/// Selects the Kindle output representation.
///
/// The default is [`Azw3`](Self::Azw3). This option is used by the Memory API;
/// the File API selects its representation from the destination extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// KF8-only AZW3 output.
    #[default]
    Azw3,
    /// Dual MOBI output with a minimal KF7 compatibility section and KF8 reading content.
    Mobi,
}

/// Controls the PalmDOC encoding used for KF8 text records.
///
/// The default is [`PalmDoc`](Self::PalmDoc). [`None`](Self::None) stores text
/// records without compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// Store text records without PalmDOC compression (KindleGen -c0).
    None,
    /// Use PalmDOC compression (KindleGen -c1).
    #[default]
    PalmDoc,
}

/// Options shared by the Memory and File APIs.
///
/// `Default` selects AZW3 and PalmDOC compression. The Memory API uses both
/// fields to choose output representation and text compression. The File API
/// uses `compression` and selects the output representation from the
/// case-insensitive `.azw3` or `.mobi` destination extension; it ignores
/// `format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConvertOptions {
    /// The output format used by the Memory API. The default is AZW3.
    pub format: OutputFormat,
    /// The text-record compression mode. The default is PalmDOC.
    pub compression: Compression,
}
