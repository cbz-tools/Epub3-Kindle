//! EPUB font obfuscation and encryption.xml handling.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use crate::error::{Error, Result};
use crate::xhtml::path::normalize_path_lossy as normalize_path;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;

use super::package::resolve_href;
use super::package_archive::{BoundedZipArchive, read_zip_entry};

const IDPF_FONT_OBFUSCATION: &str = "http://www.idpf.org/2008/embedding";

#[derive(Debug, Clone)]
struct EncryptionTarget {
    algorithm: String,
    uri: String,
}

pub(super) fn load_font_obfuscation<R: Read + std::io::Seek>(
    archive: &mut BoundedZipArchive<R>,
    parsed: &super::opf::ParsedOpf,
    opf_base: &Path,
) -> Result<HashMap<String, [u8; 20]>> {
    if !archive.contains("META-INF/encryption.xml") {
        return Ok(HashMap::new());
    }
    let encryption = read_zip_entry(archive, "META-INF/encryption.xml")?;
    let targets = parse_encryption_xml(&encryption)?;
    let mut keys = HashMap::new();
    for target in targets {
        if target.algorithm != IDPF_FONT_OBFUSCATION {
            return Err(Error::UnsupportedEpub(format!(
                "unsupported encryption algorithm {} for {}",
                target.algorithm, target.uri
            )));
        }
        let normalized_target =
            normalize_path(target.uri.split(['#', '?']).next().unwrap_or_default());
        let Some(item) = parsed
            .manifest
            .iter()
            .find(|item| resolve_href(opf_base, &item.href) == normalized_target)
        else {
            return Err(Error::InvalidEpub(format!(
                "encryption target {} is not a manifest resource",
                target.uri
            )));
        };
        if !is_font_media_type(&item.media_type) {
            return Err(Error::UnsupportedEpub(format!(
                "IDPF font obfuscation target {} is not a font resource",
                target.uri
            )));
        }
        let path = normalized_target;
        // Validate the ZIP target without decompressing it a second time;
        // resource loading performs the bounded decode below.
        if !archive.contains(&path) {
            return Err(Error::InvalidEpub(format!(
                "missing encrypted resource {path}"
            )));
        }
        let key = unique_identifier_key(parsed)?;
        if keys.insert(path.clone(), key).is_some() {
            return Err(Error::InvalidEpub(format!(
                "duplicate encryption target {path}"
            )));
        }
    }
    Ok(keys)
}

