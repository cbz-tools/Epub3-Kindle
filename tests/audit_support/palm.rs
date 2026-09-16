//! Independent PalmDB/PalmDOC/MOBI/EXTH reader for audit assertions.
//! No production parser, serializer constants, or helper functions are imported.
//! Field offsets follow the external MOBI references listed in CONVERSION_AUDIT.md.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct PalmDb<'a> {
    bytes: &'a [u8],
    pub record_offsets: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalmDocTrailer {
    pub payload_end: usize,
    pub multibyte_overlap: Vec<u8>,
    pub advertised_tbs: Vec<u8>,
    pub trailer_byte_count: usize,
    pub reverse_vwi_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdstRecord {
    pub header_size: usize,
    pub flow_count: usize,
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndxRecord {
    pub idxt_offset: usize,
    pub entry_count: usize,
    pub detail_count: usize,
    pub tagx: Vec<(u8, u8, u8)>,
    pub row_offsets: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndxRow {
    pub text: Vec<u8>,
    pub values: BTreeMap<u8, Vec<u32>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MobiHeader<'a> {
    pub record_index: usize,
    pub record: &'a [u8],
    pub compression: u16,
    pub text_length: usize,
    pub text_record_count: usize,
    pub record_size: usize,
    pub mobi_header_length: usize,
    pub mobi_type: u32,
    pub encoding: u32,
    /// MOBI language/locale field at record offset 0x5c (0x4c from MOBI magic).
    pub locale: u32,
    pub version: u32,
    pub first_resource: u32,
    pub exth_flags: u32,
    pub extra_record_data_flags: u16,
    pub indx_record: Option<u32>,
    pub fdst_record: Option<u32>,
    pub ncx_record: Option<u32>,
    pub fragment_index: Option<u32>,
    pub skeleton_index: Option<u32>,
    pub datp_index: Option<u32>,
    pub guide_index: Option<u32>,
    pub first_non_text: Option<u32>,
    pub fcis_record: Option<u32>,
    pub fcis_count: Option<u32>,
    pub flis_record: Option<u32>,
    pub flis_count: Option<u32>,
    pub fdst_flow_count: Option<u32>,
    pub index_record: Option<u32>,
    pub skeleton_record: Option<u32>,
    pub fragment_record: Option<u32>,
    pub datp_record: Option<u32>,
    pub content_record_range: Option<(u16, u16)>,
    pub exth: BTreeMap<u32, Vec<Vec<u8>>>,
}

impl<'a> PalmDb<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() < 78 {
            return Err("PalmDB header shorter than 78 bytes".into());
        }
        if &bytes[60..64] != b"BOOK" || &bytes[64..68] != b"MOBI" {
            return Err("PalmDB type/creator is not BOOK/MOBI".into());
        }
        let count = be_u16(bytes, 76)? as usize;
        let table_end = 78usize
            .checked_add(count.checked_mul(8).ok_or("record table overflow")?)
            .ok_or("record table overflow")?;
        if table_end > bytes.len() {
            return Err("record table extends past file".into());
        }
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            let off = be_u32(bytes, 78 + i * 8)? as usize;
            if off >= bytes.len() {
                return Err(format!("record {i} offset outside file: {off}"));
            }
            if let Some(prev) = offsets.last().copied() {
                if off <= prev {
                    return Err(format!("record offsets not strictly increasing at {i}"));
                }
            }
            offsets.push(off);
        }
        Ok(Self {
            bytes,
            record_offsets: offsets,
        })
    }

    pub fn record_count(&self) -> usize {
        self.record_offsets.len()
    }

    pub fn bytes_len(&self) -> usize {
        self.bytes.len()
    }

    pub fn record_table_end(&self) -> usize {
        78 + self.record_count() * 8 + 2
    }

    pub fn first_record_offset(&self) -> usize {
        self.record_offsets
            .first()
            .copied()
            .unwrap_or(self.bytes.len())
    }

    pub fn record_bounds(&self, index: usize) -> Result<(usize, usize), String> {
        let start = *self
            .record_offsets
            .get(index)
            .ok_or_else(|| format!("record {index} out of range"))?;
        let end = self
            .record_offsets
            .get(index + 1)
            .copied()
            .unwrap_or(self.bytes.len());
        if end < start || end > self.bytes.len() {
            return Err("record geometry invalid".into());
        }
        Ok((start, end))
    }

    pub fn record(&self, index: usize) -> Result<&'a [u8], String> {
        let (start, end) = self.record_bounds(index)?;
        Ok(&self.bytes[start..end])
    }

    #[allow(dead_code)]
    pub fn find_exact_record(&self, needle: &[u8]) -> Option<usize> {
        (0..self.record_count()).find(|&i| self.record(i).ok() == Some(needle))
    }

    pub fn mobi_header(&self, record_index: usize) -> Result<MobiHeader<'a>, String> {
        MobiHeader::parse(record_index, self.record(record_index)?)
    }
}

