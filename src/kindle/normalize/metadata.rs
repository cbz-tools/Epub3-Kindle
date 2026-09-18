use super::super::{KindleLayout, ir::KindleMetadata};
use crate::book::Layout;

pub(super) fn normalize_metadata(
    metadata: crate::book::Metadata,
    rendition: crate::book::RenditionSemantics,
) -> KindleMetadata {
    let mut orientation = metadata.orientation.clone();
    if orientation.is_none() {
        orientation = match rendition.spread {
            Some(crate::book::RenditionSpread::Landscape) => Some("landscape".to_owned()),
            Some(crate::book::RenditionSpread::Portrait) => Some("portrait".to_owned()),
            _ => None,
        };
    }
    let mut authors = Vec::new();
    let mut contributors = Vec::new();
    for creator in &metadata.creators {
        match creator.role.as_deref() {
            Some(role) if is_author_role(role) => authors.push(creator.value.clone()),
            Some(_) => contributors.push(creator.value.clone()),
            None if authors.is_empty() => authors.push(creator.value.clone()),
            None => contributors.push(creator.value.clone()),
        }
    }
    contributors.extend(metadata.contributors);
    if authors.is_empty() {
        if let Some(creator) = metadata.creator.as_ref() {
            authors.push(creator.clone());
        }
    }
    KindleMetadata {
        title: metadata.title,
        creator: metadata.creator,
        authors,
        contributors,
        language: metadata.language,
        identifier: metadata.identifier,
        publication_date: metadata.publication_date,
        modified: metadata.modified,
        publisher: metadata.publisher,
        description: metadata.description,
        cover_resource_id: metadata.cover,
        is_fixed_layout: metadata.is_fixed_layout,
        primary_writing_mode: metadata.primary_writing_mode,
        book_type: metadata.book_type,
        orientation,
        orientation_lock: metadata.orientation_lock,
        original_resolution: metadata.original_resolution,
        rendition_viewport: metadata.rendition_viewport,
        title_file_as: metadata.title_file_as,
        creator_file_as: metadata.creator_file_as,
        publisher_file_as: metadata.publisher_file_as,
        rendition,
    }
}

fn is_author_role(role: &str) -> bool {
    matches!(role.trim().to_ascii_lowercase().as_str(), "aut" | "author")
}

impl From<Layout> for KindleLayout {
    fn from(layout: Layout) -> Self {
        Self {
            writing_mode: layout.writing_mode,
            page_progression: layout.page_progression,
            direction: layout.direction,
        }
    }
}
