//! KF8 text-record and index construction stages.

use super::builder::{TextGeometry, fragments_from_position_map};
use super::div::Div;
use super::fdst::Fdst;
use super::guide::Guide;
use super::ncx::Ncx;
use super::position::PositionMap;
use super::skel::{Skel, SkelEntry};
use super::text::TextRecord;
use crate::error::Result;
use crate::kindle::{KindleBook, KindleSection};

pub(super) struct TextData {
    pub(super) sections: Vec<KindleSection>,
    pub(super) skel: Skel,
    pub(super) position_map: PositionMap,
    pub(super) text_records: Vec<TextRecord>,
    pub(super) text_length_u32: u32,
    pub(super) text_record_count: usize,
    pub(super) xhtml_length_u32: u32,
    pub(super) css_flow_lengths: Vec<usize>,
    pub(super) page_flow_lengths: Vec<usize>,
    pub(super) library_thumbnail: Option<Vec<u8>>,
}

pub(super) struct Indexes {
    pub(super) resc_sections: Vec<KindleSection>,
    pub(super) position_map: PositionMap,
    pub(super) text_records: Vec<TextRecord>,
    pub(super) indexing_tbs: Vec<Vec<u8>>,
    pub(super) text_length_u32: u32,
    pub(super) text_record_count: usize,
    pub(super) skel_main: Vec<u8>,
    pub(super) skel_details: Vec<Vec<u8>>,
    pub(super) fragment_main: Vec<u8>,
    pub(super) fragment_details: Vec<Vec<u8>>,
    pub(super) fragment_ctoc: Vec<Vec<u8>>,
    pub(super) guide_main: Vec<u8>,
    pub(super) guide_details: Vec<Vec<u8>>,
    pub(super) guide_ctoc: Vec<Vec<u8>>,
    pub(super) ncx_main: Vec<u8>,
    pub(super) ncx_details: Vec<Vec<u8>>,
    pub(super) ncx_ctoc: Vec<Vec<u8>>,
    pub(super) fdst: Fdst,
    pub(super) library_thumbnail: Option<Vec<u8>>,
}

pub(super) fn build_text_records(geometry: TextGeometry) -> Result<TextData> {
    let TextGeometry {
        sections,
        section_parts,
        position_map,
        css_flows,
        css_flow_lengths,
        page_flows,
        rawml_length,
        library_thumbnail,
    } = geometry;
    let mut stream_chunks = Vec::with_capacity(
        section_parts
            .iter()
            .map(|parts| parts.fragments.len() + 1)
            .sum::<usize>()
            + css_flows.len()
            + page_flows.len(),
    );
    for parts in &section_parts {
        stream_chunks.push(parts.skeleton.as_slice());
        stream_chunks.extend(parts.fragments.iter().map(Vec::as_slice));
    }
    stream_chunks.extend(css_flows.iter().map(Vec::as_slice));
    stream_chunks.extend(page_flows.iter().map(Vec::as_slice));
    let text_records = TextRecord::split_chunks(&stream_chunks);
    drop(stream_chunks);
    drop(css_flows);
    let page_flow_lengths = page_flows.iter().map(Vec::len).collect::<Vec<_>>();
    drop(page_flows);
    let expected_record_count = rawml_length
        .checked_add(4096 - 1)
        .ok_or_else(|| crate::error::Error::Output("text length overflow".to_owned()))?
        / 4096;
    if text_records.len() != expected_record_count
        || text_records
            .iter()
            .take(text_records.len().saturating_sub(1))
            .any(|record| record.data.len() != 4096)
    {
        return Err(crate::error::Error::Output(
            "PalmDOC text records do not match fixed 4096-byte coordinates".to_owned(),
        ));
    }
    if text_records.len() > u16::MAX as usize {
        return Err(crate::error::Error::Output(
            "PalmDOC text record count exceeds u16".to_owned(),
        ));
    }
    let mut skel_offset = 0u32;
    let mut skel_entries = Vec::with_capacity(section_parts.len());
    let mut xhtml_length = 0usize;
    for parts in &section_parts {
        let skel_start = skel_offset;
        let skeleton_length = u32::try_from(parts.skeleton.len()).map_err(|_| {
            crate::error::Error::Output("SKEL section length exceeds u32".to_owned())
        })?;
        let fragments_length = parts.fragments.iter().try_fold(0usize, |total, fragment| {
            total
                .checked_add(fragment.len())
                .ok_or_else(|| crate::error::Error::Output("XHTML length overflow".to_owned()))
        })?;
        let fragments_length_u32 = u32::try_from(fragments_length)
            .map_err(|_| crate::error::Error::Output("SKEL position overflow".to_owned()))?;
        let section_length = parts
            .skeleton
            .len()
            .checked_add(fragments_length)
            .ok_or_else(|| crate::error::Error::Output("XHTML length overflow".to_owned()))?;
        xhtml_length = xhtml_length
            .checked_add(section_length)
            .ok_or_else(|| crate::error::Error::Output("XHTML length overflow".to_owned()))?;
        skel_offset = skel_offset
            .checked_add(skeleton_length)
            .and_then(|offset| offset.checked_add(fragments_length_u32))
            .ok_or_else(|| crate::error::Error::Output("SKEL position overflow".to_owned()))?;
        skel_entries.push(SkelEntry {
            fragment_count: u32::try_from(parts.fragments.len()).map_err(|_| {
                crate::error::Error::Output("SKEL fragment count exceeds u32".to_owned())
            })?,
            start: skel_start,
            length: skeleton_length,
        });
    }
    let skel = Skel {
        entries: skel_entries,
    };
    drop(section_parts);
    let text_length = text_records
        .iter()
        .map(|record| record.data.len())
        .try_fold(0usize, |total, length| total.checked_add(length))
        .ok_or_else(|| crate::error::Error::Output("text length overflow".to_owned()))?;
    if text_length != rawml_length {
        return Err(crate::error::Error::Output(
            "PalmDOC text length does not match the logical stream".to_owned(),
        ));
    }
    let xhtml_length_u32 = u32::try_from(xhtml_length)
        .map_err(|_| crate::error::Error::Output("XHTML length exceeds u32".to_owned()))?;
    let text_length_u32 = u32::try_from(text_length)
        .map_err(|_| crate::error::Error::Output("text length exceeds u32".to_owned()))?;
    let text_record_count = text_records.len();
    Ok(TextData {
        sections,
        skel,
        position_map,
        text_records,
        text_length_u32,
        text_record_count,
        xhtml_length_u32,
        css_flow_lengths,
        page_flow_lengths,
        library_thumbnail,
    })
}