impl<'a> MobiHeader<'a> {
    pub fn parse(record_index: usize, record: &'a [u8]) -> Result<Self, String> {
        if record.len() < 40 {
            return Err("record 0 too short".into());
        }
        let compression = be_u16(record, 0)?;
        let text_length = be_u32(record, 4)? as usize;
        let text_record_count = be_u16(record, 8)? as usize;
        let record_size = be_u16(record, 10)? as usize;
        if &record[16..20] != b"MOBI" {
            return Err("MOBI identifier missing".into());
        }
        let mobi_header_length = be_u32(record, 20)? as usize;
        if mobi_header_length < 20
            || 16usize
                .checked_add(mobi_header_length)
                .ok_or("MOBI header overflow")?
                > record.len()
        {
            return Err("MOBI header length out of record range".into());
        }
        let mobi_type = be_u32(record, 24)?;
        let encoding = be_u32(record, 28)?;
        // External MOBI format references document the language field as the
        // 32-bit value at 0x4c relative to the MOBI header magic.
        let locale = be_u32(record, 16 + 0x4c)?;
        let version = be_u32(record, 36)?;
        let first_resource = if record.len() >= 112 {
            be_u32(record, 108)?
        } else {
            u32::MAX
        };
        let exth_flags = if record.len() >= 132 {
            be_u32(record, 128)?
        } else {
            0
        };
        let extra_record_data_flags = if record.len() >= 244 {
            be_u16(record, 242)?
        } else {
            0
        };
        let indx_raw = if record.len() >= 248 {
            be_u32(record, 244)?
        } else {
            u32::MAX
        };
        let indx_record = (indx_raw != u32::MAX).then_some(indx_raw);
        let opt_u32 = |offset: usize| -> Result<Option<u32>, String> {
            if record.len() < offset + 4 {
                return Ok(None);
            }
            let value = be_u32(record, offset)?;
            Ok((value != u32::MAX).then_some(value))
        };
        // External KindleUnpack/MOBI references: KF8 auxiliary/index pointers in record 0.
        let fdst_record = opt_u32(0xc0)?;
        let ncx_record = opt_u32(0xf4)?;
        let fragment_index = opt_u32(0xf8)?;
        let skeleton_index = opt_u32(0xfc)?;
        let datp_index = opt_u32(0x100)?;
        let guide_index = opt_u32(0x104)?;
        let first_non_text = opt_u32(16 + 0x40)?;
        let fcis_record = opt_u32(16 + 0xb8)?;
        let fcis_count = if record.len() >= 16 + 0xbc + 4 {
            Some(be_u32(record, 16 + 0xbc)?)
        } else {
            None
        };
        let flis_record = opt_u32(16 + 0xc0)?;
        let flis_count = if record.len() >= 16 + 0xc4 + 4 {
            Some(be_u32(record, 16 + 0xc4)?)
        } else {
            None
        };
        let fdst_flow_count = if record.len() >= 16 + 0xb4 + 4 {
            Some(be_u32(record, 16 + 0xb4)?)
        } else {
            None
        };
        let index_record = opt_u32(16 + 0xe8)?;
        let skeleton_record = opt_u32(16 + 0xec)?;
        let fragment_record = opt_u32(16 + 0xf8)?;
        let datp_record = opt_u32(16 + 0x100)?;
        let content_record_range = if version < 8 && record.len() >= 16 + 0xb4 {
            let first = be_u16(record, 16 + 0xb0)?;
            let last = be_u16(record, 16 + 0xb2)?;
            (first != u16::MAX || last != u16::MAX).then_some((first, last))
        } else {
            None
        };
        let exth = if exth_flags & 0x40 != 0 {
            parse_exth(record, 16 + mobi_header_length)?
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            record_index,
            record,
            compression,
            text_length,
            text_record_count,
            record_size,
            mobi_header_length,
            mobi_type,
            encoding,
            locale,
            version,
            first_resource,
            exth_flags,
            extra_record_data_flags,
            indx_record,
            fdst_record,
            ncx_record,
            fragment_index,
            skeleton_index,
            datp_index,
            guide_index,
            first_non_text,
            fcis_record,
            fcis_count,
            flis_record,
            flis_count,
            fdst_flow_count,
            index_record,
            skeleton_record,
            fragment_record,
            datp_record,
            content_record_range,
            exth,
        })
    }

    pub fn exth_u32(&self, ty: u32) -> Option<u32> {
        let bytes = self.exth.get(&ty)?.first()?;
        (bytes.len() == 4).then(|| u32::from_be_bytes(bytes.as_slice().try_into().unwrap()))
    }

    pub fn exth_text(&self, ty: u32) -> Option<String> {
        let bytes = self.exth.get(&ty)?.first()?;
        String::from_utf8(bytes.clone()).ok()
    }

    pub fn pointer_inventory(&self) -> Vec<(&'static str, u32, &'static str)> {
        let mut pointers = Vec::new();
        let coordinate = if self.version >= 8 && self.record_index > 0 {
            "section-relative"
        } else {
            "global"
        };
        for (name, value, coordinate) in [
            ("first_non_text", self.first_non_text, coordinate),
            ("first_resource", Some(self.first_resource), coordinate),
            ("fcis", self.fcis_record, coordinate),
            ("flis", self.flis_record, coordinate),
            (
                "fdst",
                (self.version >= 8).then_some(self.fdst_record).flatten(),
                coordinate,
            ),
            ("indx", self.indx_record, coordinate),
            ("ncx", self.ncx_record, coordinate),
            ("fragment", self.index_record, coordinate),
            ("skeleton", self.skeleton_index, coordinate),
            ("datp", self.datp_record, coordinate),
            ("guide", self.guide_index, coordinate),
            (
                "content_first",
                self.content_record_range.map(|(first, _)| u32::from(first)),
                "global",
            ),
            (
                "content_last",
                self.content_record_range.map(|(_, last)| u32::from(last)),
                "global",
            ),
        ] {
            if let Some(value) = value.filter(|value| *value != u32::MAX) {
                pointers.push((name, value, coordinate));
            }
        }
        for (ty, name, coordinate) in [
            (121, "EXTH 121", "global"),
            (201, "EXTH 201", "resource-relative"),
            (202, "EXTH 202", "resource-relative"),
        ] {
            if let Some(value) = self.exth_u32(ty) {
                pointers.push((name, value, coordinate));
            }
        }
        pointers
    }

    pub fn global_record_index(&self, value: u32, coordinate: &str) -> Result<usize, String> {
        if value == u32::MAX {
            return Err("NULL pointer has no global record index".into());
        }
        if coordinate == "section-relative" {
            self.record_index
                .checked_add(value as usize)
                .ok_or_else(|| "section pointer overflow".into())
        } else {
            Ok(value as usize)
        }
    }
}

