//! Build KF8 NCX/CTOC indexes from normalized navigation and position data.
//!
//! This module owns NCX semantic routing and index encoding; text-record TBS
//! calculation and trailing-byte encoding are delegated to `tbs`.

use super::indx::{
    EncodedIndexWithCtoc, RawIndexEntry, TagDefinition, encode_ctoc_entries, encode_index_pair,
};
use super::position::{PositionMap, ResolvedPosition};
use super::tbs::{
    TbsEncodingError, TbsEntry, TbsSeed, calculate_all_tbs, collect_indexing_data,
    flatten_tbs_seeds, tbs_error,
};
use crate::book::plain_display_text;
use crate::error::Result;
use crate::kindle::{KindleNavigationItem, KindleSection};

#[derive(Debug, Clone, Default)]
pub struct Ncx {
    pub items: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct NcxNode {
    label: String,
    href: String,
    depth: usize,
    parent: Option<usize>,
    first_child: Option<usize>,
    last_child: Option<usize>,
}

impl Ncx {
    /// Build the visible NCX source from the EPUB navigation tree only.
    /// Spine/read-position coverage is emitted separately through PositionMap
    /// and TBS; synthesizing visible nodes for uncovered spine sections would
    /// expose internal section nodes and change the source hierarchy.
    pub fn from_navigation(items: &[KindleNavigationItem]) -> Self {
        let mut flattened = Vec::new();
        flatten(items, &mut flattened);
        Self { items: flattened }
    }

    /// Encode using the original navigation tree when one is available.  The
    /// public `Ncx` shape remains the flat `(label, href)` API; this internal
    /// path keeps hierarchy metadata out of that interface while still
    /// emitting Kindle's optional 21/22/23 fields.
    pub(crate) fn encode_pair_with_position_map_and_navigation(
        &self,
        position_map: &PositionMap,
        sections: &[KindleSection],
        navigation: &[KindleNavigationItem],
    ) -> Result<EncodedIndexWithCtoc> {
        self.encode_pair_with_position_map_inner(position_map, sections, Some(navigation))
    }

    fn tbs_entries(&self, navigation: Option<&[KindleNavigationItem]>) -> Result<Vec<TbsEntry>> {
        let mut entries = if let Some(navigation) = navigation.filter(|items| !items.is_empty()) {
            let mut seeds = Vec::new();
            flatten_tbs_seeds(navigation, 0, None, &mut seeds);
            seeds
        } else {
            self.items
                .iter()
                .map(|_| TbsSeed {
                    entry: TbsEntry {
                        index: 0,
                        start: 0,
                        length: 0,
                        depth: 0,
                        parent: None,
                    },
                })
                .collect()
        };
        for (index, seed) in entries.iter_mut().enumerate() {
            seed.entry.index = index;
        }
        Ok(entries.into_iter().map(|seed| seed.entry).collect())
    }

    pub(crate) fn indexing_tbs_with_position_map(
        &self,
        position_map: &PositionMap,
        sections: &[KindleSection],
        text_record_lengths: &[usize],
        navigation: Option<&[KindleNavigationItem]>,
    ) -> Result<(u8, Vec<Vec<u8>>)> {
        let nodes = self.position_nodes(position_map, sections, navigation)?;
        let entries = self
            .tbs_entries(navigation)?
            .into_iter()
            .map(|mut entry| {
                let final_index = nodes
                    .iter()
                    .position(|node| node.original_index == entry.index)
                    .unwrap_or(entry.index);
                entry.index = final_index;
                entry.parent = entry.parent.map(|parent| {
                    nodes
                        .iter()
                        .position(|node| node.original_index == parent)
                        .unwrap_or(parent)
                });
                (final_index, entry)
            })
            .collect::<Vec<_>>();
        let mut entries_by_index = entries;
        entries_by_index.sort_by_key(|(index, _)| *index);
        if entries_by_index.len() != nodes.len() {
            return Err(crate::error::Error::Output(
                "TBS entries do not match PositionMap navigation entries".to_owned(),
            ));
        }
        let entries = entries_by_index
            .into_iter()
            .zip(nodes)
            .map(|((_, mut entry), node)| {
                entry.start = node.start as usize;
                entry.length = node.end.saturating_sub(node.start) as usize;
                entry
            })
            .collect::<Vec<_>>();
        let indexing_data = collect_indexing_data(&entries, text_record_lengths)?;
        match calculate_all_tbs(&indexing_data, 8) {
            Ok(tbs) => Ok((8, tbs)),
            Err(TbsEncodingError::NegativeStrandIndex) => {
                let tbs = calculate_all_tbs(&indexing_data, 5).map_err(tbs_error)?;
                Ok((5, tbs))
            }
            Err(error) => Err(tbs_error(error)),
        }
    }

