use crate::book::{Direction, PageProgression, RenditionSemantics, WritingMode};

#[derive(Debug, Clone)]
pub struct KindleBook {
    pub metadata: KindleMetadata,
    pub layout: KindleLayout,
    pub sections: Vec<KindleSection>,
    pub navigation: Vec<KindleNavigationItem>,
    pub landmarks: Vec<KindleLandmark>,
    pub resources: Vec<KindleResource>,
}

#[derive(Debug, Clone)]
pub struct KindleResource {
    pub id: String,
    pub href: String,
    pub media_type: String,
    pub properties: Vec<String>,
    pub data: Vec<u8>,
}

pub(crate) type KindlePageProgression = PageProgression;
pub(crate) type KindleWritingMode = WritingMode;

#[derive(Debug, Clone, Default)]
pub struct KindleMetadata {
    pub title: Option<String>,
    pub creator: Option<String>,
    pub authors: Vec<String>,
    pub contributors: Vec<String>,
    pub language: Option<String>,
    // Semantic source metadata is retained through the target projection.
    pub identifier: Option<String>,
    pub publication_date: Option<String>,
    #[allow(dead_code)]
    pub modified: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub cover_resource_id: Option<String>,
    pub is_fixed_layout: bool,
    pub primary_writing_mode: Option<String>,
    pub book_type: Option<String>,
    #[allow(dead_code)]
    pub orientation: Option<String>,
    pub orientation_lock: Option<String>,
    pub original_resolution: Option<String>,
    pub rendition_viewport: Option<String>,
    pub title_file_as: Option<String>,
    pub creator_file_as: Option<String>,
    pub publisher_file_as: Option<String>,
    /// Typed publication-level rendition semantics consumed by RESC metadata
    /// projection. Item-level rendition stays on each `KindleSection`.
    pub rendition: RenditionSemantics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindleLayout {
    pub writing_mode: WritingMode,
    pub page_progression: PageProgression,
    pub direction: Direction,
}

#[derive(Debug, Clone)]
pub struct KindleSection {
    pub id: String,
    pub href: String,
    pub source_xhtml: String,
    /// Whether the source spine document itself is an SVG resource rather
    /// than XHTML that happens to contain inline SVG markup.
    pub is_svg_document: bool,
    /// Stylesheet hrefs linked by this document, kept in the Kindle IR so the
    /// KF8 writer does not emit unreferenced manifest CSS into the CSS flow.
    pub referenced_styles: Vec<String>,
    /// Canonical hrefs of linked stylesheets deliberately omitted during EPUB
    /// loading; only these missing stylesheet links bypass document lookup.
    pub dropped_stylesheets: Vec<String>,
    /// Numeric dimensions from this section's explicit XHTML viewport, when
    /// present. Used only as evidence for fixed-page presentation lowering.
    pub page_viewport: Option<String>,
    pub linear: bool,
    pub layout: KindleLayoutSemantic,
    pub rendition: RenditionSemantics,
    /// Original EPUB spine itemref properties retained for RESC serialization.
    pub source_properties: Vec<String>,
    /// Source spine position; `None` identifies a synthetic Kindle section.
    pub source_spine_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KindleLayoutSemantic {
    #[default]
    Reflowable,
    PrePaginated,
}

#[derive(Debug, Clone, Default)]
pub struct KindleNavigationItem {
    pub label: String,
    pub href: String,
    pub children: Vec<KindleNavigationItem>,
}

#[derive(Debug, Clone, Default)]
pub struct KindleLandmark {
    pub kind: String,
    pub label: String,
    pub href: String,
}
