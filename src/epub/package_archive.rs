//! Bounded EPUB ZIP access and OCF archive extraction limits.

use std::collections::HashSet;
use std::io::{Cursor, Read};

use unicode_normalization::UnicodeNormalization;
use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::xhtml::path::normalize_path_checked;

/// Maximum decompressed size accepted for one EPUB ZIP entry.
const MAX_EPUB_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum decompressed bytes accepted across all resource reads in one EPUB.
const MAX_EPUB_TOTAL_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
/// Maximum number of ZIP entry extractions accepted for one EPUB.
const MAX_EPUB_EXTRACTED_ENTRY_COUNT: usize = 100_000;
/// Keep the first allocation independent of attacker-controlled ZIP metadata.
const EPUB_ENTRY_INITIAL_CAPACITY: usize = 64 * 1024;

pub(super) struct BoundedZipArchive<R> {
    archive: ZipArchive<R>,
    total_extracted_bytes: u64,
    extracted_entry_count: usize,
}

impl<R: Read + std::io::Seek> BoundedZipArchive<R> {
    pub(super) fn new(reader: R) -> Result<Self> {
        let archive = ZipArchive::new(reader)
            .map_err(|error| Error::InvalidEpub(format!("not a readable EPUB ZIP: {error}")))?;
        if archive.len() > MAX_EPUB_EXTRACTED_ENTRY_COUNT {
            return Err(Error::InvalidEpub(format!(
                "EPUB ZIP entry count exceeds limit of {MAX_EPUB_EXTRACTED_ENTRY_COUNT}"
            )));
        }
        Ok(Self {
            archive,
            total_extracted_bytes: 0,
            extracted_entry_count: 0,
        })
    }

    pub(super) fn entry_size(&mut self, path: &str) -> Result<u64> {
        self.archive
            .by_name(path)
            .map(|entry| entry.size())
            .map_err(|error| Error::InvalidEpub(format!("missing EPUB entry {path}: {error}")))
    }

    pub(super) fn contains(&self, path: &str) -> bool {
        self.archive.file_names().any(|name| name == path)
    }
}

pub(super) fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut BoundedZipArchive<R>,
    path: &str,
) -> Result<Vec<u8>> {
    if archive.extracted_entry_count >= MAX_EPUB_EXTRACTED_ENTRY_COUNT {
        return Err(Error::InvalidEpub(format!(
            "EPUB resource entry count exceeds limit of {MAX_EPUB_EXTRACTED_ENTRY_COUNT}"
        )));
    }
    let remaining_total = MAX_EPUB_TOTAL_EXTRACTED_BYTES
        .checked_sub(archive.total_extracted_bytes)
        .ok_or_else(|| Error::InvalidEpub("EPUB extracted byte count overflow".to_owned()))?;
    if remaining_total == 0 {
        return Err(Error::InvalidEpub(format!(
            "EPUB extracted bytes exceed limit of {MAX_EPUB_TOTAL_EXTRACTED_BYTES}"
        )));
    }
    let data = {
        let entry = archive
            .archive
            .by_name(path)
            .map_err(|error| Error::InvalidEpub(format!("missing EPUB entry {path}: {error}")))?;
        if entry.size() > MAX_EPUB_ENTRY_BYTES {
            return Err(Error::InvalidEpub(format!(
                "EPUB entry {path} exceeds per-entry limit of {MAX_EPUB_ENTRY_BYTES} bytes"
            )));
        }
        let read_limit = remaining_total.min(MAX_EPUB_ENTRY_BYTES);
        let mut data = Vec::with_capacity(EPUB_ENTRY_INITIAL_CAPACITY);
        let mut limited = entry.take(read_limit + 1);
        limited.read_to_end(&mut data).map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
        data
    };
    if data.len() as u64 > MAX_EPUB_ENTRY_BYTES {
        return Err(Error::InvalidEpub(format!(
            "EPUB entry {path} exceeds per-entry limit of {MAX_EPUB_ENTRY_BYTES} bytes"
        )));
    }
    if data.len() as u64 > remaining_total {
        return Err(Error::InvalidEpub(format!(
            "EPUB extracted bytes exceed total limit of {MAX_EPUB_TOTAL_EXTRACTED_BYTES}"
        )));
    }
    let actual_size = u64::try_from(data.len())
        .map_err(|_| Error::InvalidEpub("EPUB extracted byte count exceeds u64".to_owned()))?;
    archive.total_extracted_bytes = archive
        .total_extracted_bytes
        .checked_add(actual_size)
        .ok_or_else(|| Error::InvalidEpub("EPUB extracted byte count overflow".to_owned()))?;
    archive.extracted_entry_count = archive
        .extracted_entry_count
        .checked_add(1)
        .ok_or_else(|| Error::InvalidEpub("EPUB resource entry count overflow".to_owned()))?;
    Ok(data)
}

