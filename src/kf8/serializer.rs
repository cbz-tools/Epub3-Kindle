use std::io::Write;

use super::builder::Kf8Record;
use super::mobi_header::MobiHeader;
use super::palmdoc::PalmDocHeader;
use crate::error::Result;

// Observed KindleGen behavior: trailing Record 0 padding is part of the
// accepted header geometry. It is not content or a general KF8 requirement;
// this self-writer policy preserves the audited container shape without
// relying on firmware-specific handling of a shorter header record.
const RECORD_ZERO_REFERENCE_PADDING: usize = 8192;

/// Serialize a fully built KF8 book into its PalmDB/AZW3 container.
pub(crate) fn serialize(book: super::Kf8Book) -> Result<Vec<u8>> {
    let (record_zero, records) = serialized_records(book)?;
    crate::container::encode_kf8_records(
        std::iter::once(record_zero).chain(records.into_iter().map(|record| record.data)),
    )
}

impl super::Kf8Book {
    pub(crate) fn serialize_to_writer<W: Write>(self, writer: &mut W, path: &str) -> Result<()> {
        let (record_zero, records) = serialized_records(self)?;
        let mut record_lengths = Vec::with_capacity(records.len() + 1);
        record_lengths.push(record_zero.len());
        record_lengths.extend(records.iter().map(|record| record.data.len()));
        crate::container::write_kf8_records(
            record_zero,
            &record_lengths,
            records.into_iter().map(|record| record.data),
            writer,
            path,
        )
    }
}

fn serialized_records(book: super::Kf8Book) -> Result<(Vec<u8>, Vec<Kf8Record>)> {
    let super::Kf8Book {
        palm_doc,
        mut mobi,
        exth,
        title,
        resource_record_count: _,
        resc_record,
        records,
    } = book;
    let exth = exth.encode_checked()?;
    let title = title.as_deref().map(str::as_bytes).unwrap_or_default();
    let header = encode_record_zero(&palm_doc, &mut mobi, &exth, title)?;
    let record_count = records.len() + 1;
    validate_kf8_layout(&palm_doc, &mobi, resc_record, record_count, &records)?;
    Ok((header, records))
}

pub(crate) fn validate_kf8_layout(
    palm_doc: &PalmDocHeader,
    mobi: &MobiHeader,
    resc_record: u32,
    record_count: usize,
    records: &[Kf8Record],
) -> Result<()> {
    palm_doc.validate()?;
    let text_record_count = palm_doc.record_count as usize;
    let expected_first_non_text = text_record_count
        .checked_add(2)
        .ok_or_else(|| crate::error::Error::Output("KF8 record boundary overflow".to_owned()))?;
    let bridge_is_valid = records
        .get(text_record_count)
        .is_some_and(|record| record.data.as_slice() == [0, 0]);
    if text_record_count >= records.len()
        || mobi.first_non_text_record as usize != expected_first_non_text
        || !bridge_is_valid
    {
        return Err(crate::error::Error::Output(
            "PalmDOC record count does not match KF8 text record boundary".to_owned(),
        ));
    }
    mobi.validate_for_record_count(record_count)?;
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
    Ok(())
}

pub(crate) fn validate_kf8_section_pointers<F>(
    mobi: &MobiHeader,
    section_record_count: usize,
    resc_record: u32,
    starts_with: F,
) -> Result<()>
where
    F: Fn(usize, &[u8]) -> bool,
{
    if section_record_count == 0 {
        return Err(crate::error::Error::Output(
            "KF8 section record count cannot be zero".to_owned(),
        ));
    }
    let pointers = [
        (
            "first_non_text",
            mobi.first_non_text_record,
            Some(b"INDX".as_slice()),
        ),
        (
            "first_image",
            mobi.first_image_index,
            Some(b"FDST".as_slice()),
        ),
        ("fdst", mobi.fdst_record, Some(b"FDST".as_slice())),
        ("fcis", mobi.fcis_record, Some(b"FCIS".as_slice())),
        ("flis", mobi.flis_record, Some(b"FLIS".as_slice())),
        ("index", mobi.index_record, Some(b"INDX".as_slice())),
        ("ncx", mobi.ncx_record, Some(b"INDX".as_slice())),
        ("skel", mobi.skel_record, Some(b"INDX".as_slice())),
        ("guide", mobi.guide_record, Some(b"INDX".as_slice())),
    ];
    for (name, pointer, magic) in pointers {
        if pointer == u32::MAX {
            continue;
        }
        if pointer as usize >= section_record_count {
            return Err(crate::error::Error::Output(format!(
                "KF8 {name} pointer {pointer} is outside {section_record_count} relative records"
            )));
        }
        if let Some(magic) = magic {
            if pointer == 0 || !starts_with(pointer as usize - 1, magic) {
                return Err(crate::error::Error::Output(format!(
                    "KF8 {name} pointer {pointer} does not identify {}",
                    String::from_utf8_lossy(magic)
                )));
            }
        }
    }
    if resc_record == 0 || resc_record as usize >= section_record_count {
        return Err(crate::error::Error::Output(format!(
            "KF8 RESC pointer {resc_record} is outside {section_record_count} relative records"
        )));
    }
    if !starts_with(resc_record as usize - 1, b"RESC") {
        return Err(crate::error::Error::Output(format!(
            "KF8 RESC pointer {resc_record} does not identify RESC"
        )));
    }
    Ok(())
}

pub(crate) fn encode_record_zero(
    palm_doc: &PalmDocHeader,
    mobi: &mut MobiHeader,
    exth: &[u8],
    title: &[u8],
) -> Result<Vec<u8>> {
    let exth_padded = align4(exth.len())?;
    let palm_doc_bytes = palm_doc.encode();
    let mut mobi_bytes = mobi.encode();
    let mut header_len = palm_doc_bytes
        .len()
        .checked_add(mobi_bytes.len())
        .and_then(|length| length.checked_add(exth_padded))
        .ok_or_else(|| crate::error::Error::Output("record 0 header length overflow".to_owned()))?;
    if !title.is_empty() {
        mobi.title_offset = u32::try_from(header_len)
            .map_err(|_| crate::error::Error::Output("title offset exceeds u32".to_owned()))?;
        mobi.title_length = u32::try_from(title.len())
            .map_err(|_| crate::error::Error::Output("title length exceeds u32".to_owned()))?;
        header_len = header_len
            .checked_add(align4(title.len().checked_add(2).ok_or_else(|| {
                crate::error::Error::Output("title length overflow".to_owned())
            })?)?)
            .ok_or_else(|| {
                crate::error::Error::Output("record 0 title region overflow".to_owned())
            })?;
        mobi_bytes = mobi.encode();
    }
    let total_len = header_len
        .checked_add(RECORD_ZERO_REFERENCE_PADDING)
        .ok_or_else(|| crate::error::Error::Output("record 0 padding overflow".to_owned()))?;
    let mut bytes = Vec::with_capacity(total_len);
    bytes.extend_from_slice(&palm_doc_bytes);
    bytes.extend_from_slice(&mobi_bytes);
    bytes.extend_from_slice(exth);
    bytes.resize(bytes.len() + (exth_padded - exth.len()), 0);
    if !title.is_empty() {
        bytes.extend_from_slice(title);
        bytes.extend_from_slice(&[0, 0]);
        bytes.resize(header_len, 0);
    }
    bytes.resize(total_len, 0);
    Ok(bytes)
}

fn align4(length: usize) -> Result<usize> {
    length
        .checked_add(3)
        .map(|value| value & !3)
        .ok_or_else(|| crate::error::Error::Output("4-byte alignment overflow".to_owned()))
}
