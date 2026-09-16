//! Calculate and encode text-record trailing bytes used by KF8 indexes.
//!
//! The pipeline maps navigation semantic entries onto text-record intersections,
//! classifies each local entry as an [`EntryAction`], and groups those entries
//! into strand/layer intermediate data. The groups become TBS sequences, which
//! are encoded as VWI values in each record's trailing data. The intermediate
//! form preserves hierarchy and cross-record spans before byte encoding.

use super::indx::encode_vwi;
use crate::error::Result;
use crate::kindle::KindleNavigationItem;
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TbsEntry {
    pub(super) index: usize,
    pub(super) start: usize,
    pub(super) length: usize,
    pub(super) depth: usize,
    pub(super) parent: Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct TbsSeed {
    pub(super) entry: TbsEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryAction {
    Spans,
    Ends,
    Starts,
    Completes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalTbsEntry {
    pub(super) entry: TbsEntry,
    pub(super) action: EntryAction,
    pub(super) start_offset: isize,
    length_offset: isize,
    pub(super) text_record_length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TbsEncodingError {
    NegativeStrandIndex,
    ValueTooLarge,
    SiblingCountTooLarge,
}

// Classify one navigation span against a single text record. Keeping this
// action explicit preserves the distinction between starts, ends, and spans
// before the compact TBS representation is encoded.
fn fill_entry(entry: &TbsEntry, start_offset: isize, text_record_length: usize) -> LocalTbsEntry {
    let length_offset = start_offset + entry.length as isize;
    let action = if start_offset < 0 {
        if length_offset > text_record_length as isize {
            EntryAction::Spans
        } else {
            EntryAction::Ends
        }
    } else if length_offset > text_record_length as isize {
        EntryAction::Starts
    } else {
        EntryAction::Completes
    };
    LocalTbsEntry {
        entry: entry.clone(),
        action,
        start_offset,
        length_offset,
        text_record_length,
    }
}

// Follow parent/child relationships and contiguous siblings into one strand so
// hierarchy survives the later layer grouping and sequence encoding.
struct EntryPool {
    entries: Vec<Option<LocalTbsEntry>>,
    positions_by_index: HashMap<usize, Vec<usize>>,
    children_by_parent: HashMap<usize, Vec<usize>>,
    next_first: usize,
}

impl EntryPool {
    fn new(entries: Vec<LocalTbsEntry>) -> Self {
        let mut positions_by_index: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut children_by_parent: HashMap<usize, Vec<usize>> = HashMap::new();
        for (position, entry) in entries.iter().enumerate() {
            positions_by_index
                .entry(entry.entry.index)
                .or_default()
                .push(position);
            if let Some(parent) = entry.entry.parent {
                children_by_parent.entry(parent).or_default().push(position);
            }
        }
        Self {
            entries: entries.into_iter().map(Some).collect(),
            positions_by_index,
            children_by_parent,
            next_first: 0,
        }
    }

    fn take_position(&mut self, position: usize) -> Option<LocalTbsEntry> {
        let entry = self.entries.get_mut(position)?.take()?;
        Some(entry)
    }

    fn take_first(&mut self) -> Option<LocalTbsEntry> {
        while self.next_first < self.entries.len() {
            let position = self.next_first;
            self.next_first += 1;
            if self.entries[position].is_some() {
                return self.take_position(position);
            }
        }
        None
    }

    fn take_child(&mut self, parent: usize) -> Option<LocalTbsEntry> {
        let position = self
            .children_by_parent
            .get(&parent)?
            .iter()
            .copied()
            .find(|&position| self.entries[position].is_some())?;
        self.take_position(position)
    }

    fn take_sibling(
        &mut self,
        parent: &LocalTbsEntry,
        current_index: usize,
    ) -> Option<LocalTbsEntry> {
        let position = self
            .positions_by_index
            .get(&(current_index + 1))?
            .iter()
            .copied()
            .find(|&position| {
                self.entries[position].as_ref().is_some_and(|candidate| {
                    candidate.entry.depth == parent.entry.depth
                        && candidate.entry.parent == parent.entry.parent
                })
            })?;
        self.take_position(position)
    }

    fn has_child(&self, parent: usize) -> bool {
        self.children_by_parent
            .get(&parent)
            .is_some_and(|positions| {
                positions
                    .iter()
                    .any(|&position| self.entries[position].is_some())
            })
    }
}

fn populate_strand(parent: LocalTbsEntry, entries: &mut EntryPool) -> Vec<LocalTbsEntry> {
    let mut answer = vec![parent.clone()];
    if let Some(child) = entries.take_child(parent.entry.index) {
        answer.extend(populate_strand(child, entries));
    } else {
        let mut current_index = parent.entry.index;
        let mut siblings = Vec::new();
        while let Some(entry) = entries.take_sibling(&parent, current_index) {
            current_index = entry.entry.index;
            if entries.has_child(entry.entry.index) {
                siblings.extend(populate_strand(entry, entries));
                break;
            }
            siblings.push(entry);
        }
        answer.extend(siblings);
    }
    answer
}

pub(super) type StrandLayers = Vec<(usize, Vec<LocalTbsEntry>)>;

// Split local entries into strands, then group each strand by navigation depth.
fn separate_strands(entries: Vec<LocalTbsEntry>) -> Vec<StrandLayers> {
    let mut entries = EntryPool::new(entries);
    let mut answer = Vec::new();
    while let Some(top) = entries.take_first() {
        let strand = populate_strand(top, &mut entries);
        let mut layers: StrandLayers = Vec::new();
        for entry in strand {
            let layer = layers
                .iter()
                .position(|(depth, _)| *depth == entry.entry.depth);
            if let Some(layer) = layer {
                layers[layer].1.push(entry);
            } else {
                layers.push((entry.entry.depth, vec![entry]));
            }
        }
        answer.push(layers);
    }
    answer
}

pub(super) fn collect_indexing_data(
    entries: &[TbsEntry],
    text_record_lengths: &[usize],
) -> Result<Vec<Vec<StrandLayers>>> {
    // Intersect navigation spans with the fixed text-record boundaries before
    // building strands; TBS records describe these local intersections.
    let mut sorted_entries = entries.to_vec();
    sorted_entries.sort_by_key(|entry| entry.start);
    let mut data = Vec::with_capacity(text_record_lengths.len());
    let mut record_start = 0usize;
    let mut next_entry = 0usize;
    let mut active_positions: BTreeSet<usize> = BTreeSet::new();
    let mut expiry_queue: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::new();
    for &record_length in text_record_lengths {
        let next_record_start = record_start.checked_add(record_length).ok_or_else(|| {
            crate::error::Error::Output("TBS record position overflow".to_owned())
        })?;

        while let Some(&Reverse((entry_end, position))) = expiry_queue.peek() {
            if entry_end > record_start {
                break;
            }
            expiry_queue.pop();
            active_positions.remove(&position);
        }

        let mut local_entries = Vec::new();

        // Entries already reached by the sweep precede all newly reached
        // entries in sorted_entries, so this preserves the old scan order.
        for &position in &active_positions {
            let entry = &sorted_entries[position];
            let start_offset = isize::try_from(entry.start)
                .and_then(|start| isize::try_from(record_start).map(|record| start - record))
                .map_err(|_| {
                    crate::error::Error::Output("TBS position exceeds isize".to_owned())
                })?;
            local_entries.push(fill_entry(entry, start_offset, record_length));
        }

        while next_entry < sorted_entries.len()
            && sorted_entries[next_entry].start < next_record_start
        {
            let position = next_entry;
            let entry = &sorted_entries[position];
            let entry_end = entry.start.checked_add(entry.length).ok_or_else(|| {
                crate::error::Error::Output("TBS entry position overflow".to_owned())
            })?;

            if entry_end > record_start {
                let start_offset = isize::try_from(entry.start)
                    .and_then(|start| isize::try_from(record_start).map(|record| start - record))
                    .map_err(|_| {
                        crate::error::Error::Output("TBS position exceeds isize".to_owned())
                    })?;
                local_entries.push(fill_entry(entry, start_offset, record_length));
                active_positions.insert(position);
                expiry_queue.push(Reverse((entry_end, position)));
            }
            next_entry += 1;
        }

        // The former full scan checked the first entry at or beyond the
        // boundary before breaking; retain that overflow behavior while the
        // sweep advances only through intersecting start positions.
        if next_entry < sorted_entries.len() {
            sorted_entries[next_entry]
                .start
                .checked_add(sorted_entries[next_entry].length)
                .ok_or_else(|| {
                    crate::error::Error::Output("TBS entry position overflow".to_owned())
                })?;
        }
        data.push(separate_strands(local_entries));
        record_start = next_record_start;
    }
    Ok(data)
}

pub(super) fn calculate_all_tbs(
    indexing_data: &[Vec<StrandLayers>],
    tbs_type: u8,
) -> std::result::Result<Vec<Vec<u8>>, TbsEncodingError> {
    // Convert the hierarchy-preserving intermediate form into per-record TBS
    // trailing bytes, leaving all byte-level encoding in this module.
    indexing_data
        .iter()
        .map(|strands| {
            let sequences = encode_strands_as_sequences(strands, tbs_type)?;
            sequences_to_bytes(&sequences)
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct SequenceExtra {
    spans: bool,
    tbs_type: Option<u8>,
    count: Option<usize>,
    forwards: bool,
}

impl SequenceExtra {
    const fn empty() -> Self {
        Self {
            spans: false,
            tbs_type: None,
            count: None,
            forwards: false,
        }
    }

    fn flags(self) -> u8 {
        u8::from(self.spans)
            | (u8::from(self.tbs_type.is_some()) << 1)
            | (u8::from(self.count.is_some()) << 2)
            | (u8::from(self.forwards) << 3)
    }
}

#[derive(Debug, Clone, Copy)]
struct Sequence {
    value: usize,
    extra: SequenceExtra,
}

fn encode_strands_as_sequences(
    strands: &[StrandLayers],
    tbs_type: u8,
) -> std::result::Result<Vec<Sequence>, TbsEncodingError> {
    // Turn strand/layer entries into compact sequences while retaining span,
    // type, sibling-count, and direction flags required by the reader.
    let first_entry = strands
        .iter()
        .flat_map(|strand| strand.iter().flat_map(|(_, entries)| entries))
        .next()
        .map(|entry| entry.entry.index);
    let mut answer = Vec::new();
    let mut last_index = None;
    for strand in strands {
        let mut strand_sequences = Vec::new();
        for entries in strand.iter().map(|(_, entries)| entries) {
            let last = entries.last().expect("TBS layer cannot be empty");
            let first = &entries[0];
            let mut extra = SequenceExtra::empty();
            if last.action == EntryAction::Spans {
                extra.spans = true;
            }
            if first_entry == Some(first.entry.index) {
                extra.tbs_type = Some(tbs_type);
            }
            if entries.len() > 1 {
                extra.count = Some(entries.len());
            }
            let mut index = first.entry.index - first.entry.parent.unwrap_or(0);
            if !answer.is_empty() && strand_sequences.is_empty() {
                let delta = last_index.expect("later strand has a previous index") as isize
                    - first.entry.index as isize;
                if delta < 0 {
                    if tbs_type == 5 {
                        index = delta.unsigned_abs();
                    } else {
                        return Err(TbsEncodingError::NegativeStrandIndex);
                    }
                } else {
                    index = delta as usize;
                    extra.forwards = true;
                }
            }
            last_index = Some(last.entry.index);
            strand_sequences.push(Sequence {
                value: index,
                extra,
            });
        }
        for index in 0..strand_sequences.len().saturating_sub(1) {
            if strand_sequences[index].extra.spans && strand_sequences[index + 1].extra.spans {
                strand_sequences[index].extra.spans = false;
            }
        }
        answer.extend(strand_sequences);
    }
    Ok(answer)
}

fn sequences_to_bytes(sequences: &[Sequence]) -> std::result::Result<Vec<u8>, TbsEncodingError> {
    let mut answer = Vec::new();
    for (index, sequence) in sequences.iter().enumerate() {
        let flag_size = if index == 0 { 3 } else { 4 };
        answer.extend(encode_tbs_sequence(*sequence, flag_size)?);
    }
    Ok(answer)
}

fn encode_tbs_sequence(
    sequence: Sequence,
    flag_size: u32,
) -> std::result::Result<Vec<u8>, TbsEncodingError> {
    let flags = u32::from(sequence.extra.flags());
    let value = u32::try_from(sequence.value)
        .map_err(|_| TbsEncodingError::ValueTooLarge)?
        .checked_shl(flag_size)
        .and_then(|value| value.checked_add(flags))
        .ok_or(TbsEncodingError::ValueTooLarge)?;
    let mut answer = encode_vwi(value);
    if let Some(tbs_type) = sequence.extra.tbs_type {
        answer.extend(encode_vwi(u32::from(tbs_type)));
    }
    if let Some(count) = sequence.extra.count {
        answer.push(u8::try_from(count).map_err(|_| TbsEncodingError::SiblingCountTooLarge)?);
    }
    if sequence.extra.spans {
        answer.extend(encode_vwi(0));
    }
    Ok(answer)
}

pub(super) fn tbs_error(error: TbsEncodingError) -> crate::error::Error {
    let message = match error {
        TbsEncodingError::NegativeStrandIndex => "negative TBS strand index",
        TbsEncodingError::ValueTooLarge => "TBS value exceeds u32",
        TbsEncodingError::SiblingCountTooLarge => "TBS sibling count exceeds one byte",
    };
    crate::error::Error::Output(message.to_owned())
}

pub(super) fn flatten_tbs_seeds(
    items: &[KindleNavigationItem],
    depth: usize,
    parent: Option<usize>,
    output: &mut Vec<TbsSeed>,
) {
    for item in items {
        if item.href.is_empty() {
            // Unlinked EPUB navigation headings are represented in the
            // synthetic TOC only; TBS entries require a real position target.
            flatten_tbs_seeds(&item.children, depth, parent, output);
            continue;
        }
        let index = output.len();
        output.push(TbsSeed {
            entry: TbsEntry {
                index,
                start: 0,
                length: 0,
                depth,
                parent,
            },
        });
        flatten_tbs_seeds(&item.children, depth + 1, Some(index), output);
    }
}