fn parse_encryption_xml(xml: &[u8]) -> Result<Vec<EncryptionTarget>> {
    let mut reader = NsReader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut state = EncryptionXmlState::default();
    let mut stack = Vec::<OpenEncryptionElement>::new();
    let mut targets = Vec::new();
    let mut root_seen = false;
    loop {
        let (namespace, event) = reader.read_resolved_event_into(&mut buffer)?;
        match event {
            Event::Start(event) => {
                validate_encryption_attributes(&event)?;
                let kind =
                    begin_encryption_element(namespace, &event, &stack, root_seen, &mut state)?;
                if kind == EncryptionElement::Root {
                    root_seen = true;
                }
                stack.push(OpenEncryptionElement {
                    kind,
                    qname: event.name().as_ref().to_vec(),
                });
            }
            Event::Empty(event) => {
                validate_encryption_attributes(&event)?;
                let kind =
                    begin_encryption_element(namespace, &event, &stack, root_seen, &mut state)?;
                if kind == EncryptionElement::Root {
                    root_seen = true;
                } else if kind == EncryptionElement::EncryptedData {
                    finish_encrypted_data(&mut state, &mut targets)?;
                }
            }
            Event::End(event) => {
                let kind = encryption_element_kind(namespace, event.name().local_name().as_ref())
                    .ok_or_else(|| {
                    Error::InvalidEpub(format!(
                        "encryption.xml has an element in an unexpected namespace: {}",
                        String::from_utf8_lossy(event.name().as_ref())
                    ))
                })?;
                let open = stack.pop().ok_or_else(|| {
                    Error::InvalidEpub(format!(
                        "encryption.xml has an unmatched {} end tag",
                        String::from_utf8_lossy(event.name().as_ref())
                    ))
                })?;
                if open.kind != kind || open.qname != event.name().as_ref() {
                    return Err(Error::InvalidEpub(format!(
                        "encryption.xml closes {} while {} is open",
                        String::from_utf8_lossy(event.name().as_ref()),
                        String::from_utf8_lossy(&open.qname)
                    )));
                }
                if kind == EncryptionElement::EncryptedData {
                    finish_encrypted_data(&mut state, &mut targets)?;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if state.current.is_some() || !stack.is_empty() || !root_seen {
        return Err(Error::InvalidEpub(
            "encryption.xml has an unterminated or missing root element".to_owned(),
        ));
    }
    if targets.is_empty() {
        return Err(Error::InvalidEpub(
            "encryption.xml contains no EncryptedData targets".to_owned(),
        ));
    }
    Ok(targets)
}

const OCF_CONTAINER_NAMESPACE: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:container";
const XML_ENCRYPTION_NAMESPACE: &[u8] = b"http://www.w3.org/2001/04/xmlenc#";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EncryptionElement {
    Root,
    EncryptedData,
    EncryptionMethod,
    CipherData,
    CipherReference,
}

#[derive(Debug)]
struct OpenEncryptionElement {
    kind: EncryptionElement,
    qname: Vec<u8>,
}

#[derive(Default)]
struct EncryptionXmlState {
    current: Option<EncryptionTarget>,
    encryption_method_count: usize,
    cipher_data_count: usize,
    cipher_reference_count: usize,
}

fn encryption_element_kind(
    namespace: ResolveResult<'_>,
    local_name: &[u8],
) -> Option<EncryptionElement> {
    match namespace {
        ResolveResult::Bound(Namespace(uri)) if uri == OCF_CONTAINER_NAMESPACE => {
            (local_name == b"encryption").then_some(EncryptionElement::Root)
        }
        ResolveResult::Bound(Namespace(uri)) if uri == XML_ENCRYPTION_NAMESPACE => match local_name
        {
            b"EncryptedData" => Some(EncryptionElement::EncryptedData),
            b"EncryptionMethod" => Some(EncryptionElement::EncryptionMethod),
            b"CipherData" => Some(EncryptionElement::CipherData),
            b"CipherReference" => Some(EncryptionElement::CipherReference),
            _ => None,
        },
        _ => None,
    }
}

fn begin_encryption_element(
    namespace: ResolveResult<'_>,
    event: &quick_xml::events::BytesStart<'_>,
    stack: &[OpenEncryptionElement],
    root_seen: bool,
    state: &mut EncryptionXmlState,
) -> Result<EncryptionElement> {
    let kind = match encryption_element_kind(namespace, event.name().local_name().as_ref()) {
        Some(kind) => kind,
        None if stack.is_empty() => {
            return Err(Error::InvalidEpub(
                "encryption.xml root element must be encryption in the OCF container namespace"
                    .to_owned(),
            ));
        }
        None => {
            return Err(Error::InvalidEpub(format!(
                "encryption.xml element {} has an unexpected namespace or name",
                String::from_utf8_lossy(event.name().as_ref())
            )));
        }
    };
    match (stack.last().map(|element| element.kind), kind) {
        (None, EncryptionElement::Root) if !root_seen => {}
        (None, _) => {
            return Err(Error::InvalidEpub(
                "encryption.xml root element must be encryption in the OCF container namespace"
                    .to_owned(),
            ));
        }
        (Some(EncryptionElement::Root), EncryptionElement::EncryptedData) => {}
        (Some(EncryptionElement::EncryptedData), EncryptionElement::EncryptionMethod)
        | (Some(EncryptionElement::EncryptedData), EncryptionElement::CipherData) => {}
        (Some(EncryptionElement::CipherData), EncryptionElement::CipherReference) => {}
        (Some(parent), child) => {
            return Err(Error::InvalidEpub(format!(
                "encryption.xml {} has unexpected {} child; exact hierarchy is required",
                encryption_element_label(parent),
                encryption_element_label(child)
            )));
        }
    }
    match kind {
        EncryptionElement::Root => {}
        EncryptionElement::EncryptedData => {
            if encryption_xml_attr(event, b"Algorithm").is_some() {
                return Err(Error::InvalidEpub(
                    "encryption.xml EncryptedData must use a child EncryptionMethod".to_owned(),
                ));
            }
            state.current = Some(EncryptionTarget {
                algorithm: String::new(),
                uri: String::new(),
            });
            state.encryption_method_count = 0;
            state.cipher_data_count = 0;
            state.cipher_reference_count = 0;
        }
        EncryptionElement::EncryptionMethod => {
            if state.encryption_method_count != 0 {
                return Err(Error::InvalidEpub(
                    "encryption.xml EncryptedData has multiple EncryptionMethod elements"
                        .to_owned(),
                ));
            }
            let algorithm = encryption_xml_attr(event, b"Algorithm")
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    Error::InvalidEpub(
                        "encryption.xml EncryptionMethod has no Algorithm".to_owned(),
                    )
                })?;
            state
                .current
                .as_mut()
                .expect("validated EncryptedData parent")
                .algorithm = algorithm;
            state.encryption_method_count = 1;
        }
        EncryptionElement::CipherData => {
            if state.cipher_data_count != 0 {
                return Err(Error::InvalidEpub(
                    "encryption.xml EncryptedData has multiple CipherData elements".to_owned(),
                ));
            }
            state.cipher_data_count = 1;
        }
        EncryptionElement::CipherReference => {
            if state.cipher_reference_count != 0 {
                return Err(Error::InvalidEpub(
                    "encryption.xml EncryptedData has multiple CipherReference elements".to_owned(),
                ));
            }
            let uri = encryption_xml_attr(event, b"URI")
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    Error::InvalidEpub("encryption.xml CipherReference has no URI".to_owned())
                })?;
            state
                .current
                .as_mut()
                .expect("validated CipherData parent")
                .uri = uri;
            state.cipher_reference_count = 1;
        }
    }
    Ok(kind)
}

