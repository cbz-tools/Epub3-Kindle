//! Parse OPF XML into the package-level [`ParsedOpf`] intermediate form.
//!
//! Manifest, spine, metadata, and package-level cover/layout declarations are
//! handled here. XHTML semantics and KF8 serialization are outside this module.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;

use super::navigation::split_link_suffix;
use super::package::resolve_href;
use super::xhtml::{is_primary_writing_mode_meta, parse_writing_mode};
use crate::book::{
    CollectionMetadata, CreatorMetadata, Metadata, MetadataRecord, PageProgression, PageSpread,
    RenditionAlign, RenditionFlow, RenditionOrientation, RenditionSemantics, RenditionSpread,
    SemanticDocument, WritingMode,
};
use crate::error::{Error, Result};
use crate::xhtml::path::normalize_path_lossy as normalize_path;

const OPF_NAMESPACE: &[u8] = b"http://www.idpf.org/2007/opf";
const DUBLIN_CORE_NAMESPACE: &[u8] = b"http://purl.org/dc/elements/1.1/";
#[derive(Debug, Clone)]
pub(super) struct ManifestItem {
    pub(super) id: String,
    pub(super) href: String,
    pub(super) media_type: String,
    pub(super) properties: Vec<String>,
    pub(super) fallback: Option<String>,
    pub(super) media_overlay: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SpineLayout {
    #[default]
    Reflowable,
    PrePaginated,
}

impl SpineLayout {
    fn from_itemref_properties(properties: &[String]) -> Option<Self> {
        properties.iter().find_map(|property| {
            let property = property.to_ascii_lowercase();
            if property == "rendition:layout-pre-paginated" {
                Some(Self::PrePaginated)
            } else if property == "rendition:layout-reflowable" {
                Some(Self::Reflowable)
            } else {
                None
            }
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct SpineItem {
    pub(super) idref: String,
    pub(super) linear: bool,
    pub(super) properties: Vec<String>,
    pub(super) media_overlay: Option<String>,
    pub(super) layout: SpineLayout,
    pub(super) rendition: RenditionSemantics,
}

#[derive(Debug, Default)]
pub(super) struct ParsedOpf {
    pub(super) metadata: Metadata,
    pub(super) manifest: Vec<ManifestItem>,
    pub(super) spine: Vec<SpineItem>,
    pub(super) publication_layout: Option<SpineLayout>,
    pub(super) page_progression: PageProgression,
    pub(super) primary_writing_mode: Option<WritingMode>,
    pub(super) ncx_id: Option<String>,
    pub(super) unique_identifier_id: Option<String>,
    pub(super) rendition: RenditionSemantics,
}

#[derive(Debug, Clone, Copy)]
enum MetadataMetaField {
    Cover,
    FixedLayout,
    RenditionLayout,
    BookType,
    OrientationLock,
    RenditionOrientation,
    RenditionSpread,
    RenditionFlow,
    RenditionViewport,
    RenditionAlign,
    OriginalResolution,
    PrimaryWritingMode,
}

pub(super) fn parse_rootfile(xml: &[u8]) -> Result<String> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Empty(event) | Event::Start(event)
                if local_name(event.name().as_ref()) == "rootfile" =>
            {
                if let Some(path) = attr(&event, "full-path") {
                    return Ok(normalize_path(&path));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Err(Error::InvalidEpub(
        "container.xml has no rootfile full-path".to_owned(),
    ))
}

pub(super) fn parse_opf(xml: &[u8]) -> Result<ParsedOpf> {
    let mut result = ParsedOpf::default();
    let mut reader = NsReader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut metadata_depth = 0usize;
    let mut metadata_elements = Vec::new();
    loop {
        let (namespace, event) = reader.read_resolved_event_into(&mut buffer)?;
        let is_opf = is_opf_namespace(&namespace);
        let metadata_namespace = metadata_namespace(&namespace);
        match event {
            Event::Start(event) => {
                let name = local_name(event.name().as_ref());
                if name == "package" {
                    validate_package_declaration(is_opf, &event)?;
                }
                if is_opf && name == "metadata" {
                    metadata_depth += 1;
                } else if metadata_depth > 0 && is_metadata_element(metadata_namespace, &name) {
                    let metadata_namespace = metadata_namespace
                        .expect("recognized metadata element has a known namespace");
                    metadata_elements.push(MetadataElement::from_start(
                        &event,
                        name,
                        metadata_namespace,
                    ));
                } else if is_opf {
                    parse_opf_start(&event, &mut result);
                }
            }
            Event::Empty(event) => {
                let name = local_name(event.name().as_ref());
                if name == "package" {
                    validate_package_declaration(is_opf, &event)?;
                }
                if metadata_depth > 0 && is_metadata_element(metadata_namespace, &name) {
                    let metadata_namespace = metadata_namespace
                        .expect("recognized metadata element has a known namespace");
                    apply_metadata_element(
                        &mut result,
                        MetadataElement::from_empty(&event, name, metadata_namespace),
                    );
                } else if is_opf {
                    parse_opf_start(&event, &mut result);
                }
            }
            Event::Text(event) => {
                let text = event
                    .unescape()
                    .map_err(|error| Error::Xml(error.to_string()))?;
                if let Some(element) = metadata_elements.last_mut() {
                    element.text.push_str(&text);
                }
            }
            Event::CData(event) => {
                if let Some(element) = metadata_elements.last_mut() {
                    element
                        .text
                        .push_str(&String::from_utf8_lossy(event.as_ref()));
                }
            }
            Event::End(event) => {
                let name = local_name(event.name().as_ref());
                if metadata_depth > 0 && is_opf && name == "metadata" {
                    metadata_depth = metadata_depth.saturating_sub(1);
                } else if metadata_elements.last().is_some_and(|element| {
                    element.name == name && metadata_namespace == Some(element.namespace)
                }) {
                    if let Some(element) = metadata_elements.pop() {
                        apply_metadata_element(&mut result, element);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if result.manifest.is_empty() || result.spine.is_empty() {
        return Err(Error::InvalidEpub(
            "OPF must contain a manifest and spine".to_owned(),
        ));
    }
    let mut manifest_ids = HashSet::with_capacity(result.manifest.len());
    for item in &result.manifest {
        if !manifest_ids.insert(item.id.as_str()) {
            return Err(Error::InvalidEpub(format!(
                "duplicate manifest item id {}",
                item.id
            )));
        }
    }
    let unique_identifier = result.unique_identifier_id.as_deref().ok_or_else(|| {
        Error::InvalidEpub(
            "package unique-identifier attribute is required and must resolve to an identifier"
                .to_owned(),
        )
    })?;
    let identifier_resolves = result.metadata.records.iter().any(|record| {
        record.id.as_deref() == Some(unique_identifier)
            && property_matches(&record.property, "identifier")
            && !record.value.trim().is_empty()
    });
    if !identifier_resolves {
        return Err(Error::InvalidEpub(format!(
            "package unique-identifier {unique_identifier} does not resolve to an identifier"
        )));
    }
    let publication_layout = result.publication_layout.unwrap_or_default();
    for spine_item in &mut result.spine {
        spine_item.layout = SpineLayout::from_itemref_properties(&spine_item.properties)
            .unwrap_or(publication_layout);
        spine_item.rendition = rendition_from_properties(&spine_item.properties)?;
    }
    result.rendition = rendition_from_metadata(&result.metadata)?;
    finalize_metadata(&mut result.metadata);
    Ok(result)
}

fn validate_package_declaration(is_opf: bool, event: &BytesStart<'_>) -> Result<()> {
    if !is_opf {
        return Err(Error::InvalidEpub(
            "package namespace must be http://www.idpf.org/2007/opf".to_owned(),
        ));
    }
    let version = attr(event, "version").ok_or_else(|| {
        Error::InvalidEpub("package version is required and must be EPUB 3.x".to_owned())
    })?;
    if !version.starts_with("3.") {
        return Err(Error::InvalidEpub(format!(
            "unsupported package version {version:?}; expected EPUB 3.x"
        )));
    }
    Ok(())
}

fn is_opf_namespace(namespace: &ResolveResult<'_>) -> bool {
    matches!(namespace, ResolveResult::Bound(Namespace(uri)) if *uri == OPF_NAMESPACE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetadataNamespace {
    Opf,
    DublinCore,
}

fn metadata_namespace(namespace: &ResolveResult<'_>) -> Option<MetadataNamespace> {
    match namespace {
        ResolveResult::Bound(Namespace(uri)) if *uri == OPF_NAMESPACE => {
            Some(MetadataNamespace::Opf)
        }
        ResolveResult::Bound(Namespace(uri)) if *uri == DUBLIN_CORE_NAMESPACE => {
            Some(MetadataNamespace::DublinCore)
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct MetadataElement {
    name: String,
    namespace: MetadataNamespace,
    id: Option<String>,
    property: Option<String>,
    refines: Option<String>,
    scheme: Option<String>,
    content: Option<String>,
    legacy_kind: Option<MetadataMetaField>,
    text: String,
}

impl MetadataElement {
    fn from_start(event: &BytesStart<'_>, name: String, namespace: MetadataNamespace) -> Self {
        Self {
            name: name.clone(),
            namespace,
            id: attr(event, "id"),
            property: attr(event, "property"),
            refines: attr(event, "refines").map(|value| normalize_refines(&value)),
            scheme: attr(event, "scheme"),
            content: attr(event, "content"),
            legacy_kind: (name == "meta")
                .then(|| metadata_meta_kind(event))
                .flatten(),
            text: String::new(),
        }
    }

    fn from_empty(event: &BytesStart<'_>, name: String, namespace: MetadataNamespace) -> Self {
        Self::from_start(event, name, namespace)
    }
}

fn is_metadata_element(namespace: Option<MetadataNamespace>, name: &str) -> bool {
    match namespace {
        Some(MetadataNamespace::Opf) => matches!(
            name,
            "title"
                | "creator"
                | "language"
                | "identifier"
                | "date"
                | "publisher"
                | "description"
                | "contributor"
                | "meta"
                | "collection"
        ),
        Some(MetadataNamespace::DublinCore) => matches!(
            name,
            "title"
                | "creator"
                | "language"
                | "identifier"
                | "date"
                | "publisher"
                | "description"
                | "contributor"
        ),
        None => false,
    }
}

fn normalize_refines(value: &str) -> String {
    value.trim().trim_start_matches('#').to_owned()
}

fn apply_metadata_element(result: &mut ParsedOpf, element: MetadataElement) {
    let value = element
        .content
        .as_deref()
        .unwrap_or(element.text.trim())
        .trim()
        .to_owned();
    if value.is_empty() {
        return;
    }
    if element.name == "meta" {
        if let Some(field) = element.legacy_kind {
            apply_metadata_meta(result, field, &value);
        }
    }
    let property = element
        .property
        .or_else(|| (element.name != "meta").then_some(element.name.clone()))
        .unwrap_or_default();
    if property.is_empty() {
        return;
    }
    result.metadata.records.push(MetadataRecord {
        id: element.id,
        property,
        refines: element.refines,
        scheme: element.scheme,
        value,
    });
}

fn property_matches(property: &str, wanted: &str) -> bool {
    property
        .rsplit(':')
        .next()
        .is_some_and(|value| value.eq_ignore_ascii_case(wanted))
}

fn refined_value<'a>(
    records: &'a [MetadataRecord],
    target: &str,
    property: &str,
) -> Option<&'a str> {
    records
        .iter()
        .find(|record| {
            record.refines.as_deref() == Some(target)
                && property_matches(&record.property, property)
        })
        .map(|record| record.value.as_str())
}

fn finalize_metadata(metadata: &mut Metadata) {
    let records = metadata.records.as_slice();
    let titles = records
        .iter()
        .filter(|record| property_matches(&record.property, "title") && record.refines.is_none())
        .collect::<Vec<_>>();
    let main_title = titles
        .iter()
        .find(|record| {
            record.id.as_deref().is_some_and(|id| {
                refined_value(records, id, "title-type")
                    .is_some_and(|value| value.eq_ignore_ascii_case("main"))
            })
        })
        .or_else(|| titles.first());
    let title = main_title.map(|record| record.value.clone());
    let title_file_as = main_title.and_then(|record| {
        record
            .id
            .as_deref()
            .and_then(|id| refined_value(records, id, "file-as"))
            .map(str::to_owned)
    });

    let creator_records = records
        .iter()
        .filter(|record| property_matches(&record.property, "creator") && record.refines.is_none())
        .collect::<Vec<_>>();
    let creators: Vec<CreatorMetadata> = creator_records
        .iter()
        .map(|record| CreatorMetadata {
            value: record.value.clone(),
            role: record
                .id
                .as_deref()
                .and_then(|id| refined_value(records, id, "role").map(str::to_owned)),
        })
        .collect();
    let first_author = creators.iter().find(|creator| {
        creator.role.as_deref().is_none_or(|role| {
            role.eq_ignore_ascii_case("aut") || role.eq_ignore_ascii_case("author")
        })
    });
    let creator = first_author
        .or_else(|| creators.first())
        .map(|creator| creator.value.clone());
    let creator_file_as = first_author.and_then(|creator| {
        creator_records
            .iter()
            .find(|record| record.value == creator.value)
            .and_then(|record| {
                record
                    .id
                    .as_deref()
                    .and_then(|id| refined_value(records, id, "file-as"))
                    .map(str::to_owned)
            })
    });
    let (publisher, publisher_file_as) =
        if let Some(publisher) = records.iter().rev().find(|record| {
            property_matches(&record.property, "publisher") && record.refines.is_none()
        }) {
            (
                Some(publisher.value.clone()),
                publisher
                    .id
                    .as_deref()
                    .and_then(|id| refined_value(records, id, "file-as").map(str::to_owned)),
            )
        } else {
            (None, None)
        };
    let language = records
        .iter()
        .rev()
        .find(|record| property_matches(&record.property, "language") && record.refines.is_none())
        .map(|record| record.value.clone())
        .or_else(|| metadata.language.clone());
    let identifier = records
        .iter()
        .rev()
        .find(|record| property_matches(&record.property, "identifier") && record.refines.is_none())
        .map(|record| record.value.clone())
        .or_else(|| metadata.identifier.clone());
    let publication_date = records
        .iter()
        .rev()
        .find(|record| record.property.eq_ignore_ascii_case("date") && record.refines.is_none())
        .map(|record| record.value.clone())
        .or_else(|| metadata.publication_date.clone());
    let modified = records
        .iter()
        .rev()
        .find(|record| {
            record.property.eq_ignore_ascii_case("dcterms:modified") && record.refines.is_none()
        })
        .map(|record| record.value.clone())
        .or_else(|| metadata.modified.clone());
    let description = records
        .iter()
        .rev()
        .find(|record| {
            property_matches(&record.property, "description") && record.refines.is_none()
        })
        .map(|record| record.value.clone())
        .or_else(|| metadata.description.clone());
    let contributors = records
        .iter()
        .filter(|record| {
            property_matches(&record.property, "contributor") && record.refines.is_none()
        })
        .map(|record| record.value.clone())
        .collect();

    let collection = records
        .iter()
        .filter(|record| {
            property_matches(&record.property, "belongs-to-collection")
                || property_matches(&record.property, "collection")
        })
        .map(|record| CollectionMetadata {
            name: record.value.clone(),
            collection_type: collection_refinement(records, record, "collection-type"),
            group_position: collection_refinement(records, record, "group-position"),
        })
        .collect();

    metadata.title = title;
    metadata.title_file_as = title_file_as;
    metadata.creators = creators;
    metadata.creator = creator;
    metadata.creator_file_as = creator_file_as;
    metadata.publisher = publisher;
    metadata.publisher_file_as = publisher_file_as;
    metadata.language = language;
    metadata.identifier = identifier;
    metadata.publication_date = publication_date;
    metadata.modified = modified;
    metadata.description = description;
    metadata.contributors = contributors;
    metadata.collection = collection;
}

fn collection_refinement(
    records: &[MetadataRecord],
    collection: &MetadataRecord,
    property: &str,
) -> Option<String> {
    let target = collection.refines.as_deref().or(collection.id.as_deref())?;
    refined_value(records, target, property).map(str::to_owned)
}

fn metadata_meta_kind(event: &BytesStart<'_>) -> Option<MetadataMetaField> {
    if is_primary_writing_mode_meta(event) {
        return Some(MetadataMetaField::PrimaryWritingMode);
    }
    let name = attr(event, "name");
    if name
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("cover"))
    {
        return Some(MetadataMetaField::Cover);
    }
    if name
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("fixed-layout"))
    {
        return Some(MetadataMetaField::FixedLayout);
    }
    if attr(event, "property").as_deref().is_some_and(|value| {
        value
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("rendition:layout"))
    }) {
        return Some(MetadataMetaField::RenditionLayout);
    }
    if attr(event, "property").as_deref().is_some_and(|value| {
        value
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("rendition:orientation"))
    }) {
        return Some(MetadataMetaField::RenditionOrientation);
    }
    if attr(event, "property").as_deref().is_some_and(|value| {
        value
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("rendition:spread"))
    }) {
        return Some(MetadataMetaField::RenditionSpread);
    }
    if attr(event, "property").as_deref().is_some_and(|value| {
        value
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("rendition:flow"))
    }) {
        return Some(MetadataMetaField::RenditionFlow);
    }
    if attr(event, "property").as_deref().is_some_and(|value| {
        value
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("rendition:viewport"))
    }) {
        return Some(MetadataMetaField::RenditionViewport);
    }
    if attr(event, "property").as_deref().is_some_and(|value| {
        value
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("rendition:align-x"))
    }) {
        return Some(MetadataMetaField::RenditionAlign);
    }
    match name.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("book-type") => Some(MetadataMetaField::BookType),
        Some("orientation-lock") => Some(MetadataMetaField::OrientationLock),
        Some("original-resolution") => Some(MetadataMetaField::OriginalResolution),
        _ => None,
    }
}

fn apply_metadata_meta(result: &mut ParsedOpf, field: MetadataMetaField, value: &str) {
    let value = value.trim();
    match field {
        MetadataMetaField::Cover => result.metadata.cover = Some(value.to_owned()),
        MetadataMetaField::FixedLayout => {
            if value.eq_ignore_ascii_case("true") {
                result.metadata.is_fixed_layout = true;
                result.publication_layout = Some(SpineLayout::PrePaginated);
            }
        }
        MetadataMetaField::RenditionLayout => {
            let layout = if value.eq_ignore_ascii_case("pre-paginated") {
                Some(SpineLayout::PrePaginated)
            } else if value.eq_ignore_ascii_case("reflowable") {
                Some(SpineLayout::Reflowable)
            } else {
                None
            };
            if let Some(layout) = layout {
                result.publication_layout = Some(layout);
                result.metadata.is_fixed_layout |= layout == SpineLayout::PrePaginated;
            }
        }
        MetadataMetaField::BookType => result.metadata.book_type = Some(value.to_owned()),
        MetadataMetaField::OrientationLock => {
            result.metadata.orientation_lock = Some(value.to_owned());
        }
        MetadataMetaField::RenditionOrientation => {
            result.metadata.orientation = Some(value.to_owned());
        }
        MetadataMetaField::RenditionSpread => {
            result.metadata.spread = Some(value.to_owned());
        }
        MetadataMetaField::RenditionFlow => {
            result.metadata.flow = Some(value.to_owned());
        }
        MetadataMetaField::RenditionViewport => {
            result.metadata.rendition_viewport = Some(value.to_owned());
        }
        MetadataMetaField::RenditionAlign => {
            result.metadata.align_x = Some(value.to_owned());
        }
        MetadataMetaField::OriginalResolution => {
            result.metadata.original_resolution = Some(value.to_owned());
        }
        MetadataMetaField::PrimaryWritingMode => {
            let normalized = value.to_ascii_lowercase();
            let parsed = if normalized == "horizontal-rl" {
                Some(WritingMode::HorizontalTb)
            } else {
                parse_writing_mode(value)
            };
            if result.primary_writing_mode.is_none() {
                if let Some(parsed) = parsed {
                    result.primary_writing_mode = Some(parsed);
                    result.metadata.primary_writing_mode = Some(value.to_owned());
                }
            }
        }
    }
}

fn parse_opf_start(event: &BytesStart<'_>, result: &mut ParsedOpf) {
    match local_name(event.name().as_ref()).as_str() {
        "package" => {
            result.unique_identifier_id = attr(event, "unique-identifier");
        }
        "item" => {
            let Some(id) = attr(event, "id") else { return };
            let Some(href) = attr(event, "href") else {
                return;
            };
            let Some(media_type) = attr(event, "media-type") else {
                return;
            };
            let properties = attr(event, "properties")
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            let fallback = attr(event, "fallback");
            let media_overlay = attr(event, "media-overlay");
            let item = ManifestItem {
                id: id.clone(),
                href,
                media_type,
                properties,
                fallback,
                media_overlay,
            };
            if item
                .media_type
                .eq_ignore_ascii_case("application/x-dtbncx+xml")
            {
                result.ncx_id = Some(id);
            }
            result.manifest.push(item);
        }
        "itemref" => {
            if let Some(idref) = attr(event, "idref") {
                let linear = attr(event, "linear")
                    .map(|value| value != "no")
                    .unwrap_or(true);
                let properties = attr(event, "properties")
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                result.spine.push(SpineItem {
                    idref,
                    linear,
                    properties,
                    media_overlay: attr(event, "media-overlay"),
                    layout: SpineLayout::default(),
                    rendition: RenditionSemantics::default(),
                });
            }
        }
        "spine" => {
            if let Some(value) = attr(event, "page-progression-direction") {
                result.page_progression = match value.as_str() {
                    "rtl" => PageProgression::Rtl,
                    "ltr" => PageProgression::Ltr,
                    _ => PageProgression::Default,
                };
            }
        }
        _ => {}
    }
}

fn rendition_from_metadata(metadata: &Metadata) -> Result<RenditionSemantics> {
    Ok(RenditionSemantics {
        orientation: metadata
            .orientation
            .as_deref()
            .map(parse_orientation)
            .transpose()?,
        spread: metadata.spread.as_deref().map(parse_spread).transpose()?,
        flow: metadata.flow.as_deref().map(parse_flow).transpose()?,
        align_x: metadata.align_x.as_deref().map(parse_align).transpose()?,
        page_spread: None,
    })
}

fn rendition_from_properties(properties: &[String]) -> Result<RenditionSemantics> {
    let mut result = RenditionSemantics::default();
    for property in properties {
        let normalized = property.to_ascii_lowercase();
        if let Some(value) = normalized.strip_prefix("rendition:orientation-") {
            result.orientation = Some(parse_orientation(value)?);
        } else if let Some(value) = normalized.strip_prefix("rendition:spread-") {
            result.spread = Some(parse_spread(value)?);
        } else if let Some(value) = normalized.strip_prefix("rendition:flow-") {
            result.flow = Some(parse_flow(value)?);
        } else if let Some(value) = normalized.strip_prefix("rendition:align-x-") {
            result.align_x = Some(parse_align(value)?);
        } else if normalized == "page-spread-left"
            || normalized == "rendition:page-spread-left"
            || normalized == "facing-page-left"
        {
            result.page_spread = Some(PageSpread::Left);
        } else if normalized == "page-spread-right"
            || normalized == "rendition:page-spread-right"
            || normalized == "facing-page-right"
        {
            result.page_spread = Some(PageSpread::Right);
        } else if normalized == "page-spread-center" || normalized == "rendition:page-spread-center"
        {
            result.page_spread = Some(PageSpread::Center);
        }
    }
    Ok(result)
}

fn parse_orientation(value: &str) -> Result<RenditionOrientation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(RenditionOrientation::Auto),
        "portrait" => Ok(RenditionOrientation::Portrait),
        "landscape" => Ok(RenditionOrientation::Landscape),
        value => Err(Error::UnsupportedEpub(format!(
            "unsupported rendition:orientation value {value}"
        ))),
    }
}

fn parse_spread(value: &str) -> Result<RenditionSpread> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(RenditionSpread::Auto),
        "none" => Ok(RenditionSpread::None),
        "landscape" => Ok(RenditionSpread::Landscape),
        "portrait" => Ok(RenditionSpread::Portrait),
        "both" => Ok(RenditionSpread::Both),
        value => Err(Error::UnsupportedEpub(format!(
            "unsupported rendition:spread value {value}"
        ))),
    }
}

fn parse_flow(value: &str) -> Result<RenditionFlow> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(RenditionFlow::Auto),
        "paginated" => Ok(RenditionFlow::Paginated),
        "scrolled-continuous" => Ok(RenditionFlow::ScrolledContinuous),
        "scrolled-doc" => Ok(RenditionFlow::ScrolledDoc),
        value => Err(Error::UnsupportedEpub(format!(
            "unsupported rendition:flow value {value}"
        ))),
    }
}

