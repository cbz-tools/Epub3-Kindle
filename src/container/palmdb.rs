use std::io::Write;

#[derive(Debug, Clone)]
pub struct PalmDbRecord {
    pub data: Vec<u8>,
    pub attributes: u8,
}

#[derive(Debug, Clone)]
pub struct PalmDb {
    pub name: String,
    pub records: Vec<PalmDbRecord>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PalmDbEncodeOptions {
    pub(crate) creation_time: u32,
    pub(crate) modification_time: u32,
}

const PDB_HEADER_LEN: usize = 78;
const PDB_RECORD_ENTRY_LEN: usize = 8;
const PDB_NAME_MAX_LEN: usize = 31;
const PDB_RECORD_TABLE_TERMINATOR_LEN: usize = 2;
impl PalmDb {
    pub fn new(name: impl Into<String>, records: Vec<PalmDbRecord>) -> Self {
        Self {
            name: name.into(),
            records,
        }
    }

    pub fn validate(&self) -> crate::error::Result<()> {
        if self.records.len() > u16::MAX as usize {
            return Err(crate::error::Error::Output(
                "PalmDB record count exceeds u16".to_owned(),
            ));
        }
        let record_table_len = self
            .records
            .len()
            .checked_mul(PDB_RECORD_ENTRY_LEN)
            .ok_or_else(|| {
                crate::error::Error::Output("PalmDB record table overflow".to_owned())
            })?;
        let mut offset = PDB_HEADER_LEN
            .checked_add(record_table_len)
            .and_then(|value| value.checked_add(PDB_RECORD_TABLE_TERMINATOR_LEN))
            .ok_or_else(|| crate::error::Error::Output("PalmDB offset overflow".to_owned()))?;
        if offset > u32::MAX as usize {
            return Err(crate::error::Error::Output(
                "PalmDB first record offset exceeds u32".to_owned(),
            ));
        }
        for record in &self.records {
            offset = offset
                .checked_add(record.data.len())
                .ok_or_else(|| crate::error::Error::Output("PalmDB data overflow".to_owned()))?;
            if offset > u32::MAX as usize {
                return Err(crate::error::Error::Output(
                    "PalmDB record offset exceeds u32".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn encode_checked(&self) -> crate::error::Result<Vec<u8>> {
        self.encode_checked_with_options(PalmDbEncodeOptions::default())
    }

    pub(crate) fn encode_checked_with_options(
        &self,
        options: PalmDbEncodeOptions,
    ) -> crate::error::Result<Vec<u8>> {
        self.validate()?;
        Ok(self.encode_unchecked_with_timestamps(options.creation_time, options.modification_time))
    }

    pub(crate) fn write_stream_checked<W: Write, I>(
        name: &str,
        record_lengths: &[usize],
        records: I,
        writer: &mut W,
        path: &str,
    ) -> crate::error::Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        Self::write_stream_checked_with_options(
            name,
            record_lengths,
            records,
            writer,
            path,
            PalmDbEncodeOptions::default(),
        )
    }

    pub(crate) fn write_stream_checked_with_options<W: Write, I>(
        name: &str,
        record_lengths: &[usize],
        records: I,
        writer: &mut W,
        path: &str,
        options: PalmDbEncodeOptions,
    ) -> crate::error::Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        let offsets = record_offsets(record_lengths)?;
        let header = make_header(
            name,
            &offsets,
            options.creation_time,
            options.modification_time,
            |_| 0,
        );
        writer
            .write_all(&header)
            .map_err(|source| crate::error::Error::Io {
                path: path.to_owned(),
                source,
            })?;
        let mut record_index = 0;
        for record in records {
            let data = record.as_ref();
            let expected_length = record_lengths.get(record_index).ok_or_else(|| {
                crate::error::Error::Output("PalmDB record count mismatch".to_owned())
            })?;
            if data.len() != *expected_length {
                return Err(crate::error::Error::Output(
                    "PalmDB record length mismatch".to_owned(),
                ));
            }
            writer
                .write_all(data)
                .map_err(|source| crate::error::Error::Io {
                    path: path.to_owned(),
                    source,
                })?;
            record_index += 1;
        }
        if record_index != record_lengths.len() {
            return Err(crate::error::Error::Output(
                "PalmDB record count mismatch".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn write_result_stream_checked_with_options<W: Write, I>(
        name: &str,
        record_lengths: &[usize],
        records: I,
        writer: &mut W,
        path: &str,
        options: PalmDbEncodeOptions,
    ) -> crate::error::Result<()>
    where
        I: IntoIterator<Item = crate::error::Result<Vec<u8>>>,
    {
        let offsets = record_offsets(record_lengths)?;
        let header = make_header(
            name,
            &offsets,
            options.creation_time,
            options.modification_time,
            |_| 0,
        );
        writer
            .write_all(&header)
            .map_err(|source| crate::error::Error::Io {
                path: path.to_owned(),
                source,
            })?;
        let mut record_index = 0;
        for record in records {
            let data = record?;
            let expected_length = record_lengths.get(record_index).ok_or_else(|| {
                crate::error::Error::Output("PalmDB record count mismatch".to_owned())
            })?;
            if data.len() != *expected_length {
                return Err(crate::error::Error::Output(
                    "PalmDB record length mismatch".to_owned(),
                ));
            }
            writer
                .write_all(&data)
                .map_err(|source| crate::error::Error::Io {
                    path: path.to_owned(),
                    source,
                })?;
            record_index += 1;
        }
        if record_index != record_lengths.len() {
            return Err(crate::error::Error::Output(
                "PalmDB record count mismatch".to_owned(),
            ));
        }
        Ok(())
    }

    fn encode_unchecked_with_timestamps(
        &self,
        creation_time: u32,
        modification_time: u32,
    ) -> Vec<u8> {
        let record_table_len = self.records.len() * PDB_RECORD_ENTRY_LEN;
        let first_record_offset =
            PDB_HEADER_LEN + record_table_len + PDB_RECORD_TABLE_TERMINATOR_LEN;
        let mut bytes = Vec::with_capacity(first_record_offset);
        self.write_unchecked(&mut bytes, "memory", creation_time, modification_time)
            .expect("writing to Vec cannot fail");
        bytes
    }

    fn write_unchecked<W: Write>(
        &self,
        writer: &mut W,
        path: &str,
        creation_time: u32,
        modification_time: u32,
    ) -> crate::error::Result<()> {
        let record_lengths = self
            .records
            .iter()
            .map(|record| record.data.len())
            .collect::<Vec<_>>();
        let offsets = record_offsets(&record_lengths)?;
        let header = make_header(
            &self.name,
            &offsets,
            creation_time,
            modification_time,
            |index| {
                // Record 0 keeps canonical zero attributes. Later attributes are
                // preserved for resource/record flags; this writer leaves resource
                // records unflagged because their role is carried by MOBI.
                if index == 0 {
                    0
                } else {
                    self.records[index].attributes
                }
            },
        );
        writer
            .write_all(&header)
            .map_err(|source| crate::error::Error::Io {
                path: path.to_owned(),
                source,
            })?;
        for record in &self.records {
            writer
                .write_all(&record.data)
                .map_err(|source| crate::error::Error::Io {
                    path: path.to_owned(),
                    source,
                })?;
        }
        Ok(())
    }
}

fn record_offsets(record_lengths: &[usize]) -> crate::error::Result<Vec<usize>> {
    if record_lengths.len() > u16::MAX as usize {
        return Err(crate::error::Error::Output(
            "PalmDB record count exceeds u16".to_owned(),
        ));
    }
    let record_table_len = record_lengths
        .len()
        .checked_mul(PDB_RECORD_ENTRY_LEN)
        .ok_or_else(|| crate::error::Error::Output("PalmDB record table overflow".to_owned()))?;
    let first_record_offset = PDB_HEADER_LEN
        .checked_add(record_table_len)
        .and_then(|value| value.checked_add(PDB_RECORD_TABLE_TERMINATOR_LEN))
        .ok_or_else(|| crate::error::Error::Output("PalmDB offset overflow".to_owned()))?;
    if first_record_offset > u32::MAX as usize {
        return Err(crate::error::Error::Output(
            "PalmDB first record offset exceeds u32".to_owned(),
        ));
    }
    let mut offsets = Vec::with_capacity(record_lengths.len());
    let mut offset = first_record_offset;
    for record_length in record_lengths {
        offsets.push(offset);
        offset = offset
            .checked_add(*record_length)
            .ok_or_else(|| crate::error::Error::Output("PalmDB data overflow".to_owned()))?;
        if offset > u32::MAX as usize {
            return Err(crate::error::Error::Output(
                "PalmDB record offset exceeds u32".to_owned(),
            ));
        }
    }
    Ok(offsets)
}

fn make_header(
    name: &str,
    offsets: &[usize],
    creation_time: u32,
    modification_time: u32,
    attributes: impl Fn(usize) -> u8,
) -> Vec<u8> {
    let record_table_len = offsets.len() * PDB_RECORD_ENTRY_LEN;
    let first_record_offset = PDB_HEADER_LEN + record_table_len + PDB_RECORD_TABLE_TERMINATOR_LEN;
    let mut header = vec![0u8; first_record_offset];
    let mut name_len = name.len().min(PDB_NAME_MAX_LEN);
    while !name.is_char_boundary(name_len) {
        name_len -= 1;
    }
    header[..name_len].copy_from_slice(&name.as_bytes()[..name_len]);
    put_u16(&mut header, 32, 0);
    put_u16(&mut header, 34, 0);
    put_u32(&mut header, 36, creation_time);
    put_u32(&mut header, 40, modification_time);
    header[60..64].copy_from_slice(b"BOOK");
    header[64..68].copy_from_slice(b"MOBI");
    let unique_id_seed = if offsets.is_empty() {
        u32::MAX
    } else {
        (offsets.len() as u32)
            .checked_mul(2)
            .and_then(|value| value.checked_sub(1))
            .expect("record count was checked against u16")
    };
    put_u32(&mut header, 68, unique_id_seed);
    put_u32(&mut header, 72, 0);
    put_u16(&mut header, 76, offsets.len() as u16);
    for (index, offset) in offsets.iter().enumerate() {
        let base = PDB_HEADER_LEN + index * PDB_RECORD_ENTRY_LEN;
        put_u32(&mut header, base, *offset as u32);
        header[base + 4] = if index == 0 { 0 } else { attributes(index) };
        // Observed KindleGen behavior: the three-byte record UID seed is
        // zero for record 0 and advances by two. This is a PalmDB
        // serialization compatibility choice, not reader-facing content.
        let unique = (index as u32)
            .checked_mul(2)
            .expect("record count was checked");
        let unique = unique.to_be_bytes();
        header[base + 5..base + 8].copy_from_slice(&unique[1..]);
    }
    // PalmDB readers expect the table terminator to occupy the two bytes
    // immediately before the first record. It is part of the offset geometry,
    // not a synthetic bridge record.
    header
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
