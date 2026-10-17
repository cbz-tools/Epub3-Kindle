use std::io::Write;

mod palmdb;

pub(crate) use palmdb::{PalmDb, PalmDbEncodeOptions};

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
