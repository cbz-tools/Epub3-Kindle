use std::collections::{HashMap, HashSet};

/// Amazon accepts individual HTML/XHTML content documents strictly below
/// 30,000,000 decimal bytes, and fewer than 300 such documents per publication.
pub(super) const MAX_AMAZON_HTML_BYTES: u64 = 30_000_000;
const MAX_AMAZON_HTML_DOCUMENTS: usize = 300;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::super::opf::{ManifestItem, has_property, parse_opf, parse_rootfile};
use super::super::package_archive::{BoundedZipArchive, read_zip_entry, validate_ocf_paths};
use crate::book::{RenditionAlign, RenditionSemantics};
use crate::error::{Error, Result};
use crate::xhtml::path::{is_external_reference, resolve_path};
use crate::{WarningCode, WarningCollector};

pub(super) struct LoadedPackage<'a> {
    pub(super) archive: BoundedZipArchive<Cursor<&'a [u8]>>,
    pub(super) parsed: super::super::opf::ParsedOpf,
    pub(super) base: PathBuf,
    pub(super) manifest_id_index: ManifestIdIndex,
    pub(super) font_obfuscation_keys: HashMap<String, [u8; 20]>,
    pub(super) content_source_ids: HashSet<String>,
    pub(super) spine_content_source_indices: Vec<Option<usize>>,
}

pub(super) struct ManifestIdIndex {
    positions: HashMap<String, usize>,
}

impl ManifestIdIndex {
    pub(super) fn from_manifest(manifest: &[ManifestItem]) -> Self {
        let mut positions = HashMap::with_capacity(manifest.len());
        for (index, item) in manifest.iter().enumerate() {
            positions.entry(item.id.clone()).or_insert(index);
        }
        Self { positions }
    }

    pub(super) fn get<'a>(
        &self,
        manifest: &'a [ManifestItem],
        id: &str,
    ) -> Option<&'a ManifestItem> {
        self.index(id).map(|index| &manifest[index])
    }

    pub(super) fn index(&self, id: &str) -> Option<usize> {
        self.positions.get(id).copied()
    }
}

pub(super) fn load_package<'a>(
    input: &'a [u8],
    warnings: &mut WarningCollector,
) -> Result<LoadedPackage<'a>> {
    validate_ocf_paths(input)?;
    let mut archive = BoundedZipArchive::new(Cursor::new(input))?;
    let container = read_zip_entry(&mut archive, "META-INF/container.xml")?;
    let opf_path = parse_rootfile(&container)?;
    let opf = read_zip_entry(&mut archive, &opf_path)?;
    let parsed = parse_opf(&opf)?;
    let manifest_id_index = ManifestIdIndex::from_manifest(&parsed.manifest);
    if parsed
        .manifest
        .iter()
        .any(|item| has_property(item, "mathml"))
    {
        warnings.add_category_once(
            WarningCode::W005,
            "MathML manifest semantics were reduced to readable content",
        );
    }
    validate_rendition_semantics(&parsed)?;
    warn_unsupported_media_semantics(&parsed, &manifest_id_index, warnings)?;
    let base = Path::new(&opf_path)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    validate_manifest_paths(&parsed.manifest, &base)?;
    validate_amazon_document_count(&parsed.manifest, warnings);
    let font_obfuscation_keys =
        super::super::font_obfuscation::load_font_obfuscation(&mut archive, &parsed, &base)?;
    let mut content_source_ids = HashSet::new();
    let mut spine_content_source_indices = Vec::with_capacity(parsed.spine.len());
    for spine_item in &parsed.spine {
        let source = manifest_id_index
            .get(&parsed.manifest, &spine_item.idref)
            .ok_or_else(|| {
                Error::InvalidEpub(format!(
                    "spine references missing manifest item {}",
                    spine_item.idref
                ))
            })?;
        if let Some(content_source) = resolve_content_source_for_spine(
            source,
            &parsed.manifest,
            &manifest_id_index,
            warnings,
        )? {
            content_source_ids.insert(content_source.id.clone());
            spine_content_source_indices.push(manifest_id_index.index(&content_source.id));
        } else {
            spine_content_source_indices.push(None);
        }
    }
    Ok(LoadedPackage {
        archive,
        parsed,
        base,
        manifest_id_index,
        font_obfuscation_keys,
        content_source_ids,
        spine_content_source_indices,
    })
}