pub fn reconstruct_text(db: &PalmDb<'_>, header: &MobiHeader<'_>) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(header.text_length);
    for n in 0..header.text_record_count {
        let global = header.record_index + 1 + n;
        let mut record = db.record(global)?.to_vec();
        trim_trailing_data(&mut record, header.extra_record_data_flags)?;
        match header.compression {
            1 => out.extend_from_slice(&record),
            2 => out.extend_from_slice(&palmdoc_decompress(&record)?),
            other => {
                return Err(format!(
                    "unsupported compression in independent audit parser: {other}"
                ));
            }
        }
        if out.len() >= header.text_length {
            break;
        }
    }
    if out.len() < header.text_length {
        return Err(format!(
            "reconstructed text shorter than PalmDOC text length: {} < {}",
            out.len(),
            header.text_length
        ));
    }
    out.truncate(header.text_length);
    Ok(out)
}

/// Decode the physical text records without trusting PalmDOC `text_length` as
/// a truncation instruction.  KF8 has an explicit two-byte bridge record
/// after its text stream; the legacy Dual stub has one text record and then
/// enters the shared resource block.
pub fn reconstruct_text_raw(db: &PalmDb<'_>, header: &MobiHeader<'_>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for index in text_record_indices(db, header)? {
        let record = db.record(index)?;
        let payload = text_payload(record, header.extra_record_data_flags)?;
        match header.compression {
            1 => out.extend_from_slice(payload),
            2 => out.extend_from_slice(&palmdoc_decompress(payload)?),
            other => return Err(format!("unsupported PalmDOC compression: {other}")),
        }
    }
    Ok(out)
}

