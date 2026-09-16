use crate::error::Result;
use crate::kf7::{header, section, text};
use crate::kf8::{Kf8Book, Kf8Record, MobiHeader, PalmDocHeader};

use super::resource;

const DATP_STUB: &[u8] = &[
    b'D', b'A', b'T', b'P', 0x00, 0x00, 0x00, 0x0D, 0x01, 0x04, 0x00, 0x04, 0x02, 0x00, 0x00, 0x06,
    0x19, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x6D, 0x02, 0x46, 0x02, 0x66, 0x00, 0x00, 0x00,
];

#[derive(Clone, Copy)]
struct RelativeRecord {
    source_index: usize,
    is_datp_stub: bool,
}

fn build_relative_record_mapping(
    records: &[Kf8Record],
    resource_start: usize,
    resource_end: usize,
) -> Vec<RelativeRecord> {
    let mut mapping = Vec::new();
    for (source_index, record) in records.iter().enumerate() {
        if (resource_start..resource_end).contains(&source_index) {
            continue;
        }
        if record.data.as_slice() == text::EOF {
            mapping.push(RelativeRecord {
                source_index,
                is_datp_stub: true,
            });
        }
        mapping.push(RelativeRecord {
            source_index,
            is_datp_stub: false,
        });
    }
    mapping
}

/// Return the legacy RawML Start Reading coordinate for the constructed stub.
/// This follows the KF8 PositionMap convention: the target is the opening
/// `<body` tag, measured from the beginning of the document.
fn legacy_start_reading_offset(stub: &[u8]) -> Result<u32> {
    let body_start = stub
        .windows(b"<body".len())
        .position(|window| window.eq_ignore_ascii_case(b"<body"))
        .filter(|&start| {
            stub.get(start + b"<body".len())
                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>')
        })
        .ok_or_else(|| {
            crate::error::Error::Output(
                "legacy stub is missing a usable body start for EXTH 116".to_owned(),
            )
        })?;
    u32::try_from(body_start).map_err(|_| {
        crate::error::Error::Output("legacy start-reading offset exceeds u32".to_owned())
    })
}

pub(crate) struct DualLayout {
    palmdb_name: String,
    legacy_record_zero: Vec<u8>,
    legacy_stub: Vec<u8>,
    flis: Vec<u8>,
    fcis: Vec<u8>,
    boundary: Vec<u8>,
    kf8_record_zero: Vec<u8>,
    kf8_book: Kf8Book,
    resource_start: usize,
    resource_end: usize,
}

impl DualLayout {
    pub(crate) fn record_lengths(&self) -> Result<Vec<usize>> {
        let mut lengths = Vec::new();
        lengths.push(self.legacy_record_zero.len());
        lengths.push(self.legacy_stub.len());
        for record in self
            .kf8_book
            .records
            .iter()
            .skip(self.resource_start)
            .take(self.resource_end - self.resource_start)
        {
            lengths.push(resource::serialized_shared_resource_length(&record.data)?);
        }
        lengths.push(self.flis.len());
        lengths.push(self.fcis.len());
        lengths.push(self.boundary.len());
        lengths.push(self.kf8_record_zero.len());
        for (index, record) in self.kf8_book.records.iter().enumerate() {
            if (self.resource_start..self.resource_end).contains(&index) {
                continue;
            }
            if record.data.as_slice() == text::EOF {
                lengths.push(DATP_STUB.len());
            }
            lengths.push(record.data.len());
        }
        Ok(lengths)
    }

    pub(crate) fn palmdb_name(&self) -> &str {
        &self.palmdb_name
    }

    pub(crate) fn into_records(self) -> DualRecordStream {
        DualRecordStream {
            legacy_record_zero: Some(self.legacy_record_zero),
            legacy_stub: Some(self.legacy_stub),
            flis: Some(self.flis),
            fcis: Some(self.fcis),
            boundary: Some(self.boundary),
            kf8_record_zero: Some(self.kf8_record_zero),
            kf8_book: self.kf8_book,
            resource_start: self.resource_start,
            resource_end: self.resource_end,
            shared_resource_index: self.resource_start,
            kf8_source_index: 0,
            pending_kf8_record: None,
            phase: DualRecordStreamPhase::LegacyRecordZero,
        }
    }
}

#[derive(Clone, Copy)]
enum DualRecordStreamPhase {
    LegacyRecordZero,
    LegacyStub,
    SharedResources,
    Flis,
    Fcis,
    Boundary,
    Kf8RecordZero,
    Kf8Sources,
    Done,
}

