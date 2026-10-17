//! File-based EPUB conversion API.
//!
//! This API reads an EPUB path and writes to a destination path. The input must
//! have an `.epub` extension; the output extension selects KF8-only AZW3 for
//! `.azw3` or Dual MOBI for `.mobi`. Both extensions are matched without regard
//! to ASCII letter case. [`ConvertOptions::format`](crate::common::ConvertOptions::format)
//! is ignored; [`ConvertOptions::compression`](crate::common::ConvertOptions::compression)
//! applies to the output.
//!
//! Output is serialized to a same-directory temporary file, flushed and synced,
//! then moved over the destination only after serialization succeeds. The
//! caller must coordinate concurrent conversions targeting the same output
//! path.
//!
//! [`convert_with_warnings`] returns the collected non-fatal warnings in a
//! [`crate::common::ConversionOutcome`] whose value is `()`. Invalid
//! extensions, I/O failures, unsupported EPUB structures, and KF8/MOBI build or
//! serialization failures are returned as [`crate::common::Error`].
//!
//! # Example
//!
//! ```no_run
//! use epub3_kindle::{common::ConvertOptions, file};
//!
//! # fn convert() -> epub3_kindle::Result<()> {
//! file::convert("book.epub", "book.azw3", &ConvertOptions::default())
//! # }
//! ```

pub use crate::api::file::{convert, convert_with_warnings};
