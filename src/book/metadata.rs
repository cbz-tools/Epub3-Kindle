#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub creator: Option<String>,
    pub language: Option<String>,
    pub identifier: Option<String>,
    /// EPUB `dc:date`, retained independently as the publication date.
    pub publication_date: Option<String>,
    /// EPUB `dcterms:modified`, retained independently as the last-modified value.
    pub modified: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    pub is_fixed_layout: bool,
    pub primary_writing_mode: Option<String>,
    pub book_type: Option<String>,
    pub orientation: Option<String>,
    pub spread: Option<String>,
    pub flow: Option<String>,
    pub rendition_viewport: Option<String>,
    pub align_x: Option<String>,
    pub orientation_lock: Option<String>,
    pub original_resolution: Option<String>,
    /// EPUB metadata values and refinements retained until Kindle projection.
    /// These fields are crate-private because the public API intentionally
    /// keeps the canonical scalar metadata surface stable.
    pub(crate) records: Vec<MetadataRecord>,
    pub(crate) creators: Vec<CreatorMetadata>,
    pub(crate) contributors: Vec<String>,
    pub(crate) title_file_as: Option<String>,
    pub(crate) creator_file_as: Option<String>,
    pub(crate) publisher_file_as: Option<String>,
    pub(crate) collection: Vec<CollectionMetadata>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct MetadataRecord {
    pub(crate) id: Option<String>,
    pub(crate) property: String,
    pub(crate) refines: Option<String>,
    pub(crate) scheme: Option<String>,
    pub(crate) value: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CreatorMetadata {
    pub(crate) value: String,
    pub(crate) role: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct CollectionMetadata {
    pub(crate) name: String,
    pub(crate) collection_type: Option<String>,
    pub(crate) group_position: Option<String>,
}