pub(crate) struct DualRecordStream {
    legacy_record_zero: Option<Vec<u8>>,
    legacy_stub: Option<Vec<u8>>,
    flis: Option<Vec<u8>>,
    fcis: Option<Vec<u8>>,
    boundary: Option<Vec<u8>>,
    kf8_record_zero: Option<Vec<u8>>,
    kf8_book: Kf8Book,
    resource_start: usize,
    resource_end: usize,
    shared_resource_index: usize,
    kf8_source_index: usize,
    pending_kf8_record: Option<Vec<u8>>,
    phase: DualRecordStreamPhase,
}

impl Iterator for DualRecordStream {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.phase {
                DualRecordStreamPhase::LegacyRecordZero => {
                    self.phase = DualRecordStreamPhase::LegacyStub;
                    return Some(Ok(self.legacy_record_zero.take()?));
                }
                DualRecordStreamPhase::LegacyStub => {
                    self.phase = DualRecordStreamPhase::SharedResources;
                    return Some(Ok(self.legacy_stub.take()?));
                }
                DualRecordStreamPhase::SharedResources => {
                    if self.shared_resource_index < self.resource_end {
                        let index = self.shared_resource_index;
                        self.shared_resource_index += 1;
                        let data = std::mem::take(&mut self.kf8_book.records[index].data);
                        return Some(resource::serialize_shared_resource(data));
                    }
                    self.phase = DualRecordStreamPhase::Flis;
                }
                DualRecordStreamPhase::Flis => {
                    self.phase = DualRecordStreamPhase::Fcis;
                    return Some(Ok(self.flis.take()?));
                }
                DualRecordStreamPhase::Fcis => {
                    self.phase = DualRecordStreamPhase::Boundary;
                    return Some(Ok(self.fcis.take()?));
                }
                DualRecordStreamPhase::Boundary => {
                    self.phase = DualRecordStreamPhase::Kf8RecordZero;
                    return Some(Ok(self.boundary.take()?));
                }
                DualRecordStreamPhase::Kf8RecordZero => {
                    self.phase = DualRecordStreamPhase::Kf8Sources;
                    return Some(Ok(self.kf8_record_zero.take()?));
                }
                DualRecordStreamPhase::Kf8Sources => {
                    if let Some(data) = self.pending_kf8_record.take() {
                        return Some(Ok(data));
                    }
                    while self.kf8_source_index < self.kf8_book.records.len() {
                        let index = self.kf8_source_index;
                        self.kf8_source_index += 1;
                        if (self.resource_start..self.resource_end).contains(&index) {
                            continue;
                        }
                        let data = std::mem::take(&mut self.kf8_book.records[index].data);
                        if data.as_slice() == text::EOF {
                            self.pending_kf8_record = Some(data);
                            return Some(Ok(DATP_STUB.to_vec()));
                        }
                        return Some(Ok(data));
                    }
                    self.phase = DualRecordStreamPhase::Done;
                }
                DualRecordStreamPhase::Done => return None,
            }
        }
    }
}

