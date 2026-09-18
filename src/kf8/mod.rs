//! KF8 pipeline components and low-level record encoders.
//!
//! MobileRead MOBI Wiki is treated as a reverse-engineered field-layout and
//! semantic reference, while calibre/Kindling and same-input KindleGen
//! comparisons provide writer behavior references. Physical Kindle behavior is
//! the final acceptance oracle; tentative or unknown fields are not promoted
//! to requirements without corroborating evidence.

mod builder;
mod builder_indexes;
mod builder_prepare;
mod css_flow;
mod div;
mod exth;
mod fcis;
mod fdst;
mod flis;
mod format;
mod fragment;
mod fragmentize;
mod guide;
mod indx;
mod mobi_header;
mod ncx;
mod palmdoc;
mod position;
mod rawml;
mod rawml_attributes;
mod rawml_layout;
mod rawml_links;
mod rawml_structure;
mod rawml_styles;
mod resc;
mod resource;
mod serializer;
mod skel;
mod tbs;
mod text;

use crate::{WarningCollector, error::Result, kindle::KindleBook};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextCompression {
    None,
    PalmDoc,
}

pub(crate) use builder::{Kf8Book, Kf8Builder, Kf8Record};
pub(crate) use flis::encode_flis;
pub(crate) use mobi_header::MobiHeader;
pub(crate) use palmdoc::PalmDocHeader;
pub(crate) use rawml::SectionParts;
pub(crate) use serializer::{
    encode_record_zero, serialize, validate_kf8_layout, validate_kf8_section_pointers,
};
pub(crate) use text::PalmDocCompressor;

pub(crate) fn build(
    book: KindleBook,
    compression: TextCompression,
    warnings: &mut WarningCollector,
) -> Result<Kf8Book> {
    Kf8Builder::build_with_compression(
        book,
        matches!(compression, TextCompression::PalmDoc),
        warnings,
    )
}
