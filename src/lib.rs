//! Epub3-Kindle converts supported EPUB 3 publications to KF8-only AZW3 or
//! Dual MOBI. It provides a byte-oriented Memory API, a path-oriented File API,
//! and shared option, warning, and error types in the Common API.
//!
//! - [`memory`] converts EPUB bytes to AZW3 or MOBI bytes.
//! - [`mod@file`] reads an EPUB path and selects the output format from the output
//!   extension.
//! - [`common`] provides [`ConvertOptions`], [`OutputFormat`], [`Compression`],
//!   warning types, and [`Error`] / [`Result`].
//!
//! Both conversion APIs share EPUB parsing, normalization, KF8 construction,
//! warning collection, and error handling. `ConvertOptions::default()` selects
//! AZW3 and PalmDOC compression. The Memory API uses `ConvertOptions.format`;
//! the File API selects `.azw3` or `.mobi` from the output path and ignores that
//! field.
//!
//! # Memory API: AZW3
//!
//! Pass EPUB bytes and select [`OutputFormat::Azw3`] with the options. The
//! example uses the default PalmDOC compression.
//!
//! ```no_run
//! use epub3_kindle::{common::ConvertOptions, memory};
//!
//! # fn convert(epub_bytes: &[u8]) -> epub3_kindle::Result<Vec<u8>> {
//! let azw3 = memory::convert(epub_bytes, &ConvertOptions::default())?;
//! Ok(azw3)
//! # }
//! ```
//!
//! # Memory API: MOBI
//!
//! Set [`ConvertOptions::format`] to [`OutputFormat::Mobi`] to produce MOBI
//! bytes in memory.
//!
//! ```no_run
//! use epub3_kindle::{common::{ConvertOptions, OutputFormat}, memory};
//!
//! # fn convert(epub_bytes: &[u8]) -> epub3_kindle::Result<Vec<u8>> {
//! let options = ConvertOptions {
//!     format: OutputFormat::Mobi,
//!     ..ConvertOptions::default()
//! };
//! memory::convert(epub_bytes, &options)
//! # }
//! ```
//!
//! # File API
//!
//! The File API accepts an EPUB path and writes an AZW3 or MOBI file according
//! to the case-insensitive `.azw3` or `.mobi` destination extension. It writes
//! to a same-directory temporary file and replaces the destination only after
//! serialization succeeds.
//!
//! ```no_run
//! use epub3_kindle::{common::ConvertOptions, file};
//! use std::path::Path;
//!
//! # fn convert(input: &Path, output: &Path) -> epub3_kindle::Result<()> {
//! file::convert(input, output, &ConvertOptions::default())
//! # }
//! ```
//!
//! # Output format and warnings
//!
//! Use [`OutputFormat`] to choose AZW3 or MOBI for the Memory API. For the File
//! API, choose the destination extension; `ConvertOptions.format` is ignored.
//! The `*_with_warnings` functions return a [`ConversionOutcome`] on success.
//! Inspect its [`warnings`](ConversionOutcome::warnings) alongside the output;
//! warnings describe non-fatal limitations and do not turn a successful
//! conversion into an error.
//!
//! Independent API calls can run concurrently. Conversion results do not
//! share global mutable state or require a persistent parallel runtime; large
//! image work may use scoped standard threads within a conversion. Callers must
//! coordinate concurrent writes to the same output path.
//!
//! See the [migration guide](#migration-guide) for the Rust library API changes
//! from v0.4.x to v0.5.0. The CLI options and conversion behavior remain
//! compatible.
#![doc = include_str!("migration.md")]
#![warn(missing_docs)]

mod api;
mod book;
pub mod common;
mod container;
mod css;
mod epub;
mod error;
pub mod file;
mod kf7;
mod kf8;
mod kindle;
pub mod memory;
mod mobi;
mod xhtml;

pub(crate) use api::WarningCollector;
pub use common::{
    Compression, ConversionOutcome, ConversionWarning, ConvertOptions, Error, OutputFormat, Result,
    WarningCode,
};