pub(crate) fn build_dual_layout(book: Kf8Book) -> Result<DualLayout> {
    let Kf8Book {
        palm_doc,
        mut mobi,
        exth,
        title: title_text,
        resource_record_count,
        resc_record,
        records,
    } = book;
    let record_count = records.len() + 1;
    let mut kf8_exth = exth.clone();
    for record in &mut kf8_exth.records {
        if record.kind == 125 {
            record.value = 0u32.to_be_bytes().to_vec();
        }
    }
    let kf8_exth_bytes = kf8_exth.encode_checked()?;
    let title = title_text.as_deref().map(str::as_bytes).unwrap_or_default();
    let palmdb_name = title_text
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("kindle-format")
        .to_owned();
    let legacy_uid = compatibility_uid(title, &kf8_exth_bytes);
    crate::kf8::validate_kf8_layout(&palm_doc, &mobi, resc_record, record_count, &records)?;
    let resource_count = usize::try_from(resource_record_count)
        .map_err(|_| crate::error::Error::Output("resource count exceeds usize".to_owned()))?;
    let old_resource_start = resc_record
        .checked_sub(resource_record_count)
        .ok_or_else(|| {
            crate::error::Error::Output("resource record geometry underflows".to_owned())
        })?;
    if old_resource_start == 0
        || old_resource_start as usize > records.len()
        || resource_count > records.len()
        || old_resource_start as usize - 1 + resource_count != resc_record as usize - 1
    {
        return Err(crate::error::Error::Output(
            "resource record geometry does not place resources immediately before RESC".to_owned(),
        ));
    }
    let resc_index = resc_record
        .checked_sub(1)
        .ok_or_else(|| crate::error::Error::Output("RESC record index is zero".to_owned()))?
        as usize;
    if records
        .get(resc_index)
        .is_none_or(|record| !record.data.starts_with(b"RESC"))
    {
        return Err(crate::error::Error::Output(
            "RESC record index does not identify a RESC record".to_owned(),
        ));
    }

    let legacy_first_non_text: u32 = 2;
    let shared_resource_start = legacy_first_non_text;
    let legacy_flis_record = shared_resource_start
        .checked_add(resource_record_count)
        .ok_or_else(|| {
            crate::error::Error::Output("legacy FLIS record index overflow".to_owned())
        })?;
    let legacy_fcis_record = legacy_flis_record.checked_add(1).ok_or_else(|| {
        crate::error::Error::Output("legacy FCIS record index overflow".to_owned())
    })?;
    let boundary_record = legacy_fcis_record
        .checked_add(1)
        .ok_or_else(|| crate::error::Error::Output("BOUNDARY record index overflow".to_owned()))?;
    let kf8_record_zero_index = boundary_record
        .checked_add(1)
        .ok_or_else(|| crate::error::Error::Output("KF8 record 0 index overflow".to_owned()))?;

    let legacy_stub = text::STUB_HTML.to_vec();
    let legacy_start = legacy_start_reading_offset(&legacy_stub)?;
    let legacy_text_length = u32::try_from(legacy_stub.len())
        .map_err(|_| crate::error::Error::Output("legacy stub length exceeds u32".to_owned()))?;
    let legacy_text_record_count = 1u16;
    let legacy_first_content_record = 1u16;
    let current_resource_end = if resource_record_count != 0 {
        Some(
            shared_resource_start
                .checked_add(resource_record_count)
                .and_then(|end| end.checked_sub(1))
                .ok_or_else(|| {
                    crate::error::Error::Output("legacy resource record range overflows".to_owned())
                })?,
        )
    } else {
        None
    };
    let legacy_last_content_record = legacy_first_content_record
        .checked_add(legacy_text_record_count.checked_sub(1).ok_or_else(|| {
            crate::error::Error::Output("legacy text record count is zero".to_owned())
        })?)
        .ok_or_else(|| {
            crate::error::Error::Output("legacy content record range overflows u16".to_owned())
        })?;
    let legacy_content_record_range = if let Some(resource_end) = current_resource_end {
        Some((
            legacy_first_content_record,
            u16::try_from(resource_end).map_err(|_| {
                crate::error::Error::Output("legacy content record range exceeds u16".to_owned())
            })?,
        ))
    } else {
        Some((legacy_first_content_record, legacy_last_content_record))
    };
    let mut legacy_exth = exth.clone();
    let legacy_start_reading_bytes = legacy_start.to_be_bytes();
    let mut has_legacy_start_reading = false;
    for record in &mut legacy_exth.records {
        if record.kind == 116 {
            record.value = legacy_start_reading_bytes.to_vec();
            has_legacy_start_reading = true;
        }
    }
    if !has_legacy_start_reading {
        legacy_exth.push_bytes(116, legacy_start_reading_bytes);
    }
    legacy_exth.push_bytes(121, kf8_record_zero_index.to_be_bytes());
    let kf8_record_zero_bytes = kf8_record_zero_index.to_be_bytes();
    if !legacy_exth
        .records
        .iter()
        .any(|record| record.kind == 121 && record.value.as_slice() == kf8_record_zero_bytes)
    {
        return Err(crate::error::Error::Output(
            "legacy EXTH 121 does not identify KF8 record 0".to_owned(),
        ));
    }
    let first_image = if current_resource_end.is_some() {
        shared_resource_start
    } else {
        u32::MAX
    };
    let last_image = if mobi.last_image_index == u16::MAX {
        u16::MAX
    } else {
        u16::try_from(rebase_legacy_pointer(
            u32::from(mobi.last_image_index),
            old_resource_start,
            resource_count,
            shared_resource_start,
            kf8_record_zero_index,
        )?)
        .map_err(|_| {
            crate::error::Error::Output("last image record index exceeds u16".to_owned())
        })?
    };
    let mut legacy_mobi = header::new(header::LegacyHeaderFields {
        first_non_text_record: legacy_first_non_text,
        first_resource_record: first_image,
        fcis_record: legacy_fcis_record,
        flis_record: legacy_flis_record,
        last_image_index: last_image,
        content_record_range: legacy_content_record_range,
        extra_data_flags: 0x0003,
        language: mobi.language,
        uid: legacy_uid,
        ncx_record: u32::MAX,
        exth_flags: 0x0850,
    });
    let mut legacy_stub_bytes = text::compress_stub(&legacy_stub);
    let compressed_length = legacy_stub_bytes.len();
    legacy_stub_bytes.extend_from_slice(&[0x00, 0x81]);
    if legacy_stub_bytes.len() != compressed_length + 2
        || !legacy_stub_bytes.ends_with(&[0x00, 0x81])
    {
        return Err(crate::error::Error::Output(
            "legacy stub trailing bytes are not 00 81".to_owned(),
        ));
    }
    let decoded_stub = text::decode_palm_doc(&legacy_stub_bytes[..compressed_length])?;
    if decoded_stub != text::STUB_HTML {
        return Err(crate::error::Error::Output(
            "decompressed legacy stub does not match the exact HTML contract".to_owned(),
        ));
    }
    if legacy_mobi.version != 6
        || legacy_mobi.min_version != 6
        || mobi.version != 8
        || mobi.min_version != 8
    {
        return Err(crate::error::Error::Output(
            "dual MOBI headers must be KF7 v6 and KF8 v8".to_owned(),
        ));
    }
    let legacy_record_zero = crate::kf8::encode_record_zero(
        &PalmDocHeader {
            compression: 2,
            text_length: legacy_text_length,
            record_count: legacy_text_record_count,
            record_size: 4096,
            encryption: 0,
        },
        &mut legacy_mobi,
        &legacy_exth.encode_checked()?,
        title,
    )?;

    rebase_kf8_header(&mut mobi, old_resource_start, resource_count)?;
    let kf8_record_zero =
        crate::kf8::encode_record_zero(&palm_doc, &mut mobi, &kf8_exth_bytes, title)?;
    let kf8_source_record_count = records
        .len()
        .checked_sub(resource_count)
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| {
            crate::error::Error::Output("KF8 source record capacity underflows".to_owned())
        })?;
    let resource_start = old_resource_start as usize - 1;
    let resource_end = resc_record as usize - 1;
    if resource_end - resource_start != resource_count {
        return Err(crate::error::Error::Output(
            "resource record count does not match the shared pool".to_owned(),
        ));
    }
    if !records.iter().enumerate().any(|(index, record)| {
        !(resource_start..resource_end).contains(&index) && record.data.as_slice() == text::EOF
    }) {
        return Err(crate::error::Error::Output(
            "KF8 EOF record is missing from the section".to_owned(),
        ));
    }

    let kf8_resc_record =
        resc_record
            .checked_sub(u32::try_from(resource_count).map_err(|_| {
                crate::error::Error::Output("resource count exceeds u32".to_owned())
            })?)
            .ok_or_else(|| {
                crate::error::Error::Output("KF8 RESC record geometry underflows".to_owned())
            })?;
    let kf8_section_record_count = kf8_source_record_count.checked_add(1).ok_or_else(|| {
        crate::error::Error::Output("KF8 section record count overflow".to_owned())
    })?;
    let relative_records = build_relative_record_mapping(&records, resource_start, resource_end);
    crate::kf8::validate_kf8_section_pointers(
        &mobi,
        kf8_section_record_count,
        kf8_resc_record,
        |index, prefix| {
            let Some(relative) = relative_records.get(index) else {
                return false;
            };
            if relative.is_datp_stub {
                DATP_STUB.starts_with(prefix)
            } else {
                records[relative.source_index].data.starts_with(prefix)
            }
        },
    )?;
    if u32::try_from(2usize + resource_count + 2)
        .ok()
        .is_none_or(|record| record != boundary_record)
    {
        return Err(crate::error::Error::Output(
            "BOUNDARY record index does not match dual layout".to_owned(),
        ));
    }
    if u32::try_from(2usize + resource_count + 3)
        .ok()
        .is_none_or(|record| record != kf8_record_zero_index)
    {
        return Err(crate::error::Error::Output(
            "KF8 record 0 index does not match dual layout".to_owned(),
        ));
    }
    if kf8_record_zero.get(16..20) != Some(b"MOBI") {
        return Err(crate::error::Error::Output(
            "BOUNDARY must immediately precede KF8 record 0".to_owned(),
        ));
    }
    let final_record_count = 1usize
        .checked_add(1)
        .and_then(|count| count.checked_add(resource_count))
        .and_then(|count| count.checked_add(4))
        .and_then(|count| count.checked_add(kf8_source_record_count))
        .ok_or_else(|| crate::error::Error::Output("dual record count overflow".to_owned()))?;
    legacy_mobi.validate_for_record_count(final_record_count)?;
    mobi.validate_for_record_count(final_record_count)?;
    let kf8_book = Kf8Book {
        palm_doc,
        mobi,
        exth,
        title: title_text,
        resource_record_count,
        resc_record,
        records,
    };
    Ok(DualLayout {
        palmdb_name,
        legacy_record_zero,
        legacy_stub: legacy_stub_bytes,
        flis: crate::kf8::encode_flis(),
        fcis: section::encode_fcis(legacy_text_length),
        boundary: b"BOUNDARY".to_vec(),
        kf8_record_zero,
        kf8_book,
        resource_start,
        resource_end,
    })
}

