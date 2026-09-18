//! Orchestrate the KF8 build pipeline and assemble the final record set.
//!
//! Subsystems own cover lowering, CSS/RawML preparation, geometry, indexes,
//! and coordinate semantics; this module coordinates their order and combines
//! their results without redefining those algorithms.

use super::builder_indexes::{Indexes, build_indexes, build_text_records};
use super::builder_prepare::{PreparedContent, prepare_content};
use super::css_flow::{css_resource_base_href, rewrite_css_assets};
use super::exth::ExthHeader;
use super::fcis::{encode_eof, encode_fcis};
use super::fdst::Fdst;
use super::flis::encode_flis;
use super::format::to_base32;
use super::fragment::{Fragment, FragmentEntry};
use super::mobi_header::MobiHeader;
use super::palmdoc::PalmDocHeader;
use super::position::PositionMap;
use super::rawml::{SectionParts, materialize_internal_links, split_section_parts};
use super::resc::encode as encode_resc;
use super::resource::{
    BinaryResourceGeometry, is_binary_resource, is_css_resource, is_font_resource,
    is_text_resource, serialize_font_resource,
};
use super::text::PalmDocCompressor;
use crate::WarningCollector;
use crate::error::Result;
use crate::kindle::{
    KindleBook, KindlePageProgression as PageProgression, KindleResource, KindleSection,
    KindleWritingMode as WritingMode, prepare_cover_resource,
};