pub(super) fn validate_ocf_paths(input: &[u8]) -> Result<()> {
    validate_ocf_local_headers(input)?;
    let mut archive = ZipArchive::new(Cursor::new(input))
        .map_err(|error| Error::InvalidEpub(format!("not a readable EPUB ZIP: {error}")))?;
    let central_start = usize::try_from(archive.central_directory_start())
        .map_err(|_| Error::InvalidEpub("ZIP central directory offset is too large".to_owned()))?;
    let mut normalized_paths = HashSet::new();
    let mut canonical_paths = HashSet::new();
    let mut cursor = central_start;
    let mut index = 0;
    loop {
        let signature = input
            .get(cursor..)
            .and_then(|remaining| remaining.get(..4))
            .ok_or_else(|| Error::InvalidEpub("ZIP central directory is truncated".to_owned()))?;
        if signature == b"PK\x05\x05" || signature == b"PK\x05\x06" || signature == b"PK\x06\x06" {
            break;
        }
        if signature != b"PK\x01\x02" {
            return Err(Error::InvalidEpub(format!(
                "ZIP central directory entry {index} has an invalid signature"
            )));
        }
        let name_len = read_u16(input, cursor + 28)? as usize;
        let extra_len = read_u16(input, cursor + 30)? as usize;
        let comment_len = read_u16(input, cursor + 32)? as usize;
        let name_start = cursor + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| Error::InvalidEpub("ZIP entry name range overflow".to_owned()))?;
        let entry_end = name_end
            .checked_add(extra_len)
            .and_then(|end| end.checked_add(comment_len))
            .ok_or_else(|| Error::InvalidEpub("ZIP central entry range overflow".to_owned()))?;
        if entry_end > input.len() {
            return Err(Error::InvalidEpub(format!(
                "ZIP central directory entry {index} is truncated"
            )));
        }
        let name = std::str::from_utf8(&input[name_start..name_end]).map_err(|error| {
            Error::InvalidEpub(format!("OCF ZIP entry {index} name is not UTF-8: {error}"))
        })?;
        let flags = read_u16(input, cursor + 8)?;
        let method = read_u16(input, cursor + 10)?;
        if flags & 1 != 0 {
            return Err(Error::InvalidEpub(format!(
                "encrypted OCF ZIP entry {name:?} is unsupported"
            )));
        }
        if method != 0 && method != 8 {
            return Err(Error::InvalidEpub(format!(
                "unsupported OCF ZIP compression method {method} for {name:?}"
            )));
        }
        let normalized = if name.starts_with(['/', '\\']) {
            None
        } else {
            normalize_path_checked(name)
        }
        .ok_or_else(|| {
            Error::InvalidEpub(format!(
                "OCF ZIP entry {name:?} is not a valid path within the container root"
            ))
        })?;
        if !normalized_paths.insert(normalized.clone()) {
            return Err(Error::InvalidEpub(format!(
                "duplicate/conflicting OCF ZIP entry path {normalized}"
            )));
        }
        let canonical = normalized
            .nfc()
            .flat_map(char::to_lowercase)
            .collect::<String>();
        if !canonical_paths.insert(canonical) {
            return Err(Error::InvalidEpub(format!(
                "duplicate/conflicting OCF ZIP entry path {normalized} after Unicode normalization/case-fold"
            )));
        }
        cursor = entry_end;
        index += 1;
    }
    if index == 0 {
        return Err(Error::InvalidEpub("EPUB ZIP has no entries".to_owned()));
    }
    let mut mimetype = archive
        .by_name("mimetype")
        .map_err(|error| Error::InvalidEpub(format!("cannot read EPUB mimetype entry: {error}")))?;
    const EPUB_MIMETYPE: &[u8] = b"application/epub+zip";
    if mimetype.size() != EPUB_MIMETYPE.len() as u64 {
        return Err(Error::InvalidEpub(
            "EPUB mimetype entry payload must be exactly application/epub+zip".to_owned(),
        ));
    }
    let mut payload = Vec::new();
    mimetype
        .by_ref()
        .take(EPUB_MIMETYPE.len() as u64 + 1)
        .read_to_end(&mut payload)
        .map_err(|error| {
            Error::InvalidEpub(format!("cannot read EPUB mimetype entry payload: {error}"))
        })?;
    if payload != EPUB_MIMETYPE {
        return Err(Error::InvalidEpub(
            "EPUB mimetype entry payload must be exactly application/epub+zip".to_owned(),
        ));
    }
    Ok(())
}

