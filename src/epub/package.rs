//! Orchestrate EPUB ZIP, package, document, style, and navigation parsing.
//!
//! The flow is ZIP → container/OPF → resources/XHTML/CSS → navigation → Book
//! IR. Detailed OPF, navigation, and XHTML semantics live in their neighboring
//! modules rather than being reassembled here.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::package_archive::{BoundedZipArchive, read_zip_entry, validate_ocf_paths};

use crate::book::{
    Book, ContentDocument, Navigation, ReadingOrderItem, RenditionAlign, RenditionSemantics,
    Resource, Resources, Styles,
};
use crate::css::SYNTHETIC_INLINE_CSS_PROPERTY;
use crate::error::{Error, Result};
use crate::xhtml::path::{
    is_external_reference, normalize_path_lossy as normalize_path, resolve_path,
};
use crate::{WarningCode, WarningCollector};

use super::css::font_face_resource_references;
use super::navigation::{canonicalize_navigation, parse_nav_xhtml, parse_ncx};
use super::opf::{
    ManifestItem, cover_image_paths, has_property, is_legacy_svg_cover_document, parse_opf,
    parse_rootfile,
};
use super::xhtml::{
    ViewportQuality, document_styles_with_occupied_hrefs, infer_layout,
    parse_xhtml_semantics_and_document_root_writing_mode, sanitize_unsupported_xhtml,
    unique_resource_id, validate_local_resource_paths, validate_viewport,
};

/// Amazon accepts individual HTML/XHTML content documents strictly below
/// 30,000,000 decimal bytes, and fewer than 300 such documents per publication.
const MAX_AMAZON_HTML_BYTES: u64 = 30_000_000;
const MAX_AMAZON_HTML_DOCUMENTS: usize = 300;

/// Parse an EPUB package from bytes without depending on a filesystem.
struct LoadedPackage<'a> {
    archive: BoundedZipArchive<Cursor<&'a [u8]>>,
    parsed: super::opf::ParsedOpf,
    base: PathBuf,
    manifest_id_index: ManifestIdIndex,
    font_obfuscation_keys: HashMap<String, [u8; 20]>,
    content_source_ids: HashSet<String>,
    spine_content_source_indices: Vec<Option<usize>>,
}

struct ManifestIdIndex {
    positions: HashMap<String, usize>,
}

impl ManifestIdIndex {
    fn from_manifest(manifest: &[ManifestItem]) -> Self {
        let mut positions = HashMap::with_capacity(manifest.len());
        for (index, item) in manifest.iter().enumerate() {
            positions.entry(item.id.clone()).or_insert(index);
        }
        Self { positions }
    }

    fn get<'a>(&self, manifest: &'a [ManifestItem], id: &str) -> Option<&'a ManifestItem> {
        self.index(id).map(|index| &manifest[index])
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.positions.get(id).copied()
    }
}

struct LoadedResources {
    resources: Resources,
    xhtml: HashMap<String, String>,
}

struct DiscoveredContent {
    content: Vec<ContentDocument>,
    reading_items: Vec<ReadingOrderItem>,
    document_writing_modes: Vec<Option<crate::book::WritingMode>>,
    fixed_page_viewports: Vec<Option<String>>,
}

fn load_package<'a>(input: &'a [u8], warnings: &mut WarningCollector) -> Result<LoadedPackage<'a>> {
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
        super::font_obfuscation::load_font_obfuscation(&mut archive, &parsed, &base)?;
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