#[derive(Debug)]
pub(crate) struct Kf8Record {
    pub(crate) data: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct Kf8Book {
    pub palm_doc: PalmDocHeader,
    pub mobi: MobiHeader,
    pub exth: ExthHeader,
    pub title: Option<String>,
    pub resource_record_count: u32,
    pub resc_record: u32,
    pub records: Vec<Kf8Record>,
}

pub(crate) struct Kf8Builder;

pub(super) struct TextGeometry {
    pub(super) sections: Vec<KindleSection>,
    pub(super) section_parts: Vec<SectionParts>,
    pub(super) position_map: PositionMap,
    pub(super) css_flows: Vec<Vec<u8>>,
    pub(super) css_flow_lengths: Vec<usize>,
    pub(super) page_flows: Vec<Vec<u8>>,
    pub(super) rawml_length: usize,
    pub(super) library_thumbnail: Option<Vec<u8>>,
}

struct PhysicalLayout {
    records: Vec<Kf8Record>,
    position_map: PositionMap,
    fdst: Fdst,
    text_length_u32: u32,
    text_record_count: usize,
    palm_doc_compression: bool,
    first_non_text_record: u32,
    fragment_record: u32,
    skel_record: u32,
    guide_record: u32,
    ncx_record: u32,
    fdst_record: u32,
    fcis_record: u32,
    flis_record: u32,
    resc_record: u32,
    geometry: BinaryResourceGeometry,
}

impl Kf8Builder {
    pub(crate) fn build_with_compression(
        mut book: KindleBook,
        palm_doc_compression: bool,
        warnings: &mut WarningCollector,
    ) -> Result<Kf8Book> {
        let mut resources = std::mem::take(&mut book.resources);
        let library_thumbnail = prepare_cover(&book, &mut resources)?;
        let cover_resource_id = book.metadata.cover_resource_id.clone();
        let prepared = prepare_content(
            &mut book.sections,
            &resources,
            library_thumbnail,
            cover_resource_id.as_deref(),
            warnings,
        )?;
        let geometry = build_geometry(prepared)?;
        book.resources = resources;
        let text = build_text_records(geometry)?;
        let indexes = build_indexes(&book, text)?;
        let layout = assemble_records(&mut book, indexes, palm_doc_compression)?;
        build_record0(book, layout)
    }
}

fn prepare_cover(book: &KindleBook, resources: &mut [KindleResource]) -> Result<Option<Vec<u8>>> {
    prepare_cover_resource(resources, book.metadata.cover_resource_id.as_deref())
}

fn build_geometry(prepared: PreparedContent<'_>) -> Result<TextGeometry> {
    let PreparedContent {
        sections,
        resource_index,
        css_resources,
        section_index,
        page_flows,
        pending_links,
        library_thumbnail,
    } = prepared;
    let mut section_parts = sections
        .iter()
        .map(|section| split_section_parts(&section.source_xhtml))
        .collect::<Result<Vec<_>>>()?;
    let position_map = PositionMap::build(&sections, &section_parts)?;
    let mut sections = sections;
    materialize_internal_links(
        &mut sections,
        &pending_links,
        &position_map,
        &mut section_parts,
    )?;
    drop(pending_links);
    let mut css_flows = Vec::with_capacity(css_resources.len());
    for &resource in &css_resources.resources {
        let css_base_href = css_resource_base_href(&section_index, resource);
        css_flows.push(rewrite_css_assets(
            &resource.data,
            &css_base_href,
            &resource_index,
            &css_resources,
        ));
    }
    let css_flow_lengths = css_flows.iter().map(Vec::len).collect::<Vec<_>>();
    let rawml_length = section_parts
        .iter()
        .try_fold(0usize, |total, parts| {
            let total = total.checked_add(parts.skeleton.len())?;
            parts
                .fragments
                .iter()
                .try_fold(total, |total, fragment| total.checked_add(fragment.len()))
        })
        .and_then(|total| {
            css_flows
                .iter()
                .try_fold(total, |total, flow| total.checked_add(flow.len()))
        })
        .and_then(|total| {
            page_flows
                .iter()
                .try_fold(total, |total, flow| total.checked_add(flow.len()))
        })
        .ok_or_else(|| crate::error::Error::Output("text length overflow".to_owned()))?;
    for section in &mut sections {
        drop(std::mem::take(&mut section.source_xhtml));
    }
    Ok(TextGeometry {
        sections,
        section_parts,
        position_map,
        css_flows,
        css_flow_lengths,
        page_flows,
        rawml_length,
        library_thumbnail,
    })
}

fn assemble_records(
    book: &mut KindleBook,
    indexes: Indexes,
    palm_doc_compression: bool,
) -> Result<PhysicalLayout> {
    let Indexes {
        resc_sections,
        position_map,
        text_records,
        indexing_tbs,
        text_length_u32,
        text_record_count,
        skel_main,
        skel_details,
        fragment_main,
        fragment_details,
        fragment_ctoc,
        guide_main,
        guide_details,
        guide_ctoc,
        ncx_main,
        ncx_details,
        ncx_ctoc,
        fdst,
        library_thumbnail,
    } = indexes;
    let mut compressor = palm_doc_compression.then(PalmDocCompressor::new);
    let mut records = Vec::with_capacity(text_record_count);
    for (record, tbs) in text_records.into_iter().zip(indexing_tbs.iter()) {
        records.push(Kf8Record {
            data: record.into_trailing_data_with_compression(tbs, compressor.as_mut()),
        });
    }
    records.push(Kf8Record {
        data: vec![0x00, 0x00],
    });
    let first_non_text_record =
        1u32.checked_add(u32::try_from(text_record_count).map_err(|_| {
            crate::error::Error::Output("text record count exceeds u32".to_owned())
        })?)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| crate::error::Error::Output("record index overflow".to_owned()))?;
    let fragment_record = first_non_text_record;
    records.push(Kf8Record {
        data: fragment_main,
    });
    for detail in fragment_details {
        records.push(Kf8Record { data: detail });
    }
    for ctoc in fragment_ctoc {
        records.push(Kf8Record { data: ctoc });
    }
    let skel_record = u32::try_from(
        records
            .len()
            .checked_add(1)
            .ok_or_else(|| crate::error::Error::Output("SKEL record index overflow".to_owned()))?,
    )
    .map_err(|_| crate::error::Error::Output("SKEL record index overflow".to_owned()))?;
    records.push(Kf8Record { data: skel_main });
    for detail in skel_details {
        records.push(Kf8Record { data: detail });
    }
    let guide_record = if guide_main.is_empty() {
        u32::MAX
    } else {
        let record = u32::try_from(records.len().checked_add(1).ok_or_else(|| {
            crate::error::Error::Output("Guide record index overflow".to_owned())
        })?)
        .map_err(|_| crate::error::Error::Output("Guide record index overflow".to_owned()))?;
        records.push(Kf8Record { data: guide_main });
        for detail in guide_details {
            records.push(Kf8Record { data: detail });
        }
        for ctoc in guide_ctoc {
            records.push(Kf8Record { data: ctoc });
        }
        record
    };
    let ncx_record = u32::try_from(
        records
            .len()
            .checked_add(1)
            .ok_or_else(|| crate::error::Error::Output("NCX record index overflow".to_owned()))?,
    )
    .map_err(|_| crate::error::Error::Output("NCX record index overflow".to_owned()))?;
    records.push(Kf8Record { data: ncx_main });
    for detail in ncx_details {
        records.push(Kf8Record { data: detail });
    }
    for ctoc in ncx_ctoc {
        records.push(Kf8Record { data: ctoc });
    }
    let fdst_record = u32::try_from(
        records
            .len()
            .checked_add(1)
            .ok_or_else(|| crate::error::Error::Output("FDST record index overflow".to_owned()))?,
    )
    .map_err(|_| crate::error::Error::Output("FDST record index overflow".to_owned()))?;
    records.push(Kf8Record {
        data: fdst.encode(),
    });
    let binary_resources = book
        .resources
        .iter()
        .filter(|resource| is_binary_resource(resource))
        .collect::<Vec<_>>();
    let resource_record_start =
        u32::try_from(records.len().checked_add(1).ok_or_else(|| {
            crate::error::Error::Output("resource record index overflow".to_owned())
        })?)
        .map_err(|_| crate::error::Error::Output("resource record index overflow".to_owned()))?;
    let geometry = BinaryResourceGeometry::compute(
        &binary_resources,
        resource_record_start,
        book.metadata.cover_resource_id.as_deref(),
        library_thumbnail.is_some(),
    )?;
    let resources = std::mem::take(&mut book.resources);
    for resource in resources {
        if is_text_resource(&resource) || is_css_resource(&resource) {
            continue;
        }
        let data = if is_font_resource(&resource) {
            serialize_font_resource(resource.data)?
        } else {
            resource.data
        };
        records.push(Kf8Record { data });
    }
    if let Some(thumbnail) = library_thumbnail {
        records.push(Kf8Record { data: thumbnail });
    }

