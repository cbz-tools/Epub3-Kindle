//! Epub3-Kindle converts project-defined, KindleGen-compatible EPUB 3 publications into Kindle KF8-only AZW3 or canonical Dual MOBI files, with semantic and structural compatibility rather than byte-for-byte KindleGen reproduction.
//!
//! [`convert_bytes`] returns an in-memory result, while [`convert_file`] writes
//! through a same-directory temporary and atomically replaces the destination
//! after success. The `*_with_warnings` APIs return the same conversion result
//! together with all non-fatal warnings collected by the operation. Calls may run concurrently, except that callers
//! must coordinate writes to the same output path.
//!
//! The byte API returns complete KF8/AZW3 bytes. The file API selects KF8-only
//! AZW3 or Dual MOBI from the destination extension; `.mobi` output keeps the
//! canonical KF8 reading rendition and adds a minimal KF7 compatibility section.
//! File output accepts only `.azw3` and `.mobi` extensions.
//! Each operation returns a stage-specific [`Error`].
//!
//! # Library
//!
//! Convert EPUB bytes to AZW3 bytes with [`convert_bytes`], or call
//! [`convert_file`] to select AZW3 or MOBI from the output extension:
//!
//! ```no_run
//! use epub3_kindle::{ConvertOptions, convert_bytes};
//!
//! let epub_bytes: &[u8] = &[];
//! let azw3 = convert_bytes(epub_bytes, &ConvertOptions::default())?;
//! # let _ = azw3;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
#![warn(missing_docs)]

mod api;
mod book;
mod container;
mod css;
mod epub;
mod error;
mod kf7;
mod kf8;
mod kindle;
mod mobi;
mod xhtml;

pub(crate) use api::WarningCollector;
pub use api::{
    Compression, ConversionOutcome, ConversionWarning, ConvertOptions, WarningCode, convert_bytes,
    convert_bytes_with_warnings, convert_file, convert_file_with_warnings,
};
pub use error::{Error, Result};