fn load_resources(
    archive: &mut BoundedZipArchive<Cursor<&[u8]>>,
    parsed: &super::opf::ParsedOpf,
    base: &Path,
    font_obfuscation_keys: &HashMap<String, [u8; 20]>,
    content_source_ids: &HashSet<String>,
    manifest_id_index: &ManifestIdIndex,
    warnings: &mut WarningCollector,
) -> Result<LoadedResources> {
    let mut resources = Resources::default();
    let mut xhtml = HashMap::<String, String>::new();
    let spine_ids = parsed
        .spine
        .iter()
        .map(|item| item.idref.as_str())
        .collect::<HashSet<_>>();
    for item in &parsed.manifest {
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
            super::font_obfuscation::deobfuscate_font(&mut data, key);
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
            let source = decode_text_entry(&data, TextKind::Css)?;
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
    Ok(LoadedResources { resources, xhtml })
}

fn discover_content(
    parsed: &super::opf::ParsedOpf,
    base: &Path,
    resources: &mut Resources,
    xhtml: &mut HashMap<String, String>,
    manifest_id_index: &ManifestIdIndex,
    spine_content_source_indices: &[Option<usize>],
    warnings: &mut WarningCollector,
) -> Result<DiscoveredContent> {
    let mut occupied_hrefs = resources
        .items
        .iter()
        .map(|resource| normalize_path(&resource.href))
        .collect::<HashSet<_>>();
    let mut content = Vec::new();
    let mut reading_items = Vec::new();
    let mut document_writing_modes = Vec::new();
    let mut fixed_page_viewports = Vec::new();
    let cover_image_paths = cover_image_paths(parsed, base);
    let mut occupied_resource_ids = resources
        .items
        .iter()
        .map(|resource| resource.id.clone())
        .collect::<HashSet<_>>();
    if spine_content_source_indices.len() != parsed.spine.len() {
        return Err(Error::Output(
            "spine content source index does not match spine".to_owned(),
        ));
    }
    let mut remaining_source_uses = HashMap::<usize, usize>::new();
    for &effective_index in spine_content_source_indices.iter().flatten() {
        *remaining_source_uses.entry(effective_index).or_default() += 1;
    }
    for (spine_index, spine_item) in parsed.spine.iter().enumerate() {
        let source_item = manifest_id_index
            .get(&parsed.manifest, &spine_item.idref)
            .ok_or_else(|| {
                Error::InvalidEpub(format!(
                    "spine references missing manifest item {}",
                    spine_item.idref
                ))
            })?;
        let source = spine_content_source_indices[spine_index].and_then(|effective_index| {
            let effective_id = &parsed.manifest[effective_index].id;
            match remaining_source_uses.get_mut(&effective_index) {
                Some(remaining) if *remaining == 1 => xhtml.remove(effective_id),
                Some(remaining) => {
                    *remaining -= 1;
                    xhtml.get(effective_id).cloned()
                }
                None => None,
            }
        });
        if let Some(source) = source {
            let effective_item = spine_content_source_indices[spine_index]
                .and_then(|index| parsed.manifest.get(index))
                .expect("content source was validated during package loading");
            if spine_item.layout == super::opf::SpineLayout::PrePaginated {
                if validate_viewport(&source)? == ViewportQuality::Degraded {
                    warnings.add_category_once(
                        WarningCode::W004,
                        "fixed-page viewport metadata was incomplete or ambiguous and was degraded",
                    );
                }
                fixed_page_viewports.push(super::xhtml::fixed_page_viewport_resolution(&source)?);
            }
            let (mut semantic, writing_mode) =
                parse_xhtml_semantics_and_document_root_writing_mode(&source)?;
            semantic.is_cover = semantic.is_cover
                || is_legacy_svg_cover_document(
                    effective_item,
                    &semantic,
                    parsed.spine.first().map(|spine_item| &spine_item.idref),
                    &cover_image_paths,
                    base,
                );
            let is_cover = semantic.is_cover;
            drop(semantic);
            document_writing_modes.push(writing_mode);
            let discovered_styles = document_styles_with_occupied_hrefs(
                &source,
                &effective_item.href,
                &resolve_href(base, &effective_item.href),
                &mut occupied_hrefs,
                warnings,
            )?;
            let referenced_styles = discovered_styles
                .iter()
                .map(|style| style.reference.clone())
                .collect();
            let rendition = merge_rendition(parsed.rendition, spine_item.rendition);
            let mut inline_index = 0usize;
            for style in &discovered_styles {
                let Some(source) = style.inline_source.as_deref() else {
                    continue;
                };
                let Some(href) = style.resource_href.as_deref() else {
                    continue;
                };
                let base_id = format!("__inline_style_{}_{}", spine_item.idref, inline_index);
                let resource_id = unique_resource_id(&base_id, &occupied_resource_ids);
                occupied_resource_ids.insert(resource_id.clone());
                resources.items.push(Resource {
                    id: resource_id,
                    href: href.to_owned(),
                    media_type: "text/css".to_owned(),
                    properties: vec![SYNTHETIC_INLINE_CSS_PROPERTY.to_owned()],
                    data: source.as_bytes().to_vec(),
                });
                inline_index += 1;
            }
            let mut content_document = ContentDocument {
                id: spine_item.idref.clone(),
                href: effective_item.href.clone(),
                media_type: effective_item.media_type.clone(),
                source_xhtml: source,
                is_cover,
                source_properties: spine_item.properties.clone(),
                source_spine_index: spine_index,
                referenced_styles,
                rendition,
                ..ContentDocument::default()
            };
            content_document.set_layout_pre_paginated(
                spine_item.layout == super::opf::SpineLayout::PrePaginated,
            );
            content.push(content_document);
        } else if spine_item.layout == super::opf::SpineLayout::PrePaginated {
            fixed_page_viewports.push(None);
        }
        let effective_item = spine_content_source_indices[spine_index]
            .and_then(|index| parsed.manifest.get(index))
            .unwrap_or(source_item);
        reading_items.push(ReadingOrderItem {
            id: spine_item.idref.clone(),
            href: effective_item.href.clone(),
            media_type: effective_item.media_type.clone(),
            linear: spine_item.linear,
        });
    }
    Ok(DiscoveredContent {
        content,
        reading_items,
        document_writing_modes,
        fixed_page_viewports,
    })
}

#[allow(dead_code)]
pub fn parse_epub(input: &[u8]) -> Result<Book> {
    let mut warnings = WarningCollector::new();
    parse_epub_with_warnings(input, &mut warnings)
}

pub fn parse_epub_with_warnings(input: &[u8], warnings: &mut WarningCollector) -> Result<Book> {
    let LoadedPackage {
        mut archive,
        parsed,
        base,
        manifest_id_index,
        font_obfuscation_keys,
        content_source_ids,
        spine_content_source_indices,
    } = load_package(input, warnings)?;
    let LoadedResources {
        resources,
        mut xhtml,
    } = load_resources(
        &mut archive,
        &parsed,
        &base,
        &font_obfuscation_keys,
        &content_source_ids,
        &manifest_id_index,
        warnings,
    )?;
    let mut resources = resources;
    let DiscoveredContent {
        content,
        reading_items,
        document_writing_modes,
        fixed_page_viewports,
    } = discover_content(
        &parsed,
        &base,
        &mut resources,
        &mut xhtml,
        &manifest_id_index,
        &spine_content_source_indices,
        warnings,
    )?;
    let styles = validate_and_parse_styles(&content, &resources, &base, warnings)?;

    let navigation = load_navigation(&mut archive, &parsed, &base, &manifest_id_index)?;
    let layout = infer_layout(
        &styles,
        parsed.page_progression,
        parsed.primary_writing_mode,
        &document_writing_modes,
    );
    let cover = parsed.metadata.cover.clone().or_else(|| {
        parsed
            .manifest
            .iter()
            .find(|item| {
                item.properties
                    .iter()
                    .any(|property| property.eq_ignore_ascii_case("cover-image"))
            })
            .map(|item| item.id.clone())
    });
    let mut metadata = parsed.metadata;
    metadata.cover = cover;
    if metadata.original_resolution.is_none()
        && !fixed_page_viewports.is_empty()
        && fixed_page_viewports.iter().all(Option::is_some)
    {
        let first = fixed_page_viewports[0].as_deref();
        if fixed_page_viewports
            .iter()
            .all(|viewport| viewport.as_deref() == first)
        {
            metadata.original_resolution = first.map(str::to_owned);
        }
    }
    Ok(Book {
        metadata,
        reading_order: crate::book::ReadingOrder {
            items: reading_items,
            page_progression: parsed.page_progression,
        },
        navigation,
        content,
        resources,
        layout,
        rendition: parsed.rendition,
        styles,
    })
}

fn validate_and_parse_styles(
    content: &[ContentDocument],
    resources: &Resources,
    base: &Path,
    warnings: &mut WarningCollector,
) -> Result<Styles> {
    let css_resource_index = super::css::CssResourceIndex::new(content, &resources.items);
    let active_css_hrefs =
        super::css::active_css_stylesheets(&css_resource_index, content, warnings)?;
    let mut styles = Styles::default();
    for resource in resources
        .items
        .iter()
        .filter(|resource| resource.media_type.eq_ignore_ascii_case("text/css"))
    {
        if !active_css_hrefs.contains(&normalize_path(&resource.href)) {
            continue;
        }
        let source = std::str::from_utf8(&resource.data)
            .expect("EPUB CSS resources are normalized to UTF-8");
        if !resource
            .properties
            .iter()
            .any(|property| property == SYNTHETIC_INLINE_CSS_PROPERTY)
        {
            let stylesheet_path = resolve_href(base, &resource.href);
            super::css::validate_local_resource_paths(source, &stylesheet_path)?;
        }
        super::css::warn_remote_css_references(source, warnings)?;
        super::css::validate_kf8_css_with_warnings(source, warnings)?;
        styles
            .sheets
            .push(crate::epub::parse_css(&resource.href, source));
    }
    validate_required_font_resources(
        &active_css_hrefs,
        resources,
        base,
        &css_resource_index,
        warnings,
    )?;
    styles.computed = styles
        .sheets
        .iter()
        .flat_map(|sheet| sheet.computed_styles())
        .collect();
    Ok(styles)
}

fn load_navigation(
    archive: &mut BoundedZipArchive<Cursor<&[u8]>>,
    parsed: &super::opf::ParsedOpf,
    base: &Path,
    manifest_id_index: &ManifestIdIndex,
) -> Result<Navigation> {
    let nav_item = parsed
        .manifest
        .iter()
        .find(|item| has_property(item, "nav"));
    let nav_is_primary = parsed.ncx_id.is_none() && nav_item.is_some();
    let (mut navigation, navigation_path, supplemental_nav) =
        if let Some(ncx_id) = parsed.ncx_id.as_deref() {
            let item = manifest_id_index.get(&parsed.manifest, ncx_id);
            if let Some(item) = item {
                let path = resolve_href(base, &item.href);
                (
                    parse_ncx(&read_zip_entry(archive, &path)?)?,
                    Some(path),
                    nav_item.map(|nav_item| resolve_href(base, &nav_item.href)),
                )
            } else {
                (
                    Navigation::default(),
                    None,
                    nav_item.map(|nav_item| resolve_href(base, &nav_item.href)),
                )
            }
        } else if let Some(item) = nav_item {
            let path = resolve_href(base, &item.href);
            (
                parse_nav_xhtml(&read_zip_entry(archive, &path)?)?,
                Some(path),
                None,
            )
        } else {
            (Navigation::default(), None, None)
        };
    // Landmarks are a separate semantic channel from the visible TOC. Keep
    // them when an EPUB 3 package also supplies an NCX, because Guide and
    // initial-route consumers use the landmark meanings rather than the TOC
    // source selection.
    if nav_is_primary {
        let use_supplemental_items = navigation.items.is_empty();
        let mut nav = Navigation {
            items: if use_supplemental_items {
                std::mem::take(&mut navigation.items)
            } else {
                Vec::new()
            },
            page_list: std::mem::take(&mut navigation.page_list),
            landmarks: std::mem::take(&mut navigation.landmarks),
            custom: std::mem::take(&mut navigation.custom),
            ..Navigation::default()
        };
        if let Some(nav_path) = navigation_path.as_deref() {
            canonicalize_navigation(&mut nav, nav_path, base, &parsed.manifest)?;
        }
        navigation.landmarks = nav.landmarks;
        navigation.page_list = nav.page_list;
        navigation.custom = nav.custom;
        if use_supplemental_items {
            navigation.items = nav.items;
        }
    } else if let Some(nav_path) = supplemental_nav {
        let mut nav = parse_nav_xhtml(&read_zip_entry(archive, &nav_path)?)?;
        canonicalize_navigation(&mut nav, &nav_path, base, &parsed.manifest)?;
        navigation.landmarks = nav.landmarks;
        navigation.page_list = nav.page_list;
        navigation.custom = nav.custom;
        if navigation.items.is_empty() {
            navigation.items = nav.items;
        }
    }
    if let Some(path) = navigation_path {
        canonicalize_navigation(&mut navigation, &path, base, &parsed.manifest)?;
    }
    Ok(navigation)
}

fn warn_unsupported_media_semantics(
    parsed: &super::opf::ParsedOpf,
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

fn validate_rendition_semantics(parsed: &super::opf::ParsedOpf) -> Result<()> {
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

fn merge_rendition(
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

fn resolve_binary_fallback<'a>(
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

fn is_unsupported_binary_media_type(media_type: &str) -> bool {
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

pub(super) fn resolve_href(base: &Path, href: &str) -> String {
    let base = base.to_string_lossy();
    let base = format!("{base}/");
    resolve_path(&base, href).unwrap_or_default()
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

fn validate_required_font_resources(
    active_css_hrefs: &HashSet<String>,
    resources: &Resources,
    base: &Path,
    css_resource_index: &super::css::CssResourceIndex<'_>,
    warnings: &mut WarningCollector,
) -> Result<()> {
    for stylesheet in resources.items.iter().filter(|resource| {
        resource.media_type.eq_ignore_ascii_case("text/css")
            && active_css_hrefs.contains(&normalize_path(&resource.href))
    }) {
        let targets = font_face_resource_references(
            std::str::from_utf8(&stylesheet.data)
                .expect("EPUB CSS resources are normalized to UTF-8"),
        )?;
        let stylesheet_base_href =
            super::css::css_resource_base_href(css_resource_index, stylesheet);
        let stylesheet_path = resolve_href(base, &stylesheet_base_href);
        for target in targets {
            if is_external_reference(&target) {
                continue;
            }
            let resolved = resolve_path(&stylesheet_path, &target).ok_or_else(|| {
                Error::InvalidEpub(format!("font resource path {target} escapes the EPUB root"))
            })?;
            if let Some(font) = resources.items.iter().find(|resource| {
                normalize_path(&resolve_href(base, &resource.href)) == resolved
                    && is_font_media_type(&resource.media_type)
            }) {
                if font.data.is_empty() {
                    return Err(Error::InvalidEpub(format!(
                        "required embedded font {} has zero length",
                        font.id
                    )));
                }
            } else {
                warnings.add_category_once(
                    WarningCode::W004,
                    "one or more local @font-face resources could not be resolved and will use a fallback",
                );
            }
        }
    }
    Ok(())
}

fn is_html_content_document(item: &ManifestItem) -> bool {
    item.media_type
        .eq_ignore_ascii_case("application/xhtml+xml")
        || item.media_type.eq_ignore_ascii_case("text/html")
}

fn is_font_media_type(media_type: &str) -> bool {
    media_type.to_ascii_lowercase().starts_with("font/")
        || matches!(
            media_type.to_ascii_lowercase().as_str(),
            "application/font-sfnt"
                | "application/vnd.ms-opentype"
                | "application/x-font-opentype"
                | "application/x-font-ttf"
        )
}
