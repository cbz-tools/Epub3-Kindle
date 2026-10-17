//! In-memory EPUB conversion API.
//!
//! This API accepts EPUB bytes and returns AZW3 or MOBI bytes. Select the
//! representation with [`ConvertOptions::format`](crate::common::ConvertOptions::format);
//! the default is AZW3. Text compression defaults to
//! [`crate::common::Compression::PalmDoc`] and can be changed with
//! [`crate::common::ConvertOptions::compression`].
//!
//! [`convert_with_warnings`] returns the output together with any non-fatal
//! [`crate::common::ConversionWarning`]s. A parse, normalization, build, or
//! serialization failure is returned as [`crate::common::Error`].
//!
//! # Example
//!
//! ```no_run
//! use epub3_kindle::{common::{Compression, ConvertOptions, OutputFormat}, memory};
//!
//! # fn convert(epub_bytes: &[u8]) -> epub3_kindle::Result<Vec<u8>> {
//! let options = ConvertOptions {
//!     format: OutputFormat::Mobi,
//!     compression: Compression::PalmDoc,
//! };
//! memory::convert(epub_bytes, &options)
//! # }
//! ```

pub use crate::api::bytes::{convert, convert_with_warnings};
