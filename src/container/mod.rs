use std::io::Write;

mod palmdb;

pub(crate) use palmdb::{PalmDb, PalmDbEncodeOptions, PalmDbRecord};

pub(crate) fn encode_kf8_records(
    records: impl IntoIterator<Item = Vec<u8>>,
) -> crate::error::Result<Vec<u8>> {
    let records = records
        .into_iter()
        .map(|data| PalmDbRecord {
            data,
            // KF8 records carry their roles through the MOBI pointers and
            // headers; the PalmDB attribute byte remains zero.
            attributes: 0,
        })
        .collect();
    // The generated database name is part of the established AZW3 parity
    // contract, so keep it at the PalmDB boundary rather than in KF8 code.
    PalmDb::new("kindle-format", records).encode_checked()
}

pub(crate) fn write_kf8_records<W: Write>(
    record_zero: Vec<u8>,
    record_lengths: &[usize],
    records: impl IntoIterator<Item = Vec<u8>>,
    writer: &mut W,
    path: &str,
) -> crate::error::Result<()> {
    PalmDb::write_stream_checked(
        "kindle-format",
        record_lengths,
        std::iter::once(record_zero).chain(records),
        writer,
        path,
    )
}

pub(crate) fn write_records<W: Write>(
    name: &str,
    options: PalmDbEncodeOptions,
    record_lengths: &[usize],
    records: impl IntoIterator<Item = crate::error::Result<Vec<u8>>>,
    writer: &mut W,
    path: &str,
) -> crate::error::Result<()> {
    PalmDb::write_result_stream_checked_with_options(
        name,
        record_lengths,
        records,
        writer,
        path,
        options,
    )
}