fn warn_unsupported_media_semantics(
    parsed: &super::super::opf::ParsedOpf,
    manifest_id_index: &ManifestIdIndex,
    warnings: &mut WarningCollector,
) -> Result<()> {
    for item in &parsed.manifest {
        if item.media_overlay.is_some() || has_property(item, "media-overlay") {
            warnings.add_category_once(
                WarningCode::W003,
                "media-overlay playback semantics were dropped while preserving the XHTML body",
            );
        }
    }
    for spine_item in &parsed.spine {
        if spine_item.media_overlay.is_some() {
            warnings.add_category_once(
                WarningCode::W003,
                "media-overlay playback semantics were dropped while preserving the XHTML body",
            );
        }
        let item = manifest_id_index
            .get(&parsed.manifest, &spine_item.idref)
            .ok_or_else(|| {
                Error::InvalidEpub(format!(
                    "spine references missing manifest item {}",
                    spine_item.idref
                ))
            })?;
        if is_audio_media_type(&item.media_type) {
            warnings.add_category_once(
                WarningCode::W003,
                "spine audio playback was dropped without promoting the audio resource to a content document",
            );
        }
        if is_video_media_type(&item.media_type) {
            warnings.add_category_once(
                WarningCode::W003,
                "spine video playback was dropped without promoting the video resource to a content document",
            );
        }
        if is_smil_media_type(&item.media_type) {
            warnings.add_category_once(
                WarningCode::W003,
                "SMIL media-overlay playback was dropped without promoting it to a content document",
            );
        }
    }
    Ok(())
}

fn is_audio_media_type(media_type: &str) -> bool {
    media_type.eq_ignore_ascii_case("audio/mpeg")
        || media_type.eq_ignore_ascii_case("audio/mp4")
        || media_type.eq_ignore_ascii_case("audio/ogg")
        || media_type.eq_ignore_ascii_case("audio/wav")
        || media_type.eq_ignore_ascii_case("audio/webm")
        || media_type.to_ascii_lowercase().starts_with("audio/")
}

fn is_video_media_type(media_type: &str) -> bool {
    media_type.to_ascii_lowercase().starts_with("video/")
}

fn is_smil_media_type(media_type: &str) -> bool {
    media_type.eq_ignore_ascii_case("application/smil+xml")
}

pub(super) fn is_pronunciation_lexicon(media_type: &str) -> bool {
    media_type.eq_ignore_ascii_case("application/pls+xml")
}

fn validate_rendition_semantics(parsed: &super::super::opf::ParsedOpf) -> Result<()> {
    validate_rendition("publication", parsed.rendition)?;
    for item in &parsed.spine {
        validate_rendition(&format!("spine item {}", item.idref), item.rendition)?;
    }
    Ok(())
}

fn validate_rendition(scope: &str, rendition: RenditionSemantics) -> Result<()> {
    if scope == "publication" && rendition.align_x == Some(RenditionAlign::Center) {
        return Err(Error::UnsupportedEpub(format!(
            "{scope} rendition:align-x-center has no demonstrated KF8 projection"
        )));
    }
    Ok(())
}

pub(super) fn merge_rendition(
    publication: crate::book::RenditionSemantics,
    item: crate::book::RenditionSemantics,
) -> crate::book::RenditionSemantics {
    crate::book::RenditionSemantics {
        orientation: item.orientation.or(publication.orientation),
        spread: item.spread.or(publication.spread),
        flow: item.flow.or(publication.flow),
        align_x: item.align_x.or(publication.align_x),
        page_spread: item.page_spread,
    }
}

fn is_content_document(item: &ManifestItem) -> bool {
    item.media_type
        .eq_ignore_ascii_case("application/xhtml+xml")
        || item.media_type.eq_ignore_ascii_case("text/html")
        || item.media_type.eq_ignore_ascii_case("image/svg+xml")
}

fn resolve_content_source<'a>(
    item: &'a ManifestItem,
    manifest: &'a [ManifestItem],
    manifest_id_index: &ManifestIdIndex,
) -> Result<&'a ManifestItem> {
    let mut current = item;
    let mut visited = HashSet::new();
    loop {
        if is_content_document(current) {
            return Ok(current);
        }
        if !visited.insert(current.id.as_str()) {
            return Err(Error::InvalidEpub(format!(
                "fallback chain for {} contains a cycle at {}",
                item.id, current.id
            )));
        }
        let Some(fallback_id) = current.fallback.as_deref() else {
            return Err(Error::UnsupportedEpub(format!(
                "fallback chain for {} ends at unsupported media type {}",
                item.id, current.media_type
            )));
        };
        current = manifest_id_index
            .get(manifest, fallback_id)
            .ok_or_else(|| {
                Error::InvalidEpub(format!(
                    "fallback chain for {} references missing target {}",
                    item.id, fallback_id
                ))
            })?;
    }
}

