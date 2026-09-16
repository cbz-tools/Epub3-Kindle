use std::io::Write;

use crate::container::PalmDbEncodeOptions;
use crate::error::Result;
use crate::kf8::Kf8Book;

use super::layout;

pub(crate) fn serialize_to_writer<W: Write>(
    book: Kf8Book,
    writer: &mut W,
    path: &str,
) -> Result<()> {
    let plan = layout::build_dual_layout(book)?;
    let record_lengths = plan.record_lengths()?;
    let palmdb_name = plan.palmdb_name().to_owned();
    crate::container::write_records(
        &palmdb_name,
        PalmDbEncodeOptions {
            creation_time: 3_029_529_600,
            modification_time: 3_029_529_600,
        },
        &record_lengths,
        plan.into_records(),
        writer,
        path,
    )
}