    // RESC follows all binary resource records and precedes FLIS/FCIS/EOF.
    // The established MOBI header has no dedicated RESC pointer, so the
    // first-image/resource field below remains tied to image geometry.
    let resc_record = u32::try_from(records.len() + 1)
        .map_err(|_| crate::error::Error::Output("RESC record index overflow".to_owned()))?;
    records.push(Kf8Record {
        data: encode_resc(
            &resc_sections,
            book.metadata.rendition,
            book.metadata.rendition_viewport.as_deref(),
        )?,
    });

    let flis_record = u32::try_from(records.len() + 1)
        .map_err(|_| crate::error::Error::Output("FLIS record index overflow".to_owned()))?;
    records.push(Kf8Record {
        data: encode_flis(),
    });
    let fcis_record = u32::try_from(records.len() + 1)
        .map_err(|_| crate::error::Error::Output("FCIS record index overflow".to_owned()))?;
    records.push(Kf8Record {
        data: encode_fcis(text_length_u32)?,
    });
    records.push(Kf8Record { data: encode_eof() });

    Ok(PhysicalLayout {
        records,
        position_map,
        fdst,
        text_length_u32,
        text_record_count,
        palm_doc_compression,
        first_non_text_record,
        fragment_record,
        skel_record,
        guide_record,
        ncx_record,
        fdst_record,
        fcis_record,
        flis_record,
        resc_record,
        geometry,
    })
}