fn resolve_content_source_for_spine<'a>(
    item: &'a ManifestItem,
    manifest: &'a [ManifestItem],
    manifest_id_index: &ManifestIdIndex,
    warnings: &mut WarningCollector,
) -> Result<Option<&'a ManifestItem>> {
    if (is_audio_media_type(&item.media_type)
        || is_video_media_type(&item.media_type)
        || is_smil_media_type(&item.media_type))
        && item.fallback.is_none()
    {
        return Ok(None);
    }
    let content_source = resolve_content_source(item, manifest, manifest_id_index)?;
    if content_source.id != item.id {
        warnings.add_category_once(
            WarningCode::W002,
            "an unsupported manifest resource used its EPUB fallback content",
        );
    }
    Ok(Some(content_source))
}

pub(super) fn resolve_binary_fallback<'a>(
    item: &'a ManifestItem,
    manifest: &'a [ManifestItem],
    manifest_id_index: &ManifestIdIndex,
) -> Result<&'a ManifestItem> {
    let mut current = item;
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current.id.as_str()) {
            return Err(Error::InvalidEpub(format!(
                "fallback chain for {} contains a cycle at {}",
                item.id, current.id
            )));
        }
        let Some(fallback_id) = current.fallback.as_deref() else {
            return Err(Error::UnsupportedEpub(format!(
                "fallback chain for {} ends without a usable binary resource",
                item.id
            )));
        };
        current = manifest_id_index
            .get(manifest, fallback_id)
            .ok_or_else(|| {
                Error::InvalidEpub(format!(
                    "fallback chain for {} references missing target {}",
                    item.id, fallback_id
                ))
            })?;
        if is_usable_binary_fallback(current) {
            return Ok(current);
        }
    }
}

pub(super) fn is_unsupported_binary_media_type(media_type: &str) -> bool {
    let media_type = media_type.to_ascii_lowercase();
    if is_content_document_media_type(&media_type)
        || media_type == "text/css"
        || is_audio_media_type(&media_type)
        || is_video_media_type(&media_type)
        || is_smil_media_type(&media_type)
    {
        return false;
    }
    if media_type.starts_with("image/") {
        return !matches!(
            media_type.as_str(),
            "image/gif" | "image/jpeg" | "image/jpg" | "image/png" | "image/webp"
        );
    }
    !is_font_media_type(&media_type)
}

fn is_usable_binary_fallback(item: &ManifestItem) -> bool {
    let media_type = item.media_type.to_ascii_lowercase();
    if media_type == "image/svg+xml" {
        return true;
    }
    if is_content_document_media_type(&media_type)
        || media_type == "text/css"
        || is_audio_media_type(&media_type)
        || is_video_media_type(&media_type)
        || is_smil_media_type(&media_type)
    {
        return false;
    }
    if media_type.starts_with("image/") {
        return matches!(
            media_type.as_str(),
            "image/gif" | "image/jpeg" | "image/jpg" | "image/png" | "image/webp"
        );
    }
    is_font_media_type(&media_type)
}

fn is_content_document_media_type(media_type: &str) -> bool {
    media_type == "application/xhtml+xml"
        || media_type == "text/html"
        || media_type == "image/svg+xml"
}

fn validate_manifest_paths(manifest: &[ManifestItem], base: &Path) -> Result<()> {
    let base = format!("{}/", base.to_string_lossy());
    for item in manifest {
        if is_external_reference(&item.href) {
            continue;
        }
        if resolve_path(&base, &item.href).is_none() {
            return Err(Error::InvalidEpub(format!(
                "manifest item {} has a path that escapes the EPUB root",
                item.id
            )));
        }
    }
    Ok(())
}

fn validate_amazon_document_count(manifest: &[ManifestItem], warnings: &mut WarningCollector) {
    let count = manifest
        .iter()
        .filter(|item| is_html_content_document(item) && !has_property(item, "nav"))
        .count();
    if count >= MAX_AMAZON_HTML_DOCUMENTS {
        warnings.add_category_once(
            WarningCode::W006,
            "the publication meets or exceeds Amazon's 300 HTML/XHTML-document publishing guidance",
        );
    }
}

pub(super) fn is_html_content_document(item: &ManifestItem) -> bool {
    item.media_type
        .eq_ignore_ascii_case("application/xhtml+xml")
        || item.media_type.eq_ignore_ascii_case("text/html")
}

pub(super) fn is_font_media_type(media_type: &str) -> bool {
    media_type.to_ascii_lowercase().starts_with("font/")
        || matches!(
            media_type.to_ascii_lowercase().as_str(),
            "application/font-woff"
                | "application/font-sfnt"
                | "application/vnd.ms-opentype"
                | "application/x-font-opentype"
                | "application/x-font-ttf"
        )
}
