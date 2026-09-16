//! Resolve XHTML anchors and fragments into Kindle position coordinates.
//!
//! Structural fragmentation is implemented in `fragmentize`; this module maps
//! its output to PositionMap entries, navigation coordinates, guide positions,
//! and Start Reading offsets.

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::kindle::{KindleLandmark, KindleNavigationItem, KindleSection};
use crate::xhtml::path::normalize_path_lossy;
use crate::xhtml::scan::{Tag, tags};

use super::SectionParts;
pub(crate) use super::fragmentize::{FragmentContext, body_range};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Position data for an anchor in generated XHTML and its KF8 projections.
///
/// `normalized_offset` is source/normalized-XHTML local; `physical_rawml_offset`
/// and `rendered_offset` are absolute reconstructed-RawML coordinates.
/// `insert_position` is the SKEL/FRAG insertion coordinate, while
/// `fragment_payload_offset` is fragment-local. `pos_fid`/`pos_off` are Kindle
/// navigation coordinates, not NCX semantic offset/length ranges.
pub struct PositionMapEntry {
    pub section_index: usize,
    pub normalized_offset: u32,
    pub element_id: Option<String>,
    pub aid: Option<String>,
    pub cid: Option<String>,
    pub fragment_index: u32,
    pub file_number: u32,
    pub sequence_number: u32,
    pub insert_position: u32,
    pub fragment_payload_offset: u32,
    pub fragment_payload_length: u32,
    pub physical_rawml_offset: u32,
    pub rendered_offset: u32,
    pub ncx_offset: Option<u32>,
    pub ncx_length: Option<u32>,
    pub pos_fid: Option<u32>,
    pub pos_off: Option<u32>,
    pub(crate) element_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionMapFragment {
    pub section_index: usize,
    pub fragment_index: u32,
    pub file_number: u32,
    pub sequence_number: u32,
    /// Selector context in this writer's observed Calibre/KindleGen-compatible
    /// form. `P` denotes the current parent before descending into an
    /// oversized aid-bearing element; `S` denotes the post-descent/sibling
    /// context. This is the DOM insertion contract exercised by the
    /// Physical-Kindle-passing baseline, not a claim about every proprietary
    /// selector meaning.
    pub(crate) selector: String,
    pub insert_position: u32,
    /// Document-local payload-stream coordinate used by FRAG tag 6.
    /// This is distinct from `insert_position`, which is the SKEL/RawML
    /// insertion coordinate, and from `physical_payload_start`, which is the
    /// absolute RawML payload coordinate.
    pub(crate) payload_stream_start: u32,
    /// Absolute RawML coordinate of this payload, including prior documents
    /// and the owning skeleton. FRAG tag-6 `start` uses `payload_stream_start`
    /// instead.
    pub physical_payload_start: u32,
    pub payload_length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPosition {
    pub section_index: usize,
    pub fragment_index: usize,
    pub sequence_number: u32,
    pub insert_position: u32,
    pub payload_offset: u32,
    pub payload_length: u32,
    pub rendered_offset: u32,
}

#[derive(Debug, Clone)]
struct SectionPosition {
    href: String,
    start: u32,
    end: u32,
    body_tag_start: usize,
    body_content_start: usize,
    linear: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PositionMap {
    pub entries: Vec<PositionMapEntry>,
    pub fragments: Vec<PositionMapFragment>,
    sections: Vec<SectionPosition>,
    section_by_href: HashMap<String, usize>,
    entry_by_section_fragment: HashMap<(usize, String), usize>,
    section_first_fragment: Vec<Option<usize>>,
}

impl PositionMap {
    pub(crate) fn build(sections: &[KindleSection], parts: &[SectionParts]) -> Result<Self> {
        if sections.len() != parts.len() {
            return Err(Error::Output(
                "position map section geometry does not match sections".to_owned(),
            ));
        }
        let mut map = Self::default();
        let mut section_start = 0u32;
        let mut sequence = 0u32;
        for (section_index, (section, parts)) in sections.iter().zip(parts).enumerate() {
            let (body_tag_start, body_content_start, body_end) = body_range(&section.source_xhtml)
                .ok_or_else(|| {
                    Error::Output(format!(
                        "generated XHTML is missing a usable body in {}",
                        section.href
                    ))
                })?;
            let source_len = u32::try_from(section.source_xhtml.len())
                .map_err(|_| Error::Output("position map section exceeds u32".to_owned()))?;
            let section_end = section_start
                .checked_add(source_len)
                .ok_or_else(|| Error::Output("position map offset overflow".to_owned()))?;
            let skeleton_len = u32::try_from(parts.skeleton.len())
                .map_err(|_| Error::Output("position map skeleton exceeds u32".to_owned()))?;
            let mut payload_start = section_start
                .checked_add(skeleton_len)
                .ok_or_else(|| Error::Output("position map payload offset overflow".to_owned()))?;
            // `section_start` and `payload_start` are absolute reconstructed
            // RawML coordinates. `rendered_fragment_offset` is the separate
            // document-local FRAG payload-stream coordinate and resets here.
            let mut rendered_fragment_offset = 0u32;
            let mut section_fragments = Vec::with_capacity(parts.fragments.len());
            map.section_first_fragment.push(None);
            for (fragment_index, fragment) in parts.fragments.iter().enumerate() {
                let payload_length = u32::try_from(fragment.len())
                    .map_err(|_| Error::Output("position map fragment exceeds u32".to_owned()))?;
                let skeleton_offset = parts
                    .fragment_contexts
                    .get(fragment_index)
                    .map(|context| context.skeleton_offset)
                    .unwrap_or(parts.insertion_offset as usize);
                let insert_position = section_start
                    .checked_add(u32::try_from(skeleton_offset).map_err(|_| {
                        Error::Output("position map skeleton insertion exceeds u32".to_owned())
                    })?)
                    .and_then(|value| value.checked_add(rendered_fragment_offset))
                    .ok_or_else(|| {
                        Error::Output("position map insert offset overflow".to_owned())
                    })?;
                section_fragments.push((
                    fragment_index,
                    rendered_fragment_offset,
                    insert_position,
                    payload_start,
                    payload_length,
                    sequence,
                ));
                if fragment_index == 0 {
                    map.section_first_fragment[section_index] = Some(map.fragments.len());
                }
                map.fragments.push(PositionMapFragment {
                    section_index,
                    fragment_index: u32::try_from(fragment_index).map_err(|_| {
                        Error::Output("position map fragment index exceeds u32".to_owned())
                    })?,
                    file_number: u32::try_from(section_index).map_err(|_| {
                        Error::Output("position map file number exceeds u32".to_owned())
                    })?,
                    sequence_number: sequence,
                    selector: parts
                        .fragment_contexts
                        .get(fragment_index)
                        .map(|context| context.selector.clone())
                        .unwrap_or_default(),
                    insert_position,
                    payload_stream_start: rendered_fragment_offset,
                    physical_payload_start: payload_start,
                    payload_length,
                });
                rendered_fragment_offset = rendered_fragment_offset
                    .checked_add(payload_length)
                    .ok_or_else(|| {
                        Error::Output("position map fragment offset overflow".to_owned())
                    })?;
                payload_start = payload_start.checked_add(payload_length).ok_or_else(|| {
                    Error::Output("position map payload range overflow".to_owned())
                })?;
                sequence = sequence
                    .checked_add(1)
                    .ok_or_else(|| Error::Output("position map sequence exceeds u32".to_owned()))?;
            }
            if section_fragments.is_empty() {
                return Err(Error::Output("each section requires one FRAG".to_owned()));
            }
            let mut fragment_cursor = 0;
            for tag in tags(&section.source_xhtml) {
                let Some(aid) = tag.attribute("aid") else {
                    continue;
                };
                let in_body = (body_content_start..body_end).contains(&tag.start);
                let (fragment_index, relative) = if in_body {
                    // A parent element retained in SKEL starts before its
                    // child payload. Bind it to the next payload context;
                    // only a terminal shell uses the preceding/last context.
                    fragment_context_for_tag(
                        &parts.fragment_contexts,
                        tag.start,
                        &mut fragment_cursor,
                    )
                } else {
                    (0, 0)
                };
                let (
                    _,
                    _rendered_fragment_offset,
                    insert_position,
                    payload_start,
                    payload_length,
                    sequence_number,
                ) = section_fragments[fragment_index];
                let physical = if in_body {
                    payload_start.checked_add(relative).ok_or_else(|| {
                        Error::Output("position map physical offset overflow".to_owned())
                    })?
                } else {
                    section_start
                        .checked_add(u32::try_from(tag.start).map_err(|_| {
                            Error::Output("position map normalized offset exceeds u32".to_owned())
                        })?)
                        .ok_or_else(|| {
                            Error::Output("position map physical offset overflow".to_owned())
                        })?
                };
                let rendered_offset = section_start
                    .checked_add(u32::try_from(tag.start).map_err(|_| {
                        Error::Output("position map rendered offset exceeds u32".to_owned())
                    })?)
                    .ok_or_else(|| {
                        Error::Output("position map rendered offset overflow".to_owned())
                    })?;
                let entry_index = map.entries.len();
                let element_id = tag
                    .attribute("id")
                    .or_else(|| tag.attribute("name"))
                    .map(str::to_owned);
                if let Some(element_id) = element_id.as_ref() {
                    map.entry_by_section_fragment
                        .entry((section_index, element_id.clone()))
                        .or_insert(entry_index);
                }
                map.entries.push(PositionMapEntry {
                    section_index,
                    normalized_offset: u32::try_from(tag.start).map_err(|_| {
                        Error::Output("position map normalized offset exceeds u32".to_owned())
                    })?,
                    element_id,
                    aid: Some(aid.to_owned()),
                    cid: None,
                    fragment_index: u32::try_from(fragment_index).map_err(|_| {
                        Error::Output("position map fragment index exceeds u32".to_owned())
                    })?,
                    file_number: u32::try_from(section_index).map_err(|_| {
                        Error::Output("position map file number exceeds u32".to_owned())
                    })?,
                    sequence_number,
                    insert_position,
                    fragment_payload_offset: relative,
                    fragment_payload_length: payload_length,
                    physical_rawml_offset: physical,
                    rendered_offset,
                    ncx_offset: None,
                    ncx_length: None,
                    pos_fid: Some(sequence_number),
                    pos_off: Some(if in_body { relative } else { 0 }),
                    element_name: tag.name().to_ascii_lowercase(),
                });
            }
            let section_position = SectionPosition {
                href: section.href.clone(),
                start: section_start,
                end: section_end,
                body_tag_start,
                body_content_start,
                linear: section.linear,
            };
            let normalized_href = normalize_path_lossy(&section_position.href);
            map.section_by_href
                .entry(normalized_href)
                .or_insert(section_index);
            map.sections.push(section_position);
            section_start = section_end;
        }
        Ok(map)
    }

    pub(crate) fn resolve(&self, href: &str) -> Result<ResolvedPosition> {
        let (path, fragment) = split_href(href);
        let normalized_path = normalize_path_lossy(path);
        let section_index = self
            .section_by_href
            .get(&normalized_path)
            .copied()
            .ok_or_else(|| Error::Output(format!("position target does not resolve: {href}")))?;
        let section = &self.sections[section_index];
        if let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) {
            let entry = self
                .entry_by_section_fragment
                .get(&(section_index, fragment.to_owned()))
                .and_then(|&entry_index| self.entries.get(entry_index))
                .ok_or_else(|| {
                    Error::Output(format!("position fragment does not resolve: {href}"))
                })?;
            return Ok(ResolvedPosition {
                section_index,
                fragment_index: entry.fragment_index as usize,
                sequence_number: entry.sequence_number,
                insert_position: entry.insert_position,
                payload_offset: entry.fragment_payload_offset,
                payload_length: entry.fragment_payload_length,
                rendered_offset: entry.rendered_offset,
            });
        }
        let fragment = self
            .section_first_fragment
            .get(section_index)
            .and_then(|index| *index)
            .and_then(|index| self.fragments.get(index))
            .ok_or_else(|| Error::Output(format!("section has no fragment: {href}")))?;
        Ok(ResolvedPosition {
            section_index,
            fragment_index: fragment.fragment_index as usize,
            sequence_number: fragment.sequence_number,
            insert_position: fragment.insert_position,
            payload_offset: 0,
            payload_length: fragment.payload_length,
            rendered_offset: section
                .start
                .checked_add(
                    u32::try_from(section.body_content_start)
                        .map_err(|_| Error::Output("body position exceeds u32".to_owned()))?,
                )
                .ok_or_else(|| Error::Output("body position overflow".to_owned()))?,
        })
    }

    pub(crate) fn section_end(&self, section_index: usize) -> Option<u32> {
        self.sections.get(section_index).map(|section| section.end)
    }

    pub(crate) fn document_end(&self) -> u32 {
        self.sections.last().map(|section| section.end).unwrap_or(0)
    }

    pub(crate) fn guide_positions(
        &self,
        landmarks: &[KindleLandmark],
        navigation: &[KindleNavigationItem],
    ) -> Result<Vec<GuidePosition>> {
        let mut positions = Vec::new();
        for landmark in landmarks {
            let kind = match landmark.kind.to_ascii_lowercase().as_str() {
                "text" | "body" | "bodymatter" => "text",
                "titlepage" | "title-page" => "titlepage",
                "toc" => "toc",
                _ => continue,
            };
            let resolved = match self.resolve(&landmark.href) {
                Ok(resolved) => resolved,
                Err(error) if kind == "toc" => {
                    // EPUB navigation commonly marks the nav document itself
                    // as the TOC landmark. It is not in the spine and is not
                    // emitted as a KF8 section here. Resolve that semantic
                    // route through the first visible TOC target instead.
                    let Some(fallback) = first_navigation_target(navigation) else {
                        continue;
                    };
                    self.resolve(fallback).map_err(|fallback_error| {
                        Error::Output(format!(
                            "TOC landmark {} and its first navigation target do not resolve: {error}; {fallback_error}",
                            landmark.href
                        ))
                    })?
                }
                Err(error) => return Err(error),
            };
            positions.push(GuidePosition {
                kind: kind.to_owned(),
                label: landmark.label.clone(),
                sequence_number: resolved.sequence_number,
                off: resolved.payload_offset,
            });
        }
        // KindleGen's semantic route is ordered independently from the EPUB
        // landmarks list. Keep that stable order while retaining duplicate
        // landmarks within each semantic class.
        positions.sort_by_key(|position| match position.kind.as_str() {
            "text" => 0,
            "titlepage" => 1,
            "toc" => 2,
            _ => 3,
        });
        Ok(positions)
    }

    /// Resolve EXTH 116's absolute RawML Start Reading/Beginning position from
    /// EPUB bodymatter/text/start semantics. This is distinct from Guide
    /// fid/off navigation and follows the KindleGen/reference coordinate
    /// model. KindleGen and the acceptance book's Physical Kindle test
    /// confirmed that this target opens the summary; future artifacts still
    /// require their own device validation.
    pub(crate) fn start_reading_offset(&self, landmarks: &[KindleLandmark]) -> Result<Option<u32>> {
        for landmark in landmarks {
            let kind = landmark.kind.to_ascii_lowercase();
            if !matches!(kind.as_str(), "text" | "body" | "bodymatter" | "start") {
                continue;
            }
            let Ok(resolved) = self.resolve(&landmark.href) else {
                continue;
            };
            let section = &self.sections[resolved.section_index];
            return section
                .start
                .checked_add(u32::try_from(section.body_tag_start).map_err(|_| {
                    Error::Output("start-reading body offset exceeds u32".to_owned())
                })?)
                .map(Some)
                .ok_or_else(|| Error::Output("start-reading offset overflow".to_owned()));
        }

        self.sections
            .iter()
            .find(|section| section.linear)
            .map(|section| {
                section
                    .start
                    .checked_add(u32::try_from(section.body_tag_start).map_err(|_| {
                        Error::Output("start-reading body offset exceeds u32".to_owned())
                    })?)
                    .map(Some)
                    .ok_or_else(|| Error::Output("start-reading offset overflow".to_owned()))
            })
            .transpose()
            .map(|offset| offset.flatten())
    }
}

fn first_navigation_target(navigation: &[KindleNavigationItem]) -> Option<&str> {
    let first = navigation.first()?;
    if first.href.is_empty() {
        first_navigation_target(&first.children)
    } else {
        Some(first.href.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuidePosition {
    pub kind: String,
    pub label: String,
    pub sequence_number: u32,
    pub off: u32,
}

#[derive(Debug, Default)]
pub(crate) struct AidAssignment {
    pub(crate) xhtml: String,
    pub(crate) anchors: AnchorIndex,
}

pub(crate) fn assign_aids(source: String, next_aid: &mut u32) -> Result<AidAssignment> {
    let bytes = source.as_bytes();
    let mut result = Vec::with_capacity(source.len());
    let mut anchors = AnchorIndex::default();
    let mut cursor = 0;
    let mut changed = false;
    for tag in tags(&source) {
        if tag.start < cursor {
            continue;
        }
        result.extend_from_slice(&bytes[cursor..tag.start]);
        if let Ok(offset) = u32::try_from(result.len()) {
            anchors.add_tag(&tag, offset);
        }
        let tag_bytes = &bytes[tag.start..tag.end];
        if !is_position_bearing(&tag) || tag.source.as_bytes().get(tag.start + 1) == Some(&b'/') {
            result.extend_from_slice(tag_bytes);
        } else {
            changed = true;
            let aid = to_base32(*next_aid);
            *next_aid = next_aid
                .checked_add(1)
                .ok_or_else(|| Error::Output("aid numbering exceeds u32".to_owned()))?;
            result.extend_from_slice(&replace_or_add_attribute(tag_bytes, b"aid", aid.as_bytes()));
        }
        cursor = tag.end;
    }
    if !changed {
        return Ok(AidAssignment {
            xhtml: source,
            anchors,
        });
    }
    result.extend_from_slice(&bytes[cursor..]);
    Ok(AidAssignment {
        xhtml: String::from_utf8(result)
            .map_err(|_| Error::Output("generated XHTML is not valid UTF-8".to_owned()))?,
        anchors,
    })
}

/// Decompose the body with the bounded DOM-context strategy observed in
/// Calibre writer8 and KindleGen. Oversized aid-bearing elements are
/// traversed; their shells remain in SKEL and their descendants become
/// fragments carrying the `P`/`S` selector context. This preserves a
/// reconstructable DOM context, rather than treating every fragment as a
/// body-level byte chunk. The acceptance book confirmed the resulting
/// hierarchy on a Physical Kindle; this does not claim to reproduce every
/// proprietary writer8 rule. Fragment payloads remain separate from SKEL
/// insertion geometry.
fn is_position_bearing(tag: &Tag<'_>) -> bool {
    let local_name = tag.name().rsplit(':').next().unwrap_or(tag.name());
    [
        "body", "h1", "h2", "h3", "h4", "h5", "h6", "p", "div", "section", "article", "li", "nav",
        "ol", "a", "ul", "span",
    ]
    .iter()
    .any(|wanted| local_name.eq_ignore_ascii_case(wanted))
        || tag.attribute("id").is_some()
        || tag.attribute("name").is_some()
}

fn replace_or_add_attribute(tag: &[u8], name: &[u8], value: &[u8]) -> Vec<u8> {
    let mut cursor = 1usize;
    while cursor < tag.len() && !tag[cursor].is_ascii_whitespace() && tag[cursor] != b'>' {
        cursor += 1;
    }
    while cursor < tag.len() {
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || tag[cursor] == b'>' || tag[cursor] == b'/' {
            break;
        }
        let name_start = cursor;
        while cursor < tag.len()
            && !tag[cursor].is_ascii_whitespace()
            && !matches!(tag[cursor], b'=' | b'>')
        {
            cursor += 1;
        }
        let attribute_name = &tag[name_start..cursor];
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || tag[cursor] != b'=' {
            cursor += 1;
            continue;
        }
        cursor += 1;
        while cursor < tag.len() && tag[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let quote = tag.get(cursor).copied();
        if !matches!(quote, Some(b'"') | Some(b'\'')) {
            continue;
        }
        let value_start = cursor + 1;
        let Some(relative_end) = tag[value_start..]
            .iter()
            .position(|byte| *byte == quote.unwrap())
        else {
            break;
        };
        let value_end = value_start + relative_end;
        if attribute_name.eq_ignore_ascii_case(name) {
            let mut output = tag.to_vec();
            output.splice(value_start..value_end, value.iter().copied());
            return output;
        }
        cursor = value_end + 1;
    }
    let insert = if tag.ends_with(b"/>") {
        tag.len() - 2
    } else {
        tag.len() - 1
    };
    let mut output = Vec::with_capacity(tag.len() + value.len() + name.len() + 4);
    output.extend_from_slice(&tag[..insert]);
    output.extend_from_slice(b" aid=\"");
    output.extend_from_slice(value);
    output.extend_from_slice(b"\"");
    output.extend_from_slice(&tag[insert..]);
    output
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AnchorIndex {
    offsets: HashMap<String, u32>,
}

impl AnchorIndex {
    pub(crate) fn new(source: &str) -> Self {
        let mut offsets = HashMap::new();
        for tag in tags(source) {
            let Some(offset) = u32::try_from(tag.start).ok() else {
                continue;
            };
            for attribute in ["id", "name"] {
                if let Some(value) = tag.attribute(attribute) {
                    if !offsets.contains_key(value) {
                        offsets.insert(value.to_owned(), offset);
                    }
                }
            }
        }
        Self { offsets }
    }

    fn add_tag(&mut self, tag: &Tag<'_>, offset: u32) {
        for attribute in ["id", "name"] {
            if let Some(value) = tag.attribute(attribute) {
                if !self.offsets.contains_key(value) {
                    self.offsets.insert(value.to_owned(), offset);
                }
            }
        }
    }

    pub(crate) fn offset(&self, fragment: &str) -> Option<u32> {
        self.offsets.get(fragment).copied()
    }
}

#[allow(dead_code)]
pub(crate) fn anchor_offset(source: &str, fragment: &str) -> Option<u32> {
    AnchorIndex::new(source).offset(fragment)
}

fn fragment_context_for_tag(
    contexts: &[FragmentContext],
    tag_start: usize,
    fragment_cursor: &mut usize,
) -> (usize, u32) {
    // `fragmentize_body` appends chunks in source order, `merge_fragment_chunks`
    // preserves that order, and `split_section_parts` maps those chunks to
    // contexts without reordering them. Since `tags` also yields source order,
    // the first context whose end is after this tag is a monotonic cursor.
    while *fragment_cursor < contexts.len() && contexts[*fragment_cursor].source_end <= tag_start {
        *fragment_cursor += 1;
    }

    let Some(context) = contexts.get(*fragment_cursor) else {
        return (contexts.len().saturating_sub(1), 0);
    };
    if context.source_start <= tag_start {
        return (
            *fragment_cursor,
            u32::try_from(tag_start - context.source_start).unwrap_or(0),
        );
    }

    // The cursor is the first context after a gap, matching the old
    // first-after fallback. If there is no such context, the last-context
    // fallback is returned above.
    (*fragment_cursor, 0)
}

fn split_href(href: &str) -> (&str, Option<&str>) {
    let Some(hash) = href.find('#') else {
        return (href.split('?').next().unwrap_or(href), None);
    };
    (&href[..hash], href[hash + 1..].split('?').next())
}

fn to_base32(value: u32) -> String {
    const DIGITS: &[u8; 32] = b"0123456789ABCDEFGHIJKLMNOPQRSTUV";
    let mut value = value;
    let mut digits = Vec::new();
    loop {
        digits.push(DIGITS[(value % 32) as usize]);
        value /= 32;
        if value == 0 {
            break;
        }
    }
    digits.reverse();
    String::from_utf8(digits).expect("aid alphabet is ASCII")
}