fn validate_ocf_local_headers(input: &[u8]) -> Result<()> {
    let mut cursor = 0usize;
    while input
        .get(cursor..cursor.saturating_add(4))
        .is_some_and(|signature| signature == b"PK\x03\x04")
    {
        let flags = read_u16(input, cursor + 6)?;
        let method = read_u16(input, cursor + 8)?;
        let name_len = read_u16(input, cursor + 26)? as usize;
        let extra_len = read_u16(input, cursor + 28)? as usize;
        let name_start = cursor
            .checked_add(30)
            .ok_or_else(|| Error::InvalidEpub("ZIP local header range overflow".to_owned()))?;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| Error::InvalidEpub("ZIP local entry name range overflow".to_owned()))?;
        let data_start = name_end
            .checked_add(extra_len)
            .ok_or_else(|| Error::InvalidEpub("ZIP local entry data range overflow".to_owned()))?;
        if data_start > input.len() {
            return Ok(());
        }
        let name = std::str::from_utf8(&input[name_start..name_end]).unwrap_or("<non-UTF-8>");
        if flags & 1 != 0 {
            return Err(Error::InvalidEpub(format!(
                "encrypted OCF ZIP entry {name:?} is unsupported"
            )));
        }
        if method != 0 && method != 8 {
            return Err(Error::InvalidEpub(format!(
                "unsupported OCF ZIP compression method {method} for {name:?}"
            )));
        }
        if flags & 8 != 0 {
            break;
        }
        let compressed_size = u32::from_le_bytes([
            *input
                .get(cursor + 18)
                .ok_or_else(|| Error::InvalidEpub("ZIP local header is truncated".to_owned()))?,
            *input
                .get(cursor + 19)
                .ok_or_else(|| Error::InvalidEpub("ZIP local header is truncated".to_owned()))?,
            *input
                .get(cursor + 20)
                .ok_or_else(|| Error::InvalidEpub("ZIP local header is truncated".to_owned()))?,
            *input
                .get(cursor + 21)
                .ok_or_else(|| Error::InvalidEpub("ZIP local header is truncated".to_owned()))?,
        ]) as usize;
        let Some(next) = data_start.checked_add(compressed_size) else {
            return Ok(());
        };
        if next > input.len() {
            return Ok(());
        }
        cursor = next;
    }
    Ok(())
}

fn read_u16(input: &[u8], offset: usize) -> Result<u16> {
    let bytes = input
        .get(offset..offset + 2)
        .ok_or_else(|| Error::InvalidEpub("ZIP record is truncated".to_owned()))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}