pub(super) fn build_indexes(book: &KindleBook, text: TextData) -> Result<Indexes> {
    let TextData {
        sections,
        skel,
        position_map,
        text_records,
        text_length_u32,
        text_record_count,
        xhtml_length_u32,
        css_flow_lengths,
        page_flow_lengths,
        library_thumbnail,
    } = text;
    skel.validate()?;
    let fragments = fragments_from_position_map(&position_map);
    fragments.validate()?;
    let div = Div::for_records(text_records.len());
    div.validate(text_records.len())?;
    let mut fdst_ranges = vec![(0, xhtml_length_u32)];
    let mut flow_start = xhtml_length_u32;
    for length in css_flow_lengths {
        let length = u32::try_from(length)
            .map_err(|_| crate::error::Error::Output("CSS flow length exceeds u32".to_owned()))?;
        let flow_end = flow_start
            .checked_add(length)
            .ok_or_else(|| crate::error::Error::Output("CSS flow position overflow".to_owned()))?;
        fdst_ranges.push((flow_start, flow_end));
        flow_start = flow_end;
    }
    for length in page_flow_lengths {
        let length = u32::try_from(length)
            .map_err(|_| crate::error::Error::Output("page flow length exceeds u32".to_owned()))?;
        let flow_end = flow_start
            .checked_add(length)
            .ok_or_else(|| crate::error::Error::Output("page flow position overflow".to_owned()))?;
        fdst_ranges.push((flow_start, flow_end));
        flow_start = flow_end;
    }
    let fdst = Fdst::from_ranges(&fdst_ranges);
    fdst.validate(text_length_u32)?;
    let (skel_main, skel_details) = skel
        .encode_pair()
        .map_err(|error| crate::error::Error::Output(format!("SKEL: {error}")))?;
    let fragment_selectors = position_map
        .fragments
        .iter()
        .map(|fragment| fragment.selector.clone())
        .collect::<Vec<_>>();
    let (fragment_main, fragment_details, fragment_ctoc) = fragments
        .encode_pair_with_ctoc(&fragment_selectors)
        .map_err(|error| {
            crate::error::Error::Output(format!(
                "FRAG ({} entries): {error}",
                fragments.entries.len()
            ))
        })?;
    let ncx = Ncx::from_navigation(&book.navigation);
    let text_record_lengths = text_records
        .iter()
        .map(|record| record.data.len())
        .collect::<Vec<_>>();
    let (_, indexing_tbs) = ncx.indexing_tbs_with_position_map(
        &position_map,
        &sections,
        &text_record_lengths,
        Some(&book.navigation),
    )?;
    if indexing_tbs.len() != text_records.len() {
        return Err(crate::error::Error::Output(
            "TBS count does not match PalmDOC text record count".to_owned(),
        ));
    }
    let (ncx_main, ncx_details, ncx_ctoc) = ncx
        .encode_pair_with_position_map_and_navigation(&position_map, &sections, &book.navigation)
        .map_err(|error| crate::error::Error::Output(format!("NCX: {error}")))?;
    let guide =
        Guide::from_positions(position_map.guide_positions(&book.landmarks, &book.navigation)?);
    let (guide_main, guide_details, guide_ctoc) = guide
        .encode_pair()
        .map_err(|error| crate::error::Error::Output(format!("Guide: {error}")))?;
    Ok(Indexes {
        resc_sections: sections,
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
    })
}