pub fn text_record_indices(db: &PalmDb<'_>, header: &MobiHeader<'_>) -> Result<Vec<usize>, String> {
    let first = header
        .record_index
        .checked_add(1)
        .ok_or("text index overflow")?;
    if header.version < 8 {
        if first >= db.record_count() {
            return Err("KF7 text record is missing".into());
        }
        return Ok(vec![first]);
    }
    let mut indices = Vec::new();
    for index in first..db.record_count() {
        if db.record(index)? == [0, 0] {
            if indices.is_empty() {
                return Err("KF8 text bridge precedes all text records".into());
            }
            return Ok(indices);
        }
        indices.push(index);
    }
    Err("KF8 text bridge record is missing".into())
}

pub fn text_payload(record: &[u8], flags: u16) -> Result<&[u8], String> {
    if flags == 0 {
        return Ok(record);
    }
    Ok(&record[..parse_trailer(record, flags)?.payload_end])
}

pub fn parse_trailer(record: &[u8], flags: u16) -> Result<PalmDocTrailer, String> {
    if flags == 0 {
        return Ok(PalmDocTrailer {
            payload_end: record.len(),
            multibyte_overlap: Vec::new(),
            advertised_tbs: Vec::new(),
            trailer_byte_count: 0,
            reverse_vwi_bytes: 0,
        });
    }
    let (trailer_byte_count, reverse_vwi_bytes) = reverse_varint_at_end(record)?;
    if trailer_byte_count < reverse_vwi_bytes || trailer_byte_count > record.len() {
        return Err("trailing-data length is outside text record".into());
    }
    let tbs_start = record.len() - trailer_byte_count;
    let tbs_end = record.len() - reverse_vwi_bytes;
    let shifted = flags;
    let has_multibyte = shifted & 1 != 0;
    let has_tbs = shifted & 2 != 0;
    let mut marker_start = tbs_start;
    let mut overlap = Vec::new();
    if has_tbs {
        if tbs_start == 0 {
            return Err("TBS trailer has no preceding multibyte marker".into());
        }
        let mut found = None;
        for overlap_len in 0..=3usize {
            let marker = tbs_start
                .checked_sub(overlap_len + 1)
                .ok_or("marker underflow")?;
            if record[marker] != overlap_len as u8 {
                continue;
            }
            let candidate = &record[marker + 1..tbs_start];
            if overlap_len == 0 || candidate.iter().all(|byte| (*byte & 0xc0) == 0x80) {
                found = Some((marker, candidate.to_vec()));
                break;
            }
        }
        let (marker, candidate) = found.ok_or("multibyte marker is not framed before TBS")?;
        marker_start = marker;
        overlap = candidate;
    } else if has_multibyte {
        return Err(
            "multibyte flag without TBS framing is unsupported by this audit parser".into(),
        );
    }
    Ok(PalmDocTrailer {
        payload_end: marker_start,
        multibyte_overlap: overlap,
        advertised_tbs: if has_tbs {
            record[tbs_start..tbs_end].to_vec()
        } else {
            Vec::new()
        },
        trailer_byte_count,
        reverse_vwi_bytes,
    })
}

