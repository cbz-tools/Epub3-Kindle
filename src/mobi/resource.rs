use std::io::Write;

use flate2::{Compression, write::ZlibEncoder};

use crate::error::Result;

pub(crate) fn serialize_shared_resource(data: Vec<u8>) -> Result<Vec<u8>> {
    if !is_sfnt_font(&data) && !is_woff_font(&data) {
        return Ok(data);
    }
    let uncompressed_length = u32::try_from(data.len())
        .map_err(|_| crate::error::Error::Output("font length exceeds u32".to_owned()))?;
    let mut compressed = Vec::new();
    let mut encoder = ZlibEncoder::new(&mut compressed, Compression::best());
    encoder.write_all(&data).map_err(|error| {
        crate::error::Error::Output(format!("FONT zlib compression failed: {error}"))
    })?;
    encoder.finish().map_err(|error| {
        crate::error::Error::Output(format!("FONT zlib finalization failed: {error}"))
    })?;

    let mut container = Vec::with_capacity(24 + compressed.len());
    container.extend_from_slice(b"FONT");
    container.extend_from_slice(&uncompressed_length.to_be_bytes());
    container.extend_from_slice(&1u32.to_be_bytes());
    container.extend_from_slice(&24u32.to_be_bytes());
    container.extend_from_slice(&0u32.to_be_bytes());
    container.extend_from_slice(&0u32.to_be_bytes());
    container.extend_from_slice(&compressed);
    Ok(container)
}

pub(crate) fn serialized_shared_resource_length(data: &[u8]) -> Result<usize> {
    if !is_sfnt_font(data) && !is_woff_font(data) {
        return Ok(data.len());
    }
    let _uncompressed_length = u32::try_from(data.len())
        .map_err(|_| crate::error::Error::Output("font length exceeds u32".to_owned()))?;
    let mut encoder = ZlibEncoder::new(CountingWriter::default(), Compression::best());
    encoder.write_all(data).map_err(|error| {
        crate::error::Error::Output(format!("FONT zlib compression failed: {error}"))
    })?;
    let compressed_length = encoder
        .finish()
        .map_err(|error| {
            crate::error::Error::Output(format!("FONT zlib finalization failed: {error}"))
        })?
        .0;
    24usize
        .checked_add(compressed_length)
        .ok_or_else(|| crate::error::Error::Output("FONT container length overflow".to_owned()))
}

#[derive(Default)]
struct CountingWriter(usize);

impl Write for CountingWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(data.len())
            .ok_or_else(|| std::io::Error::other("FONT compressed length overflow"))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn is_sfnt_font(data: &[u8]) -> bool {
    matches!(
        data.get(..4),
        Some(b"OTTO") | Some(b"true") | Some(b"ttcf") | Some([0, 1, 0, 0])
    )
}

fn is_woff_font(data: &[u8]) -> bool {
    data.starts_with(b"wOFF")
}