fn build_record0(book: KindleBook, layout: PhysicalLayout) -> Result<Kf8Book> {
    let PhysicalLayout {
        records,
        position_map,
        fdst,
        text_length_u32,
        text_record_count,
        palm_doc_compression,
        first_non_text_record,
        fragment_record,
        skel_record,
        guide_record,
        ncx_record,
        fdst_record,
        fcis_record,
        flis_record,
        resc_record,
        geometry,
    } = layout;
    let mut exth = ExthHeader::default();
    if book.metadata.authors.is_empty() {
        if let Some(value) = &book.metadata.creator {
            exth.push_text(100, value);
        }
    } else {
        for value in &book.metadata.authors {
            exth.push_text(100, value);
        }
    }
    for value in &book.metadata.contributors {
        exth.push_text(108, value);
    }
    if let Some(value) = &book.metadata.publisher {
        exth.push_text(101, value);
    }
    if let Some(value) = &book.metadata.description {
        exth.push_text(103, value);
    }
    if let Some(value) = &book.metadata.title {
        exth.push_text(503, value);
    }
    if let Some(value) = &book.metadata.title_file_as {
        exth.push_text(508, value);
    }
    if let Some(value) = &book.metadata.creator_file_as {
        exth.push_text(517, value);
    }
    if let Some(value) = &book.metadata.publisher_file_as {
        exth.push_text(522, value);
    }
    // Preserve the source Package Document identifier separately from the
    // publication date. EXTH 112 is the MOBI source identifier field and
    // EXTH 106 carries EPUB dc:date; dcterms:modified remains in the semantic
    // IR for consumers that need last-modified metadata.
    if let Some(value) = &book.metadata.identifier {
        exth.push_text(112, value);
    }
    if let Some(value) = &book.metadata.publication_date {
        exth.push_text(106, value);
    }
    if let Some(value) = &book.metadata.language {
        exth.push_text(524, value);
    }
    exth.push_bytes(125, geometry.count.to_be_bytes());
    if let Some(start_reading_offset) = position_map.start_reading_offset(&book.landmarks)? {
        exth.push_bytes(116, start_reading_offset.to_be_bytes());
    }
    if let Some(cover_offset) = geometry.cover_offset {
        exth.push_bytes(201, cover_offset.to_be_bytes());
    }
    if let Some(thumbnail_offset) = geometry.thumbnail_offset {
        exth.push_bytes(202, thumbnail_offset.to_be_bytes());
        exth.push_text(129, format!("kindle:embed:{}", to_base32(thumbnail_offset)));
    }
    let writing_mode = if book.metadata.is_fixed_layout {
        book.metadata
            .primary_writing_mode
            .as_deref()
            .unwrap_or_else(|| writing_mode_value(book.layout.writing_mode))
    } else {
        writing_mode_value(book.layout.writing_mode)
    };
    exth.push_text(525, writing_mode);
    exth.push_text(527, page_progression_value(book.layout.page_progression));
    if book.metadata.is_fixed_layout {
        exth.push_text(122, "true");
        if let Some(value) = book.metadata.book_type.as_deref().map(str::trim) {
            let value = if value.eq_ignore_ascii_case("comic") {
                Some("comic")
            } else if value.eq_ignore_ascii_case("children") {
                Some("children")
            } else {
                None
            };
            if let Some(value) = value {
                exth.push_text(123, value);
            }
        }
    }
    let orientation = if book.metadata.is_fixed_layout {
        book.metadata
            .orientation_lock
            .as_deref()
            .or(book.metadata.orientation.as_deref())
    } else {
        book.metadata.orientation.as_deref()
    };
    if let Some(value) = orientation {
        let value = if value.eq_ignore_ascii_case("auto") {
            "none"
        } else {
            value
        };
        exth.push_text(124, value);
    }
    if book.metadata.is_fixed_layout {
        if let Some(value) = &book.metadata.original_resolution {
            exth.push_text(126, value);
        }
    }
    let mut mobi = MobiHeader {
        first_non_text_record,
        first_resource_record: geometry.first_image_record,
        first_image_index: geometry.first_image_record,
        fcis_record,
        fcis_count: 1,
        flis_record,
        flis_count: 1,
        last_image_index: geometry.last_image_record,
        extra_data_flags: 0x0003,
        fdst_record,
        fdst_flow_count: fdst.entries.len() as u32,
        index_record: fragment_record,
        ncx_record,
        skel_record,
        guide_record,
        ..MobiHeader::default()
    };
    mobi.language = book
        .metadata
        .language
        .as_deref()
        .map(language_code)
        .unwrap_or(0);
    let title = book.metadata.title;
    Ok(Kf8Book {
        palm_doc: PalmDocHeader {
            compression: if palm_doc_compression { 2 } else { 1 },
            text_length: text_length_u32,
            record_count: text_record_count as u16,
            record_size: 4096,
            encryption: 0,
        },
        mobi,
        exth,
        title,
        resource_record_count: geometry.count,
        resc_record,
        records,
    })
}

fn writing_mode_value(writing_mode: WritingMode) -> &'static str {
    match writing_mode {
        WritingMode::HorizontalTb => "horizontal-lr",
        WritingMode::VerticalRl => "vertical-rl",
        WritingMode::VerticalLr => "vertical-lr",
    }
}

fn page_progression_value(page_progression: PageProgression) -> &'static str {
    match page_progression {
        PageProgression::Default => "default",
        PageProgression::Ltr => "ltr",
        PageProgression::Rtl => "rtl",
    }
}

fn language_code(language: &str) -> u32 {
    let mut subtags = language.trim().split(['-', '_']);
    let primary = subtags.next().unwrap_or_default().to_ascii_lowercase();
    let main = match primary.as_str() {
        "ja" | "jpn" => 0x11,
        "en" | "eng" => 0x09,
        "de" | "deu" | "ger" => 0x07,
        "es" | "spa" => 0x0a,
        "fr" | "fra" | "fre" => 0x0c,
        "it" | "ita" => 0x10,
        "ko" | "kor" => 0x12,
        "zh" | "chi" | "zho" => 0x04,
        _ => return 0,
    };
    let dialect = subtags
        .filter_map(|subtag| {
            let subtag = subtag.to_ascii_lowercase();
            match (primary.as_str(), subtag.as_str()) {
                ("en" | "eng", "us") => Some(0x04),
                ("zh" | "chi" | "zho", "cn") => Some(0x08),
                ("zh" | "chi" | "zho", "tw") => Some(0x04),
                _ => None,
            }
        })
        .next()
        .unwrap_or(0);
    main | (dialect << 8)
}

pub(super) fn fragments_from_position_map(position_map: &PositionMap) -> Fragment {
    // PositionMap owns the canonical document-local payload-stream offset.
    // FRAG tag 6 uses it for `start`; insert_position remains the separate
    // SKEL/RawML insertion coordinate and must not be used to derive it.
    Fragment {
        entries: position_map
            .fragments
            .iter()
            .map(|fragment| FragmentEntry {
                insert_position: fragment.insert_position,
                file_number: fragment.file_number,
                sequence: fragment.sequence_number,
                start: fragment.payload_stream_start,
                length: fragment.payload_length,
            })
            .collect(),
    }
}