fn compatibility_uid(title: &[u8], serialized_metadata: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for byte in title.iter().chain(serialized_metadata) {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    if hash == 0 || hash == 1 { 2 } else { hash }
}

fn rebase_kf8_header(
    mobi: &mut MobiHeader,
    old_resource_start: u32,
    resource_count: usize,
) -> Result<()> {
    let fdst_record = rebase_pointer(mobi.fdst_record, old_resource_start, resource_count)?;
    for pointer in [
        &mut mobi.first_non_text_record,
        &mut mobi.fcis_record,
        &mut mobi.flis_record,
        &mut mobi.index_record,
        &mut mobi.ncx_record,
        &mut mobi.skel_record,
        &mut mobi.guide_record,
    ] {
        *pointer = rebase_pointer(*pointer, old_resource_start, resource_count)?;
    }
    mobi.fdst_record = fdst_record;
    mobi.first_resource_record = fdst_record;
    mobi.first_image_index = fdst_record;
    mobi.last_image_index = u16::MAX;
    Ok(())
}

fn rebase_pointer(pointer: u32, old_resource_start: u32, resource_count: usize) -> Result<u32> {
    if pointer == u32::MAX {
        return Ok(pointer);
    }
    let resource_count_u32 = u32::try_from(resource_count)
        .map_err(|_| crate::error::Error::Output("resource count exceeds u32".to_owned()))?;
    let resource_end = old_resource_start
        .checked_add(resource_count_u32)
        .ok_or_else(|| crate::error::Error::Output("resource record range overflow".to_owned()))?;
    if pointer < old_resource_start {
        Ok(pointer)
    } else if pointer < resource_end {
        Err(crate::error::Error::Output(
            "KF8 pointer targets the shared resource block".to_owned(),
        ))
    } else {
        pointer
            .checked_sub(resource_count_u32)
            .ok_or_else(|| crate::error::Error::Output("KF8 record pointer overflow".to_owned()))
    }
}

fn rebase_legacy_pointer(
    pointer: u32,
    old_resource_start: u32,
    resource_count: usize,
    shared_resource_start: u32,
    kf8_record_zero: u32,
) -> Result<u32> {
    if pointer == u32::MAX {
        return Ok(pointer);
    }
    let resource_count_u32 = u32::try_from(resource_count)
        .map_err(|_| crate::error::Error::Output("resource count exceeds u32".to_owned()))?;
    let resource_end = old_resource_start
        .checked_add(resource_count_u32)
        .ok_or_else(|| crate::error::Error::Output("resource record range overflow".to_owned()))?;
    if pointer < old_resource_start {
        pointer
            .checked_add(kf8_record_zero)
            .ok_or_else(|| crate::error::Error::Output("KF7 record pointer overflow".to_owned()))
    } else if pointer < resource_end {
        shared_resource_start
            .checked_add(pointer - old_resource_start)
            .ok_or_else(|| {
                crate::error::Error::Output("shared resource pointer overflow".to_owned())
            })
    } else {
        pointer
            .checked_add(kf8_record_zero)
            .and_then(|value| value.checked_sub(resource_count_u32))
            .ok_or_else(|| crate::error::Error::Output("KF7 record pointer overflow".to_owned()))
    }
}
