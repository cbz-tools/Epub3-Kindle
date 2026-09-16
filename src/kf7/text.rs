use crate::error::Result;

pub(crate) const STUB_HTML: &[u8] = b"<html><head></head><body>stub</body></html>";
pub(crate) const EOF: &[u8] = b"\xe9\x8e\r\n";

pub(crate) fn compress_stub(source: &[u8]) -> Vec<u8> {
    let mut compressor = crate::kf8::PalmDocCompressor::new();
    let mut encoded = Vec::new();
    compressor.compress(source, &mut encoded);
    encoded
}

pub(crate) fn decode_palm_doc(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let byte = input[cursor];
        cursor += 1;
        match byte {
            0x00..=0x08 => {
                let literal_length = usize::from(byte);
                let end = cursor.checked_add(literal_length).ok_or_else(|| {
                    crate::error::Error::Output("legacy PalmDOC literal overflow".to_owned())
                })?;
                let literal = input.get(cursor..end).ok_or_else(|| {
                    crate::error::Error::Output("legacy PalmDOC literal is truncated".to_owned())
                })?;
                decoded.extend_from_slice(literal);
                cursor = end;
            }
            0x09..=0x7f => decoded.push(byte),
            0x80..=0xbf => {
                let next = *input.get(cursor).ok_or_else(|| {
                    crate::error::Error::Output(
                        "legacy PalmDOC back-reference is truncated".to_owned(),
                    )
                })?;
                cursor += 1;
                let distance = (usize::from(byte & 0x3f) << 5) | usize::from(next >> 3);
                let length = usize::from(next & 0x07) + 3;
                if distance == 0 || distance > decoded.len() {
                    return Err(crate::error::Error::Output(
                        "legacy PalmDOC back-reference is invalid".to_owned(),
                    ));
                }
                for _ in 0..length {
                    let source = decoded.len() - distance;
                    let value = *decoded.get(source).ok_or_else(|| {
                        crate::error::Error::Output(
                            "legacy PalmDOC back-reference exceeds decoded data".to_owned(),
                        )
                    })?;
                    decoded.push(value);
                }
            }
            0xc0..=0xff => {
                decoded.push(b' ');
                decoded.push(byte ^ 0x80);
            }
        }
    }
    Ok(decoded)
}
