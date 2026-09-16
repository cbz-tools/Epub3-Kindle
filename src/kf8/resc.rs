//! Encode the KindleGen-observed RESC resource/spine projection.
//!
//! The binary prefix, text header, and 4096-byte payload shape are based on
//! KINDLEGEN-FINAL/REFERENCE evidence, not an Amazon official specification.

use crate::book::{RenditionFlow, RenditionOrientation, RenditionSemantics, RenditionSpread};
use crate::error::{Error, Result};
use crate::kindle::KindleSection;

const RESC_PAYLOAD_SIZE: usize = 4096;
const RESC_PREFIX_SIZE: usize = 16;

/// Serialize the source-spine projection used by KindleGen's RESC record.
///
/// Synthetic sections have no source spine index and are intentionally omitted.
/// The remaining sections are ordered by their original spine position while
/// their `skelid` values refer to the emitted section/SKEL entry index.
pub(crate) fn encode(
    sections: &[KindleSection],
    publication_rendition: RenditionSemantics,
    rendition_viewport: Option<&str>,
) -> Result<Vec<u8>> {
    let mut entries = sections
        .iter()
        .enumerate()
        .filter_map(|(skelid, section)| {
            section
                .source_spine_index
                .map(|spine_index| (spine_index, skelid, section))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|(spine_index, _, _)| *spine_index);

    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0">"#,
    );
    if publication_rendition.orientation.is_some()
        || publication_rendition.spread.is_some()
        || publication_rendition.flow.is_some()
        || rendition_viewport.is_some()
    {
        xml.push_str("<metadata>");
        if let Some(orientation) = publication_rendition.orientation {
            xml.push_str(r#"<meta property="rendition:orientation">"#);
            push_xml_attribute(&mut xml, rendition_orientation_value(orientation));
            xml.push_str("</meta>");
        }
        if let Some(spread) = publication_rendition.spread {
            xml.push_str(r#"<meta property="rendition:spread">"#);
            push_xml_attribute(&mut xml, rendition_spread_value(spread));
            xml.push_str("</meta>");
        }
        if let Some(flow) = publication_rendition.flow {
            xml.push_str(r#"<meta property="rendition:flow">"#);
            push_xml_attribute(&mut xml, rendition_flow_value(flow));
            xml.push_str("</meta>");
        }
        if let Some(viewport) = rendition_viewport {
            xml.push_str(r#"<meta property="rendition:viewport">"#);
            push_xml_attribute(&mut xml, viewport);
            xml.push_str("</meta>");
        }
        xml.push_str("</metadata>");
    }
    xml.push_str("<spine>");
    for (_, skelid, section) in entries {
        xml.push_str("<itemref idref=\"");
        push_xml_attribute(&mut xml, &section.id);
        xml.push('"');
        if !section.source_properties.is_empty() {
            xml.push_str(" properties=\"");
            push_xml_attribute(&mut xml, &section.source_properties.join(" "));
            xml.push('"');
        }
        xml.push_str(" skelid=\"");
        xml.push_str(&skelid.to_string());
        xml.push_str("\" linear=\"");
        xml.push_str(if section.linear { "yes" } else { "no" });
        xml.push_str("\"/>");
    }
    xml.push_str("</spine></package>");
    let xml = xml.into_bytes();
    let xml_length = u32::try_from(xml.len())
        .map_err(|_| Error::Output("RESC XML length exceeds u32".to_owned()))?;
    let header = format!("size={}&version=1&type=1", resc_base32(xml_length));
    let payload_length = header
        .len()
        .checked_add(xml.len())
        .ok_or_else(|| Error::Output("RESC payload length overflow".to_owned()))?;
    let payload_blocks = payload_length
        .checked_add(RESC_PAYLOAD_SIZE - 1)
        .ok_or_else(|| Error::Output("RESC payload block count overflow".to_owned()))?
        / RESC_PAYLOAD_SIZE;
    let payload_capacity = payload_blocks
        .checked_mul(RESC_PAYLOAD_SIZE)
        .ok_or_else(|| Error::Output("RESC payload capacity overflow".to_owned()))?;

    let mut record = vec![0u8; RESC_PREFIX_SIZE + payload_capacity];
    record[0..4].copy_from_slice(b"RESC");
    record[4..8].copy_from_slice(&0x10u32.to_be_bytes());
    record[8..12].copy_from_slice(&1u32.to_be_bytes());
    let header_length = u32::try_from(header.len())
        .map_err(|_| Error::Output("RESC header length exceeds u32".to_owned()))?;
    record[12..16].copy_from_slice(&header_length.to_be_bytes());
    record[16..16 + header.len()].copy_from_slice(header.as_bytes());
    record[16 + header.len()..16 + payload_length].copy_from_slice(&xml);
    Ok(record)
}

fn push_xml_attribute(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            character => output.push(character),
        }
    }
}

fn rendition_spread_value(value: RenditionSpread) -> &'static str {
    match value {
        RenditionSpread::Auto => "auto",
        RenditionSpread::None => "none",
        RenditionSpread::Landscape => "landscape",
        RenditionSpread::Portrait => "portrait",
        RenditionSpread::Both => "both",
    }
}

fn rendition_orientation_value(value: RenditionOrientation) -> &'static str {
    match value {
        RenditionOrientation::Auto => "auto",
        RenditionOrientation::Portrait => "portrait",
        RenditionOrientation::Landscape => "landscape",
    }
}

fn rendition_flow_value(value: RenditionFlow) -> &'static str {
    match value {
        RenditionFlow::Auto => "auto",
        RenditionFlow::Paginated => "paginated",
        RenditionFlow::ScrolledContinuous => "scrolled-continuous",
        RenditionFlow::ScrolledDoc => "scrolled-doc",
    }
}

// RESC's size field uses the variable-width Kindle base32 spelling observed
// in KindleGen output (`1HA`, not the fixed-width embed spelling `01HA`).
fn resc_base32(mut value: u32) -> String {
    const DIGITS: &[u8; 32] = b"0123456789ABCDEFGHIJKLMNOPQRSTUV";
    let mut digits = Vec::new();
    while value != 0 {
        digits.push(DIGITS[(value % 32) as usize]);
        value /= 32;
    }
    if digits.is_empty() {
        digits.push(b'0');
    }
    digits.reverse();
    String::from_utf8(digits).expect("RESC base32 alphabet is ASCII")
}
