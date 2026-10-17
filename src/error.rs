use thiserror::Error;

/// Result type returned by conversion and parsing operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur while reading, converting, serializing, or writing a
/// book.
///
/// EPUB ZIP/XML/input problems retain their input-oriented categories. KF8
/// construction errors use [`Kf8Build`](Self::Kf8Build), and AZW3/MOBI
/// serialization errors use [`Container`](Self::Container). File operations
/// use [`Io`](Self::Io), with a path description and the underlying I/O error.
#[derive(Debug, Error)]
pub enum Error {
    /// An input or output I/O operation failed.
    #[error("I/O error for {path}: {source}")]
    Io {
        /// The input or output path, or a description of the affected stream.
        path: String,
        #[source]
        /// The underlying operating-system I/O error.
        source: std::io::Error,
    },
    /// The input bytes are not a valid EPUB package.
    #[error("invalid EPUB: {0}")]
    InvalidEpub(String),
    /// The requested input form is not supported by the conversion API.
    #[error("unsupported input: {0}")]
    UnsupportedInput(String),
    /// The EPUB uses a package structure that this parser does not support.
    #[error("unsupported EPUB structure: {0}")]
    UnsupportedEpub(String),
    /// XHTML or CSS could not be interpreted during EPUB semantic extraction.
    #[error("invalid XHTML or CSS: {0}")]
    InvalidXhtmlCss(String),
    /// The parsed EPUB could not be normalized into the Kindle-specific IR.
    #[error("Kindle normalization failed: {0}")]
    KindleNormalization(String),
    /// The Kindle IR could not be assembled into KF8 records.
    #[error("KF8 build failed: {0}")]
    Kf8Build(String),
    /// The assembled records could not be encoded into a PalmDB/AZW3 container.
    #[error("PalmDB container failed: {0}")]
    Container(String),
    /// A requested conversion option is not supported.
    #[error("unsupported option: {0}")]
    UnsupportedOption(String),
    /// XML parsing failed while reading EPUB package or document metadata.
    #[error("XML error: {0}")]
    Xml(String),
    /// The EPUB ZIP archive could not be opened or read.
    #[error("ZIP error: {0}")]
    Zip(String),
    /// A required output representation or KF8 invariant could not be produced.
    #[error("output error: {0}")]
    Output(String),
}

impl From<quick_xml::Error> for Error {
    fn from(error: quick_xml::Error) -> Self {
        Self::Xml(error.to_string())
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Zip(error.to_string())
    }
}