fn parse_align(value: &str) -> Result<RenditionAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "center" => Ok(RenditionAlign::Center),
        value => Err(Error::UnsupportedEpub(format!(
            "unsupported rendition:align-x value {value}"
        ))),
    }
}

pub(super) fn has_property(item: &ManifestItem, property: &str) -> bool {
    item.properties
        .iter()
        .any(|value| value.eq_ignore_ascii_case(property))
}

pub(super) fn cover_image_paths(parsed: &ParsedOpf, opf_base: &Path) -> HashSet<String> {
    parsed
        .manifest
        .iter()
        .filter(|item| {
            has_property(item, "cover-image")
                || (parsed.metadata.cover.as_deref() == Some(item.id.as_str())
                    && item.media_type.to_ascii_lowercase().starts_with("image/"))
        })
        .map(|item| resolve_href(opf_base, &item.href))
        .collect()
}

pub(super) fn is_legacy_svg_cover_document(
    item: &ManifestItem,
    semantic: &SemanticDocument,
    first_spine_id: Option<&String>,
    cover_image_paths: &HashSet<String>,
    opf_base: &Path,
) -> bool {
    first_spine_id == Some(&item.id)
        && (has_property(item, "svg")
            || has_property(item, "cover")
            || has_property(item, "cover-document"))
        && !cover_image_paths.is_empty()
        && semantic.image_references.iter().any(|reference| {
            let document_path = resolve_href(opf_base, &item.href);
            let document_base = Path::new(&document_path)
                .parent()
                .unwrap_or_else(|| Path::new(""));
            let (reference_path, _) = split_link_suffix(reference);
            cover_image_paths.contains(&resolve_href(document_base, reference_path))
        })
}

pub(super) fn local_name(name: &[u8]) -> String {
    local_name_ref(name).to_ascii_lowercase()
}

pub(super) fn local_name_ref(name: &[u8]) -> &str {
    let name = std::str::from_utf8(name).unwrap_or_default();
    name.rsplit(':').next().unwrap_or(name)
}

pub(super) fn attr(event: &BytesStart<'_>, wanted: &str) -> Option<String> {
    event.attributes().flatten().find_map(|attribute| {
        if local_name(attribute.key.as_ref()) == wanted {
            attribute
                .unescape_value()
                .ok()
                .map(|value| value.into_owned())
        } else {
            None
        }
    })
}