fn reverse_varint_at_end(data: &[u8]) -> Result<(usize, usize), String> {
    if data.is_empty() {
        return Err("empty record has no reverse VWI".into());
    }
    let mut value = 0usize;
    let mut shift = 0usize;
    for (used, byte) in data.iter().rev().take(8).copied().enumerate() {
        value |= usize::from(byte & 0x7f)
            .checked_shl(u32::try_from(shift).map_err(|_| "reverse VWI overflow")?)
            .ok_or("reverse VWI overflow")?;
        if byte & 0x80 != 0 {
            if value < used + 1 {
                return Err("reverse VWI length is shorter than its encoding".into());
            }
            return Ok((value, used + 1));
        }
        shift += 7;
    }
    Err("unterminated reverse VWI".into())
}

pub fn parse_fdst(record: &[u8]) -> Result<FdstRecord, String> {
    if record.len() < 12 || &record[..4] != b"FDST" {
        return Err("FDST framing missing".into());
    }
    let header_size = be_u32(record, 4)? as usize;
    let flow_count = be_u32(record, 8)? as usize;
    let end = header_size
        .checked_add(flow_count.checked_mul(8).ok_or("FDST range overflow")?)
        .ok_or("FDST range overflow")?;
    if header_size < 12 || end > record.len() {
        return Err("FDST ranges outside record".into());
    }
    let mut ranges = Vec::with_capacity(flow_count);
    for index in 0..flow_count {
        let offset = header_size + index * 8;
        ranges.push((be_u32(record, offset)?, be_u32(record, offset + 4)?));
    }
    Ok(FdstRecord {
        header_size,
        flow_count,
        ranges,
    })
}

pub fn parse_indx(record: &[u8]) -> Result<IndxRecord, String> {
    if record.len() < 52 || &record[..4] != b"INDX" {
        return Err("INDX framing missing".into());
    }
    let idxt_offset = be_u32(record, 20)? as usize;
    let detail_count = be_u32(record, 24)? as usize;
    let entry_count = be_u32(record, 36)? as usize;
    if idxt_offset < 52
        || idxt_offset
            .checked_add(4 + detail_count * 2)
            .is_none_or(|end| end > record.len())
    {
        return Err("INDX IDXT table outside record".into());
    }
    if &record[idxt_offset..idxt_offset + 4] != b"IDXT" {
        return Err("INDX IDXT signature missing".into());
    }
    let mut tagx = Vec::new();
    let is_main = be_u32(record, 12)? == 0;
    let tagx_offset = be_u32(record, 4)? as usize;
    if is_main && tagx_offset != 0 {
        if tagx_offset > record.len()
            || tagx_offset
                .checked_add(12)
                .is_none_or(|end| end > idxt_offset)
            || record.get(tagx_offset..tagx_offset + 4) != Some(b"TAGX")
        {
            return Err("INDX TAGX table outside record".into());
        }
        let tagx_len = be_u32(record, tagx_offset + 4)? as usize;
        let tagx_end = tagx_offset.checked_add(tagx_len).ok_or("TAGX overflow")?;
        if tagx_len < 12 || tagx_end > idxt_offset {
            return Err("INDX TAGX length outside record".into());
        }
        let mut cursor = tagx_offset + 12;
        while cursor + 4 <= tagx_end {
            let def = (record[cursor], record[cursor + 1], record[cursor + 2]);
            cursor += 4;
            if def == (0, 0, 0) {
                break;
            }
            tagx.push(def);
        }
    }
    let mut row_offsets = Vec::with_capacity(detail_count);
    for index in 0..detail_count {
        let offset = be_u16(record, idxt_offset + 4 + index * 2)? as usize;
        if offset < 52 || offset >= idxt_offset {
            return Err("INDX row offset outside detail area".into());
        }
        row_offsets.push(offset);
    }
    Ok(IndxRecord {
        idxt_offset,
        entry_count,
        detail_count,
        tagx,
        row_offsets,
    })
}