    fn encode_pair_with_position_map_inner(
        &self,
        position_map: &PositionMap,
        sections: &[KindleSection],
        navigation: Option<&[KindleNavigationItem]>,
    ) -> Result<EncodedIndexWithCtoc> {
        let nodes = self.position_nodes(position_map, sections, navigation)?;
        if nodes.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        if nodes.len() > u32::MAX as usize {
            return Err(crate::error::Error::Output(
                "NCX entry count exceeds u32".to_owned(),
            ));
        }
        if !nodes.is_empty() && sections.is_empty() {
            return Err(crate::error::Error::Output(
                "NCX entries require at least one XHTML section".to_owned(),
            ));
        }
        let hierarchical = nodes.iter().any(|node| node.node.depth > 0);
        let tag_definitions = ncx_tag_definitions(hierarchical);
        let labels = nodes
            .iter()
            .map(|node| plain_display_text(&node.node.label))
            .collect::<Vec<_>>();
        let (ctoc_offsets, ctoc) = encode_ctoc_entries(labels.iter().map(String::as_bytes))?;
        let mut entries = Vec::with_capacity(nodes.len());
        for (index, (node, ctoc_offset)) in nodes.iter().zip(ctoc_offsets).enumerate() {
            let mut tags = vec![
                (1, vec![node.start]),
                (2, vec![node.end.saturating_sub(node.start)]),
                (3, vec![ctoc_offset]),
                (
                    4,
                    vec![u32::try_from(node.node.depth).map_err(|_| {
                        crate::error::Error::Output("NCX depth exceeds u32".to_owned())
                    })?],
                ),
            ];
            // Parent/child tags refer to final binary NCX record indexes, not
            // semantic navigation IDs or pre-sort positions.
            if hierarchical {
                if let Some(parent) = node.node.parent {
                    tags.push((
                        21,
                        vec![u32::try_from(parent).map_err(|_| {
                            crate::error::Error::Output("NCX parent index exceeds u32".to_owned())
                        })?],
                    ));
                }
                if let Some(first_child) = node.node.first_child {
                    tags.push((
                        22,
                        vec![u32::try_from(first_child).map_err(|_| {
                            crate::error::Error::Output("NCX child index exceeds u32".to_owned())
                        })?],
                    ));
                }
                if let Some(last_child) = node.node.last_child {
                    tags.push((
                        23,
                        vec![u32::try_from(last_child).map_err(|_| {
                            crate::error::Error::Output("NCX child index exceeds u32".to_owned())
                        })?],
                    ));
                }
            }
            tags.push((
                6,
                vec![node.resolved.sequence_number, node.resolved.payload_offset],
            ));
            entries.push(RawIndexEntry {
                text: index.to_string().into_bytes(),
                tags,
            });
        }
        let ctoc_count = u32::try_from(ctoc.len())
            .map_err(|_| crate::error::Error::Output("NCX CTOC count exceeds u32".to_owned()))?;
        let (main, details) = encode_index_pair(&entries, &tag_definitions, ctoc_count)?;
        Ok((main, details, ctoc))
    }