fn finish_encrypted_data(
    state: &mut EncryptionXmlState,
    targets: &mut Vec<EncryptionTarget>,
) -> Result<()> {
    let entry = state.current.take().ok_or_else(|| {
        Error::InvalidEpub("encryption.xml has an unmatched EncryptedData end tag".to_owned())
    })?;
    if state.encryption_method_count != 1
        || state.cipher_data_count != 1
        || state.cipher_reference_count != 1
    {
        return Err(Error::InvalidEpub(
            "encryption.xml EncryptedData must contain exactly one EncryptionMethod, CipherData, and CipherReference"
                .to_owned(),
        ));
    }
    targets.push(entry);
    Ok(())
}

fn encryption_element_label(element: EncryptionElement) -> &'static str {
    match element {
        EncryptionElement::Root => "encryption",
        EncryptionElement::EncryptedData => "EncryptedData",
        EncryptionElement::EncryptionMethod => "EncryptionMethod",
        EncryptionElement::CipherData => "CipherData",
        EncryptionElement::CipherReference => "CipherReference",
    }
}

fn validate_encryption_attributes(event: &quick_xml::events::BytesStart<'_>) -> Result<()> {
    for attribute in event.attributes() {
        attribute.map_err(|error| {
            Error::InvalidEpub(format!("encryption.xml has a malformed attribute: {error}"))
        })?;
    }
    Ok(())
}

fn encryption_xml_attr(event: &quick_xml::events::BytesStart<'_>, wanted: &[u8]) -> Option<String> {
    event.attributes().flatten().find_map(|attribute| {
        (attribute.key.as_ref() == wanted).then(|| {
            attribute
                .unescape_value()
                .ok()
                .map(|value| value.into_owned())
        })
    })?
}

fn unique_identifier_key(parsed: &super::opf::ParsedOpf) -> Result<[u8; 20]> {
    let id = parsed.unique_identifier_id.as_deref().ok_or_else(|| {
        Error::InvalidEpub("encrypted fonts require package unique-identifier attribute".to_owned())
    })?;
    let records = parsed
        .metadata
        .records
        .iter()
        .filter(|record| {
            record.id.as_deref() == Some(id)
                && record
                    .property
                    .rsplit(':')
                    .next()
                    .is_some_and(|property| property.eq_ignore_ascii_case("identifier"))
                && record.refines.is_none()
        })
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return Err(Error::InvalidEpub(format!(
            "package unique-identifier {id} does not resolve to one dc:identifier"
        )));
    }
    let normalized = records[0]
        .value
        .chars()
        .filter(|character| !matches!(character, ' ' | '\t' | '\r' | '\n'))
        .collect::<String>();
    if normalized.is_empty() {
        return Err(Error::InvalidEpub(
            "package unique identifier is empty".to_owned(),
        ));
    }
    Ok(sha1_digest(normalized.as_bytes()))
}

pub(super) fn deobfuscate_font(source: &mut [u8], key: &[u8; 20]) {
    for (index, byte) in source.iter_mut().take(1040).enumerate() {
        *byte ^= key[index % key.len()];
    }
}

fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut message = input.to_vec();
    let bit_length = (message.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    let mut state = [
        0x67452301u32,
        0xEFCDAB89,
        0x98BADCFE,
        0x10325476,
        0xC3D2E1F0,
    ];
    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes(chunk[start..start + 4].try_into().unwrap());
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = tuple5(state);
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }
    let mut digest = [0u8; 20];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn tuple5(values: [u32; 5]) -> (u32, u32, u32, u32, u32) {
    (values[0], values[1], values[2], values[3], values[4])
}

fn is_font_media_type(media_type: &str) -> bool {
    matches!(
        media_type.to_ascii_lowercase().as_str(),
        "application/font-woff"
            | "font/woff"
            | "application/font-sfnt"
            | "application/vnd.ms-opentype"
            | "application/x-font-opentype"
            | "application/x-font-ttf"
            | "font/sfnt"
            | "font/ttf"
            | "font/otf"
            | "font/opentype"
            | "font/truetype"
    )
}