pub fn decode_indx_rows(record: &[u8], index: &IndxRecord) -> Result<Vec<IndxRow>, String> {
    let mut rows = Vec::with_capacity(index.row_offsets.len());
    for (row_index, &start) in index.row_offsets.iter().enumerate() {
        let len = *record.get(start).ok_or("INDX row length outside record")? as usize;
        let text_start = start.checked_add(1).ok_or("INDX row overflow")?;
        let control_pos = text_start.checked_add(len).ok_or("INDX row overflow")?;
        let control = *record
            .get(control_pos)
            .ok_or("INDX control outside record")?;
        let mut cursor = control_pos + 1;
        let mut values = BTreeMap::new();
        for &(tag, values_per, mask) in &index.tagx {
            let shift = mask.trailing_zeros();
            let occurrences = if mask.count_ones() == 1 {
                u32::from(control & mask != 0)
            } else {
                u32::from((control & mask) >> shift)
            };
            for _ in 0..occurrences {
                for _ in 0..values_per {
                    let (value, used) = decode_vwi(record, cursor)?;
                    cursor = cursor.checked_add(used).ok_or("INDX value overflow")?;
                    values.entry(tag).or_insert_with(Vec::new).push(value);
                }
            }
        }
        if cursor > index.idxt_offset {
            return Err(format!("INDX row {row_index} overlaps IDXT"));
        }
        rows.push(IndxRow {
            text: record[text_start..control_pos].to_vec(),
            values,
        });
    }
    Ok(rows)
}

pub fn decode_vwi(bytes: &[u8], start: usize) -> Result<(u32, usize), String> {
    let mut value = 0u32;
    for (used, byte) in bytes
        .get(start..)
        .ok_or("VWI starts outside record")?
        .iter()
        .copied()
        .enumerate()
    {
        value = value
            .checked_shl(7)
            .ok_or("VWI overflow")?
            .checked_add(u32::from(byte & 0x7f))
            .ok_or("VWI overflow")?;
        if byte & 0x80 != 0 {
            return Ok((value, used + 1));
        }
        if used >= 4 {
            return Err("VWI exceeds five bytes".into());
        }
    }
    Err("unterminated VWI".into())
}

pub fn encode_vwi_audit(mut value: u32) -> Vec<u8> {
    let mut chunks = vec![value & 0x7f];
    value >>= 7;
    while value != 0 {
        chunks.push(value & 0x7f);
        value >>= 7;
    }
    chunks.reverse();
    let last = chunks.len() - 1;
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| (chunk as u8) | if index == last { 0x80 } else { 0 })
        .collect()
}

pub fn parse_aux_record(record: &[u8]) -> Result<(&'static str, usize), String> {
    let kind = if record.starts_with(b"FCIS") {
        "FCIS"
    } else if record.starts_with(b"FLIS") {
        "FLIS"
    } else if record.starts_with(b"DATP") {
        "DATP"
    } else {
        return Err("unknown auxiliary record".into());
    };
    if record.len() < 8 {
        return Err(format!("{kind} record header is truncated"));
    }
    let declared = be_u32(record, 4)? as usize;
    if declared < 8 || declared > record.len() {
        return Err(format!("{kind} declared geometry outside record"));
    }
    Ok((kind, declared))
}

