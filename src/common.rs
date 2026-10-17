//! Shared types for the public Memory and File APIs.
//!
//! [`ConvertOptions::default()`](ConvertOptions) selects AZW3 with PalmDOC
//! compression. Use [`OutputFormat`] to select the Memory API representation;
//! the File API chooses the format from its destination extension. The
//! `*_with_warnings` functions return [`ConversionOutcome`] on success and
//! [`Result`] / [`Error`] on failure.
//!
//! This module is the canonical import path for shared API types. The crate
//! root also re-exports the same type definitions for compatibility; these are
//! aliases to the same items, not separate types.

pub use crate::api::{
    Compression, ConversionOutcome, ConversionWarning, ConvertOptions, OutputFormat, WarningCode,
};
pub use crate::error::{Error, Result};
