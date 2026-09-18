use std::collections::{HashMap, HashSet};

use std::io::Cursor;
use std::path::Path;

use super::super::package_archive::{BoundedZipArchive, read_zip_entry};
use super::super::xhtml::{sanitize_unsupported_xhtml, validate_local_resource_paths};
use super::manifest::{
    MAX_AMAZON_HTML_BYTES, ManifestIdIndex, is_html_content_document, is_pronunciation_lexicon,
    is_unsupported_binary_media_type, resolve_binary_fallback,
};
use super::resolve_href;
use crate::book::{Resource, Resources};
use crate::error::{Error, Result};
use crate::xhtml::path::{is_external_reference, normalize_path_lossy as normalize_path};
use crate::{WarningCode, WarningCollector};

pub(super) struct LoadedResources {
    pub(super) resources: Resources,
    pub(super) xhtml: HashMap<String, String>,
    pub(super) dropped_css_hrefs: HashSet<String>,
}

pub(super) fn load_resources(
    archive: &mut BoundedZipArchive<Cursor<&[u8]>>,
    parsed: &super::super::opf::ParsedOpf,
    base: &Path,
    font_obfuscation_keys: &HashMap<String, [u8; 20]>,
    content_source_ids: &HashSet<String>,
    manifest_id_index: &ManifestIdIndex,
    warnings: &mut WarningCollector,
) -> Result<LoadedResources> {
    let mut resources = Resources::default();
    let mut xhtml = HashMap::<String, String>::new();
    let mut dropped_css_hrefs = HashSet::new();
    let spine_ids = parsed
        .spine
        .iter()
        .map(|item| item.idref.as_str())
        .collect::<HashSet<_>>();
    for item in &parsed.manifest {
        if is_pronunciation_lexicon(&item.media_type) {
            if !is_external_reference(&item.href) {
                let path = resolve_href(base, &item.href);
                if path.is_empty() {
                    return Err(Error::InvalidEpub(format!(
                        "manifest item {} has a path that escapes the EPUB root",
                        item.id
                    )));
                }
                // Pronunciation lexicons remain available to source XHTML, but
                // Kindle does not consume the PLS payload as a transport resource.
                // Check local archive entries before omitting them from Book.resources.
                archive.entry_size(&path)?;
            }
            warnings.add_once(
                WarningCode::W002,
                "PLS pronunciation lexicon semantics were dropped for Kindle output",
            );
            continue;
        }
        if is_adobe_page_template(&item.media_type) {
            if !is_external_reference(&item.href) {
                let path = resolve_href(base, &item.href);
                if path.is_empty() {
                    return Err(Error::InvalidEpub(format!(
                        "manifest item {} has a path that escapes the EPUB root",
                        item.id
                    )));
                }
                // XPGT is unsupported by Kindle, but still validate its local
                // archive entry before omitting it from Kindle resources.
                archive.entry_size(&path)?;
            }
            warnings.add_category_once(
                WarningCode::W004,
                "Adobe XPGT page-template semantics were dropped for Kindle output",
            );
            continue;
        }
        if is_external_reference(&item.href) {
            // Remote resources are not EPUB ZIP entries. Preserve the
            // external reference in source documents/CSS, but do not turn it
            // into an accidental ZIP path lookup failure. The
            // `remote-resources` property is therefore an explicit safe
            // omission boundary for resources that cannot be packaged.
            continue;
        }
        let binary_fallback = if !spine_ids.contains(item.id.as_str())
            && is_unsupported_binary_media_type(&item.media_type)
            && item.fallback.is_some()
        {
            Some(resolve_binary_fallback(
                item,
                &parsed.manifest,
                manifest_id_index,
            )?)
        } else {
            None
        };
        if binary_fallback.is_some() {
            warnings.add_category_once(
                WarningCode::W002,
                "an unsupported manifest resource used its EPUB binary fallback",
            );
        }
        let source_item = binary_fallback.unwrap_or(item);
        let path = resolve_href(base, &source_item.href);
        if path.is_empty() {
            return Err(Error::InvalidEpub(format!(
                "manifest item {} has a path that escapes the EPUB root",
                source_item.id
            )));
        }
        if is_html_content_document(source_item) {
            let size = archive.entry_size(&path)?;
            if size >= MAX_AMAZON_HTML_BYTES {
                warnings.add_category_once(
                    WarningCode::W006,
                    "one or more HTML/XHTML content documents meet or exceed Amazon's 30,000,000-byte publishing guidance",
                );
            }
        }
        let mut data = read_zip_entry(archive, &path)?;
        if let Some(key) = font_obfuscation_keys.get(&path) {
            super::super::font_obfuscation::deobfuscate_font(&mut data, key);
        }
        let resource_data = if source_item
            .media_type
            .eq_ignore_ascii_case("application/xhtml+xml")
            || source_item.media_type.eq_ignore_ascii_case("text/html")
        {
            let source = decode_text_entry(&data, TextKind::Xhtml)?;
            validate_local_resource_paths(&source, &path)?;
            let source = if content_source_ids.contains(source_item.id.as_str()) {
                sanitize_unsupported_xhtml(&source, warnings)?
            } else {
                source
            };
            if content_source_ids.contains(source_item.id.as_str()) {
                xhtml.insert(source_item.id.clone(), source);
            }
            // ContentDocument owns the parsed XHTML source. XHTML resources
            // are routing metadata only after parsing and are never emitted
            // as binary KF8 resources, so do not retain a second full byte
            // buffer in Book.resources.
            Vec::new()
        } else if source_item.media_type.eq_ignore_ascii_case("text/css") {
            let source = match decode_text_entry(&data, TextKind::Css) {
                Ok(source) => source,
                Err(Error::InvalidEpub(message))
                    if message.starts_with("invalid UTF-8 CSS entry:") =>
                {
                    dropped_css_hrefs.insert(normalize_path(&item.href));
                    warnings.add_category_once(
                        WarningCode::W004,
                        "a stylesheet with invalid UTF-8 was dropped for Kindle output",
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            // Downstream CSS flows consume the one canonical UTF-8 form.
            source.into_bytes()
        } else if source_item.media_type.eq_ignore_ascii_case("image/svg+xml")
            && content_source_ids.contains(source_item.id.as_str())
        {
            let source = decode_text_entry(&data, TextKind::Xhtml)?;
            validate_local_resource_paths(&source, &path)?;
            let source = sanitize_unsupported_xhtml(&source, warnings)?;
            xhtml.insert(source_item.id.clone(), svg_content_document(&source)?);
            Vec::new()
        } else {
            data
        };
        resources.items.push(Resource {
            id: item.id.clone(),
            href: item.href.clone(),
            media_type: source_item.media_type.clone(),
            properties: item.properties.clone(),
            data: resource_data,
        });
    }
    Ok(LoadedResources {
        resources,
        xhtml,
        dropped_css_hrefs,
    })
}

fn is_adobe_page_template(media_type: &str) -> bool {
    media_type.eq_ignore_ascii_case("application/adobe-page-template+xml")
}

fn svg_content_document(source: &str) -> Result<String> {
    let source = source.trim_start_matches('\u{feff}');
    let source = strip_svg_prolog(source);
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>SVG Content Document</title></head><body><div class="epub-svg-content">{source}</div></body></html>"#
    ))
}

fn strip_svg_prolog(source: &str) -> &str {
    let mut source = source.trim_start();
    if source.starts_with("<?xml") {
        if let Some(end) = source.find("?>") {
            source = &source[end + 2..];
        }
    }
    source.trim_start()
}

#[derive(Debug, Clone, Copy)]
enum TextKind {
    Xhtml,
    Css,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

fn decode_text_entry(data: &[u8], kind: TextKind) -> Result<String> {
    let (encoding, offset) = if data.starts_with(&[0xef, 0xbb, 0xbf]) {
        (TextEncoding::Utf8, 3)
    } else if data.starts_with(&[0xff, 0xfe]) {
        (TextEncoding::Utf16Le, 2)
    } else if data.starts_with(&[0xfe, 0xff]) {
        (TextEncoding::Utf16Be, 2)
    } else if looks_like_utf16le(data) {
        (TextEncoding::Utf16Le, 0)
    } else if looks_like_utf16be(data) {
        (TextEncoding::Utf16Be, 0)
    } else {
        (TextEncoding::Utf8, 0)
    };
    let source = match encoding {
        TextEncoding::Utf8 => std::str::from_utf8(&data[offset..])
            .map(str::to_owned)
            .map_err(|error| {
                Error::InvalidEpub(format!(
                    "invalid UTF-8 {} entry: {error}",
                    text_kind_name(kind)
                ))
            })?,
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            if (data.len() - offset) % 2 != 0 {
                return Err(Error::InvalidEpub(format!(
                    "odd-length UTF-16 {} entry",
                    text_kind_name(kind)
                )));
            }
            let little_endian = encoding == TextEncoding::Utf16Le;
            let units = data[offset..].chunks_exact(2).map(|pair| {
                if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            });
            char::decode_utf16(units)
                .collect::<std::result::Result<String, _>>()
                .map_err(|error| {
                    Error::InvalidEpub(format!(
                        "invalid UTF-16 {} entry: {error}",
                        text_kind_name(kind)
                    ))
                })?
        }
    };
    validate_declared_encoding(&source, kind, encoding)?;
    Ok(source)
}

fn looks_like_utf16le(data: &[u8]) -> bool {
    data.len() >= 4 && data[1] == 0 && data[3] == 0 && data[0] != 0 && data[2] != 0
}

fn looks_like_utf16be(data: &[u8]) -> bool {
    data.len() >= 4 && data[0] == 0 && data[2] == 0 && data[1] != 0 && data[3] != 0
}

fn text_kind_name(kind: TextKind) -> &'static str {
    match kind {
        TextKind::Xhtml => "XHTML",
        TextKind::Css => "CSS",
    }
}