pub fn palmdoc_decompress(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < input.len() {
        let c = input[i];
        i += 1;
        match c {
            0x00 | 0x09..=0x7f => out.push(c),
            0x01..=0x08 => {
                let n = c as usize;
                if i + n > input.len() {
                    return Err("PalmDOC literal run exceeds record".into());
                }
                out.extend_from_slice(&input[i..i + n]);
                i += n;
            }
            0x80..=0xbf => {
                if i >= input.len() {
                    return Err("PalmDOC back-reference missing second byte".into());
                }
                let pair = (((c as u16) & 0x3f) << 8) | input[i] as u16;
                i += 1;
                let distance = (pair >> 3) as usize;
                let length = ((pair & 0x7) + 3) as usize;
                if distance == 0 || distance > out.len() {
                    return Err("PalmDOC back-reference distance invalid".into());
                }
                for _ in 0..length {
                    let b = out[out.len() - distance];
                    out.push(b);
                }
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
    }
    Ok(out)
}

pub fn assert_record_pointer(
    db: &PalmDb<'_>,
    value: u32,
    field: &str,
    allow_none: bool,
) -> Result<(), String> {
    if allow_none && value == u32::MAX {
        return Ok(());
    }
    if value as usize >= db.record_count() {
        return Err(format!(
            "{field} points outside actual PalmDB record count: {value} >= {}",
            db.record_count()
        ));
    }
    Ok(())
}

fn parse_exth(record: &[u8], offset: usize) -> Result<BTreeMap<u32, Vec<Vec<u8>>>, String> {
    if offset + 12 > record.len() || &record[offset..offset + 4] != b"EXTH" {
        return Err("EXTH flag set but EXTH header missing".into());
    }
    let length = be_u32(record, offset + 4)? as usize;
    let count = be_u32(record, offset + 8)? as usize;
    if length < 12 || offset + length > record.len() {
        return Err("EXTH length out of range".into());
    }
    let mut pos = offset + 12;
    let end = offset + length;
    let mut out: BTreeMap<u32, Vec<Vec<u8>>> = BTreeMap::new();
    for _ in 0..count {
        if pos + 8 > end {
            return Err("EXTH record header out of range".into());
        }
        let ty = be_u32(record, pos)?;
        let len = be_u32(record, pos + 4)? as usize;
        if len < 8 || pos + len > end {
            return Err("EXTH record length out of range".into());
        }
        out.entry(ty)
            .or_default()
            .push(record[pos + 8..pos + len].to_vec());
        pos += len;
    }
    if pos > end {
        return Err("EXTH records exceed EXTH length".into());
    }
    Ok(out)
}

fn trim_trailing_data(data: &mut Vec<u8>, flags: u16) -> Result<(), String> {
    let mut shifted = flags;
    let multibyte = shifted & 1 != 0;
    let mut trailers = 0usize;
    while shifted > 1 {
        if shifted & 2 != 0 {
            trailers += 1;
        }
        shifted >>= 1;
    }
    for _ in 0..trailers {
        let size = reverse_varint_size(data)?;
        if size > data.len() {
            return Err("trailing-data size exceeds text record".into());
        }
        data.truncate(data.len() - size);
    }
    if multibyte && !data.is_empty() {
        let size = (data[data.len() - 1] & 0x03) as usize + 1;
        if size > data.len() {
            return Err("multibyte overlap exceeds text record".into());
        }
        data.truncate(data.len() - size);
    }
    Ok(())
}

fn reverse_varint_size(data: &[u8]) -> Result<usize, String> {
    if data.is_empty() {
        return Err("empty trailing-data record".into());
    }
    let mut value = 0usize;
    let mut shift = 0usize;
    let mut consumed = 0usize;
    for &b in data.iter().rev().take(4) {
        consumed += 1;
        value |= ((b & 0x7f) as usize) << shift;
        if b & 0x80 != 0 {
            return if value >= consumed {
                Ok(value)
            } else {
                Err("invalid trailing-data size".into())
            };
        }
        shift += 7;
    }
    Err("unterminated trailing-data size".into())
}

fn be_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let s = bytes
        .get(offset..offset + 2)
        .ok_or("u16 read out of range")?;
    Ok(u16::from_be_bytes(s.try_into().unwrap()))
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let s = bytes
        .get(offset..offset + 4)
        .ok_or("u32 read out of range")?;
    Ok(u32::from_be_bytes(s.try_into().unwrap()))
}
