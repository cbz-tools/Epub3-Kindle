//! Authority-first EPUB -> Kindle audit E2E.
//!
//! The requirement catalog, fixed input recipes and assertion contracts were
//! frozen from external standards/product contract before production code or
//! legacy tests were consulted. The audit support code deliberately does not
//! import private production modules, serializer constants, or production parsers.

mod audit_authority;
mod audit_support;