    fn position_nodes(
        &self,
        position_map: &PositionMap,
        sections: &[KindleSection],
        navigation: Option<&[KindleNavigationItem]>,
    ) -> Result<Vec<OrderedMapNode>> {
        let semantic_nodes = semantic_ncx_nodes(&self.items, navigation);
        let nodes = semantic_nodes
            .into_iter()
            .enumerate()
            .map(|(original_index, node)| {
                let resolved = position_map.resolve(&node.href)?;
                let end = position_map
                    .section_end(resolved.section_index)
                    .ok_or_else(|| {
                        crate::error::Error::Output("NCX section has no range".to_owned())
                    })?;
                Ok(OrderedMapNode {
                    original_index,
                    node,
                    start: resolved.rendered_offset.min(end),
                    end,
                    resolved,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // Preserve the source navigation traversal order.  A breadth/depth
        // sort changes the semantic order of a nested TOC (parent, sibling,
        // child) even though the hierarchy fields still look plausible.
        // The source-order vector is already the order used by TBS and the
        // source navigation tree, so only remap the hierarchy indexes below.
        let order = (0..nodes.len()).collect::<Vec<_>>();
        let mut final_index = vec![0usize; nodes.len()];
        // Establish Kindle's final binary order before remapping semantic
        // parent/first_child/last_child references to final record indexes.
        for (index, &old_index) in order.iter().enumerate() {
            final_index[old_index] = index;
        }
        let mut ordered = order
            .into_iter()
            .map(|old_index| {
                let mut node = nodes[old_index].clone();
                node.node.parent = node.node.parent.map(|parent| final_index[parent]);
                node.node.first_child = node.node.first_child.map(|child| final_index[child]);
                node.node.last_child = node.node.last_child.map(|child| final_index[child]);
                node
            })
            .collect::<Vec<_>>();
        apply_ncx_ranges(&mut ordered, position_map.document_end());
        let _ = sections;
        Ok(ordered)
    }
}

#[derive(Debug, Clone)]
struct OrderedMapNode {
    original_index: usize,
    node: NcxNode,
    start: u32,
    end: u32,
    resolved: ResolvedPosition,
}

fn apply_ncx_ranges(nodes: &mut [OrderedMapNode], logical_end: u32) {
    for index in 0..nodes.len() {
        let boundary = ((index + 1)..nodes.len())
            .find(|&next| nodes[next].node.depth <= nodes[index].node.depth)
            .map(|next| nodes[next].start)
            .unwrap_or(logical_end);
        nodes[index].end = boundary.max(nodes[index].start);
    }
}

fn ncx_tag_definitions(hierarchical: bool) -> Vec<TagDefinition> {
    let _ = hierarchical;
    // 21/22/23 are part of the NCX schema even for a flat navigation tree.
    // Flat rows simply omit those values; encode_detail then leaves their
    // control bits clear while retaining the definitions in TAGX.
    vec![
        (1, 1, 0x01),
        (2, 1, 0x02),
        (3, 1, 0x04),
        (4, 1, 0x08),
        (21, 1, 0x10),
        (22, 1, 0x20),
        (23, 1, 0x40),
        (6, 2, 0x80),
    ]
}

fn semantic_ncx_nodes(
    items: &[(String, String)],
    navigation: Option<&[KindleNavigationItem]>,
) -> Vec<NcxNode> {
    if let Some(navigation) = navigation.filter(|items| !items.is_empty()) {
        let flattened = flatten_ncx_nodes(navigation);
        if flattened.len() == items.len()
            && flattened
                .iter()
                .zip(items)
                .all(|(node, (label, href))| node.label == *label && node.href == *href)
        {
            return flattened;
        }
    }
    items
        .iter()
        .map(|(label, href)| NcxNode {
            label: label.clone(),
            href: href.clone(),
            depth: 0,
            parent: None,
            first_child: None,
            last_child: None,
        })
        .collect()
}

fn flatten_ncx_nodes(items: &[KindleNavigationItem]) -> Vec<NcxNode> {
    fn visit(
        items: &[KindleNavigationItem],
        depth: usize,
        parent: Option<usize>,
        output: &mut Vec<NcxNode>,
    ) {
        for item in items {
            if item.href.is_empty() {
                // An unlinked EPUB navigation heading remains visible in the
                // synthetic TOC, but it has no KF8 position target. Keep its
                // descendants without inventing a link for the heading.
                visit(&item.children, depth, parent, output);
                continue;
            }
            let index = output.len();
            output.push(NcxNode {
                label: item.label.clone(),
                href: item.href.clone(),
                depth,
                parent,
                first_child: None,
                last_child: None,
            });
            let first_child = output.len();
            visit(&item.children, depth + 1, Some(index), output);
            let last_child = output.len().checked_sub(1);
            if first_child < output.len() {
                output[index].first_child = Some(first_child);
                output[index].last_child = last_child;
            }
        }
    }

    let mut output = Vec::new();
    visit(items, 0, None, &mut output);
    output
}

fn flatten(items: &[KindleNavigationItem], output: &mut Vec<(String, String)>) {
    for item in items {
        if !item.href.is_empty() {
            output.push((plain_display_text(&item.label), item.href.clone()));
        }
        flatten(&item.children, output);
    }
}