fn validate_declared_encoding(source: &str, kind: TextKind, actual: TextEncoding) -> Result<()> {
    let declaration = match kind {
        TextKind::Xhtml => find_quoted_encoding(source, "encoding"),
        TextKind::Css => find_css_charset(source),
    };
    let Some(declaration) = declaration else {
        return Ok(());
    };
    let normalized_declaration = declaration
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], "");
    let declared = normalize_encoding_name(&declaration).ok_or_else(|| {
        Error::InvalidEpub(format!(
            "unsupported {} encoding declaration {declaration}",
            text_kind_name(kind)
        ))
    })?;
    let generic_utf16 = normalized_declaration == "utf16"
        && matches!(actual, TextEncoding::Utf16Le | TextEncoding::Utf16Be);
    if !generic_utf16 && declared != actual {
        return Err(Error::InvalidEpub(format!(
            "{} encoding declaration {declaration} does not match decoded bytes",
            text_kind_name(kind)
        )));
    }
    Ok(())
}

fn normalize_encoding_name(value: &str) -> Option<TextEncoding> {
    match value
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], "")
        .as_str()
    {
        "utf8" => Some(TextEncoding::Utf8),
        "utf16" | "utf16le" => Some(TextEncoding::Utf16Le),
        "utf16be" => Some(TextEncoding::Utf16Be),
        _ => None,
    }
}

fn find_quoted_encoding(source: &str, key: &str) -> Option<String> {
    let prefix = source.get(..source.len().min(1024))?;
    let lower = prefix.to_ascii_lowercase();
    let start = lower.find(key)? + key.len();
    let remainder = &prefix[start..];
    let quote = remainder.find(['\'', '"'])?;
    let quoted = &remainder[quote + 1..];
    let end = quoted.find(remainder.as_bytes()[quote] as char)?;
    Some(quoted[..end].to_owned())
}

fn find_css_charset(source: &str) -> Option<String> {
    let prefix = source
        .trim_start()
        .get(..source.trim_start().len().min(256))?;
    let lower = prefix.to_ascii_lowercase();
    let start = lower.find("@charset")? + "@charset".len();
    let remainder = &prefix[start..];
    let quote = remainder.find(['\'', '"'])?;
    let quoted = &remainder[quote + 1..];
    let end = quoted.find(remainder.as_bytes()[quote] as char)?;
    Some(quoted[..end].to_owned())
}
