mod metadata;
mod navigation;
mod resources;
mod sections;

use super::data_uri::materialize_data_images;
use super::{KindleBook, KindleLayout};
use crate::book::Book;
use metadata::normalize_metadata;
use navigation::{normalize_landmarks, normalize_navigation};
use resources::{normalize_resources, project_css_resources};
use sections::{CoverPhase, normalize_sections, prepare_cover_phase};

/// Normalize the semantic Book IR into the Kindle-specific IR consumed by the
/// KF8 writer. Container and record details deliberately stay in `kf8`.
pub(crate) fn normalize(book: Book) -> KindleBook {
    let CoverPhase {
        cover_hrefs,
        cover_ids,
        keep_comic_cover_page,
        omitted_cover_hrefs,
    } = prepare_cover_phase(&book);
    let mut sections = normalize_sections(
        book.content,
        book.reading_order.items,
        &cover_hrefs,
        &cover_ids,
        keep_comic_cover_page,
    );
    let (navigation, toc_href) = normalize_navigation(
        book.navigation.items,
        book.navigation.page_list.clone(),
        &book.resources.items,
        &mut sections,
        &cover_hrefs,
        &omitted_cover_hrefs,
    );
    let landmarks = normalize_landmarks(
        book.navigation.landmarks,
        &sections,
        toc_href.as_deref(),
        &omitted_cover_hrefs,
    );
    let is_comic = book
        .metadata
        .book_type
        .as_deref()
        .is_some_and(|book_type| book_type.trim().eq_ignore_ascii_case("comic"));
    let mut resources = normalize_resources(book.resources.items, is_comic);
    materialize_data_images(&mut sections, &mut resources);
    project_css_resources(&mut resources);
    let metadata = normalize_metadata(book.metadata, book.rendition);
    KindleBook {
        metadata,
        layout: KindleLayout {
            writing_mode: book.layout.writing_mode,
            page_progression: book.layout.page_progression,
            direction: book.layout.direction,
        },
        sections,
        navigation,
        landmarks,
        resources,
    }
}
