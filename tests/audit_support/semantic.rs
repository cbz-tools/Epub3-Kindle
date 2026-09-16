//! Independent source semantic model and KF8 target projection.
//!
//! This module deliberately uses only the public ZIP input and the bytes
//! emitted by the public conversion API.  It does not import production EPUB,
//! Kindle, CSS, or MOBI helpers.  The small lexical scanner is intentionally
//! an audit-side projection, not a second implementation of the converter.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Cursor;

use flate2::read::ZlibDecoder;
use std::io::Read;
use zip::ZipArchive;

use super::palm::{MobiHeader, PalmDb};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavItem {
    pub label: String,
    pub href: String,
    pub children: Vec<NavItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSection {
    pub href: String,
    pub visible_text: String,
    pub ids: Vec<String>,
    pub links: Vec<(String, String)>,
    pub images: Vec<(String, String)>,
    pub tags: Vec<String>,
    pub lang: Option<String>,
    pub direction: Option<String>,
    pub ruby: Vec<(String, String)>,
    pub stylesheet_hrefs: Vec<String>,
    pub source_properties: BTreeSet<String>,
    pub fixed: bool,
    pub viewport: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssExpectation {
    pub href: String,
    pub source: String,
    pub imports: Vec<String>,
    pub urls: Vec<String>,
    pub declarations: Vec<(String, String)>,
    pub media_branches: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SourceModel {
    pub title: String,
    pub identifier: String,
    pub language: String,
    pub publication_date: Option<String>,
    pub modified: String,
    pub progression: String,
    pub rendition_layout: Option<String>,
    pub orientation: Option<String>,
    pub spread: Option<String>,
    pub viewport: Option<String>,
    pub original_resolution: Option<String>,
    pub sections: Vec<SourceSection>,
    pub resources: BTreeMap<String, (String, Vec<u8>)>,
    pub css: Vec<CssExpectation>,
    pub toc: Vec<NavItem>,
    pub landmarks: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSection {
    pub visible_text: String,
    pub ids: BTreeSet<String>,
    pub links: Vec<(String, String)>,
    pub images: Vec<String>,
    pub image_alts: Vec<String>,
    pub stylesheet_hrefs: Vec<String>,
    pub tags: Vec<String>,
    pub lang: Option<String>,
    pub direction: Option<String>,
    pub fixed: bool,
    pub ruby: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct TargetProjection<'a> {
    pub db: PalmDb<'a>,
    pub header: MobiHeader<'a>,
    pub rawml: String,
    pub sections: Vec<TargetSection>,
    pub css: String,
    pub embedded_resource_numbers: Vec<usize>,
    pub font_records: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetNcxEntry {
    pub label: String,
    pub depth: Option<u32>,
    pub parent: Option<u32>,
    pub sequence: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticNode {
    pub tag: String,
    pub alt: Option<String>,
    pub children: Vec<SemanticNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEdge {
    pub owner: String,
    pub target: String,
    pub kind: String,
}

/// Reconstruct the supported structural subset without relying on the
/// converter's DOM/parser.  Attributes whose values are transport-rewritten
/// are intentionally excluded; image alternative text remains part of the
/// semantic node.
pub fn semantic_tree(source: &str) -> Vec<SemanticNode> {
    let mut roots = Vec::new();
    let mut stack: Vec<SemanticNode> = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('<') {
        let start = cursor + relative;
        let Some(end_relative) = source[start..].find('>') else {
            break;
        };
        let end = start + end_relative + 1;
        let tag = &source[start..end];
        if tag.starts_with("<!--") || tag.starts_with("<?") || tag.starts_with("<!") {
            cursor = end;
            continue;
        }
        let name = tag_name(tag);
        if name.is_empty() {
            cursor = end;
            continue;
        }
        if tag.starts_with("</") {
            if let Some(node) = stack.pop() {
                if stack.is_empty() {
                    roots.push(node);
                } else if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
        } else if tag.ends_with("/>")
            || matches!(
                name.as_str(),
                "area"
                    | "br"
                    | "col"
                    | "embed"
                    | "hr"
                    | "img"
                    | "input"
                    | "link"
                    | "meta"
                    | "param"
                    | "source"
                    | "track"
                    | "wbr"
            )
        {
            let node = SemanticNode {
                tag: name.clone(),
                alt: (name == "img").then(|| attr(tag, "alt").unwrap_or_default()),
                children: Vec::new(),
            };
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else {
                roots.push(node);
            }
        } else {
            stack.push(SemanticNode {
                tag: name,
                alt: None,
                children: Vec::new(),
            });
        }
        cursor = end;
    }
    while let Some(node) = stack.pop() {
        if stack.is_empty() {
            roots.push(node);
        } else if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
        }
    }
    roots
}

pub fn source_semantic_tree(epub: &[u8], path: &str) -> Result<Vec<SemanticNode>, String> {
    let source = source_document(epub, path)?;
    let body = element_body(&source).ok_or("source body missing")?;
    Ok(semantic_tree(&body))
}

pub fn source_document(epub: &[u8], path: &str) -> Result<String, String> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    archive
        .by_name(path)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(text(&bytes))
}

pub fn source_local_resource_edges(epub: &[u8]) -> Result<Vec<ResourceEdge>, String> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        files.insert(file.name().to_owned(), bytes);
    }
    let opf_path = files
        .keys()
        .find(|path| path.ends_with(".opf"))
        .cloned()
        .ok_or("source OPF missing")?;
    let opf = text(files.get(&opf_path).ok_or("source OPF bytes missing")?);
    let opf_base = parent(&opf_path);
    let manifest = tags_named(&opf, "item")
        .into_iter()
        .filter_map(|tag| {
            Some((
                resolve(&opf_base, &attr(&tag, "href")?),
                attr(&tag, "media-type")?,
            ))
        })
        .collect::<Vec<_>>();
    let mut edges = Vec::new();
    for (href, media_type) in manifest {
        let Some(bytes) = files.get(&href) else {
            continue;
        };
        if media_type.eq_ignore_ascii_case("text/css") {
            let css = text(bytes);
            for raw in all_css_urls(&css) {
                if let Some(target) = local_edge_target(&parent(&href), &raw) {
                    edges.push(ResourceEdge {
                        owner: href.clone(),
                        target,
                        kind: "css-url".to_owned(),
                    });
                }
            }
            continue;
        }
        if !media_type.eq_ignore_ascii_case("application/xhtml+xml")
            && !media_type.eq_ignore_ascii_case("text/html")
            && !media_type.eq_ignore_ascii_case("image/svg+xml")
        {
            continue;
        }
        let source = text(bytes);
        for tag in tags_named(&source, "*") {
            let name = tag_name(&tag);
            let attributes = match name.as_str() {
                "link" => attr(&tag, "rel")
                    .filter(|rel| {
                        rel.split_whitespace()
                            .any(|value| value.eq_ignore_ascii_case("stylesheet"))
                    })
                    .and_then(|_| attr(&tag, "href").map(|value| vec![("stylesheet", value)])),
                "img" => attr(&tag, "src").map(|value| vec![("image", value)]),
                "image" | "use" => {
                    let mut values = Vec::new();
                    if let Some(value) = attr(&tag, "href") {
                        values.push(("svg", value));
                    }
                    if let Some(value) = attr(&tag, "xlink:href") {
                        values.push(("svg", value));
                    }
                    (!values.is_empty()).then_some(values)
                }
                "object" => attr(&tag, "data").map(|value| vec![("object", value)]),
                _ => None,
            };
            for (kind, raw) in attributes.into_iter().flatten() {
                if let Some(target) = local_edge_target(&parent(&href), &raw) {
                    edges.push(ResourceEdge {
                        owner: href.clone(),
                        target,
                        kind: kind.to_owned(),
                    });
                }
            }
        }
    }
    Ok(edges)
}

fn local_edge_target(base: &str, raw: &str) -> Option<String> {
    let path = raw.split(['#', '?']).next().unwrap_or(raw);
    if path.is_empty()
        || path.starts_with("data:")
        || path.starts_with("http:")
        || path.starts_with("https:")
        || path.starts_with("//")
    {
        return None;
    }
    Some(resolve(base, path))
}

impl SourceModel {
    pub fn parse(epub: &[u8]) -> Result<Self, String> {
        let mut archive = ZipArchive::new(Cursor::new(epub)).map_err(|e| e.to_string())?;
        let mut files = BTreeMap::new();
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).map_err(|e| e.to_string())?;
            let name = file.name().to_owned();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            files.insert(name, bytes);
        }
        let opf_path = files
            .keys()
            .find(|path| path.to_ascii_lowercase().ends_with(".opf"))
            .cloned()
            .ok_or("source Package Document missing")?;
        let opf = text(files.get(&opf_path).ok_or("source OPF bytes missing")?);
        let base = parent(&opf_path);
        let title = element_text(&opf, "dc:title").ok_or("source title missing")?;
        let language = element_text(&opf, "dc:language").ok_or("source language missing")?;
        let publication_date = element_text(&opf, "dc:date");
        let modified = meta_property(&opf, "dcterms:modified").ok_or("source modified missing")?;
        let identifier = element_with_attr(&opf, "dc:identifier", "id", "pub-id")
            .or_else(|| element_text(&opf, "dc:identifier"))
            .ok_or("source identifier missing")?;
        let progression = tag_attr(&opf, "spine", "page-progression-direction")
            .unwrap_or_else(|| "default".to_owned());
        let rendition_layout = meta_property(&opf, "rendition:layout");
        let orientation = meta_property(&opf, "rendition:orientation");
        let spread = meta_property(&opf, "rendition:spread");
        let viewport = meta_property(&opf, "rendition:viewport");
        let original_resolution = meta_name(&opf, "original-resolution");

        let manifest = tags_named(&opf, "item")
            .into_iter()
            .filter_map(|tag| {
                let id = attr(&tag, "id")?;
                let href = attr(&tag, "href")?;
                let media = attr(&tag, "media-type")?;
                let properties = attr(&tag, "properties").unwrap_or_default();
                Some((id, resolve(&base, &href), media, properties))
            })
            .collect::<Vec<_>>();
        let by_id = manifest
            .iter()
            .map(|(id, href, media, properties)| {
                (
                    id.clone(),
                    (href.clone(), media.clone(), properties.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        let spine = tags_named(&opf, "itemref")
            .into_iter()
            .filter_map(|tag| {
                let idref = attr(&tag, "idref")?;
                let (href, media, properties) = by_id.get(&idref)?.clone();
                let item_properties = attr(&tag, "properties").unwrap_or_default();
                Some((href, media, format!("{properties} {item_properties}")))
            })
            .collect::<Vec<_>>();

        let mut resources = BTreeMap::new();
        for (_, href, media, _) in &manifest {
            if let Some(bytes) = files.get(href) {
                resources.insert(href.clone(), (media.clone(), bytes.clone()));
            }
        }
        let nav_href = manifest
            .iter()
            .find(|(_, _, _, properties)| properties.split_whitespace().any(|p| p == "nav"))
            .map(|(_, href, _, _)| href.clone())
            .ok_or("source navigation document missing")?;
        let nav = text(files.get(&nav_href).ok_or("source nav bytes missing")?);
        let toc = nav_section(&nav, "toc")
            .map(|body| parse_ol(body, &base))
            .unwrap_or_default();
        let landmarks = nav_section(&nav, "landmarks")
            .map(|body| {
                tags_named(body, "a")
                    .into_iter()
                    .filter_map(|tag| {
                        let kind = attr(&tag, "epub:type")?;
                        let href = attr(&tag, "href")?;
                        Some((kind, resolve(&base, &href)))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let global_fixed = rendition_layout.as_deref() == Some("pre-paginated");
        let mut sections = Vec::new();
        for (href, media, properties) in spine {
            if !media.eq_ignore_ascii_case("application/xhtml+xml")
                && !media.eq_ignore_ascii_case("text/html")
            {
                continue;
            }
            let source = text(files.get(&href).ok_or("spine document bytes missing")?);
            let html_tag = tags_named(&source, "html")
                .into_iter()
                .next()
                .unwrap_or_default();
            let body = element_body(&source).unwrap_or_else(|| source.clone());
            let mut section = parse_section(&href, &source, &body, &base);
            section.fixed = global_fixed
                || properties
                    .split_whitespace()
                    .any(|p| p.eq_ignore_ascii_case("rendition:layout-pre-paginated"));
            section.viewport = meta_name(&source, "viewport").or_else(|| viewport.clone());
            section.lang = attr(&html_tag, "lang").or_else(|| attr(&html_tag, "xml:lang"));
            section.direction = tags_named(&source, "body")
                .into_iter()
                .next()
                .and_then(|tag| attr(&tag, "dir"));
            section.stylesheet_hrefs = tags_named(&source, "link")
                .into_iter()
                .filter(|tag| {
                    attr(tag, "rel")
                        .unwrap_or_default()
                        .split_whitespace()
                        .any(|value| value.eq_ignore_ascii_case("stylesheet"))
                })
                .filter_map(|tag| {
                    attr(&tag, "href").map(|href| resolve_document(&section.href, &href))
                })
                .collect();
            section.source_properties = properties
                .split_whitespace()
                .filter(|property| !property.is_empty())
                .map(str::to_owned)
                .collect();
            sections.push(section);
        }
        let css = manifest
            .iter()
            .filter(|(_, _, media, _)| media.eq_ignore_ascii_case("text/css"))
            .filter_map(|(_, href, _, _)| files.get(href).map(|bytes| (href.clone(), text(bytes))))
            .map(|(href, source)| CssExpectation::from_source(&href, &source))
            .collect();
        Ok(Self {
            title,
            identifier,
            language,
            publication_date,
            modified,
            progression,
            rendition_layout,
            orientation,
            spread,
            viewport,
            original_resolution,
            sections,
            resources,
            css,
            toc,
            landmarks,
        })
    }
}

impl CssExpectation {
    fn from_source(href: &str, source: &str) -> Self {
        let imports = css_urls_after(source, "@import")
            .into_iter()
            .map(|u| resolve(&parent(href), &u))
            .collect();
        let urls = all_css_urls(source)
            .into_iter()
            .filter(|u| !u.starts_with("data:") && !u.starts_with("http:"))
            .map(|u| resolve(&parent(href), &u))
            .collect();
        let mut declarations = Vec::new();
        for property in [
            "writing-mode",
            "-webkit-writing-mode",
            "text-combine-upright",
            "-webkit-text-emphasis-style",
            "-webkit-text-orientation",
            "font-family",
            "font-size",
            "line-height",
            "color",
            "background",
        ] {
            for value in declarations_for(source, property) {
                declarations.push((property.to_owned(), value));
            }
        }
        let media_branches = ["amzn-kf8", "amzn-mobi"]
            .into_iter()
            .filter(|branch| source.to_ascii_lowercase().contains(branch))
            .map(str::to_owned)
            .collect();
        Self {
            href: href.to_owned(),
            source: source.to_owned(),
            imports,
            urls,
            declarations,
            media_branches,
        }
    }
}

impl<'a> TargetProjection<'a> {
    pub fn parse(output: &'a [u8]) -> Result<Self, String> {
        Self::parse_at(output, 0)
    }

    /// Project a selected MOBI section.  Dual files contain an independent
    /// KF7 Record 0 followed by a KF8 Record 0; the audit must not mistake the
    /// compatibility stub for the canonical KF8 semantic projection.
    pub fn parse_at(output: &'a [u8], record_index: usize) -> Result<Self, String> {
        let db = PalmDb::parse(output)?;
        let header = db.mobi_header(record_index)?;
        let raw_bytes = super::palm::reconstruct_text_raw(&db, &header)?;
        let rawml = text(&raw_bytes);
        let html_starts = rawml
            .match_indices("<html")
            .map(|(p, _)| p)
            .collect::<Vec<_>>();
        let css_start = find_css_tail_start(&rawml);
        let mut sections = Vec::new();
        for (index, start) in html_starts.iter().copied().enumerate() {
            let end = html_starts
                .get(index + 1)
                .copied()
                .or(css_start)
                .unwrap_or(rawml.len());
            let segment = rawml[start..end].to_owned();
            sections.push(parse_target_section(&segment));
        }
        let css = css_start
            .map(|start| rawml[start..].to_owned())
            .unwrap_or_default();
        let mut embedded_resource_numbers = Vec::new();
        let mut font_records = Vec::new();
        for number in find_embed_numbers(&rawml) {
            embedded_resource_numbers.push(number);
        }
        for index in 0..db.record_count() {
            let record = db.record(index)?;
            if record.starts_with(b"FONT") {
                font_records.push(index);
            }
        }
        Ok(Self {
            db,
            header,
            rawml,
            sections,
            css,
            embedded_resource_numbers,
            font_records,
        })
    }

    pub fn body_text(&self) -> String {
        self.sections
            .iter()
            .map(|s| s.visible_text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn semantic_tree(&self, section: usize) -> Result<Vec<SemanticNode>, String> {
        let start = self
            .rawml
            .match_indices("<html")
            .nth(section)
            .map(|(offset, _)| offset)
            .ok_or("target section missing")?;
        let end = self
            .rawml
            .match_indices("<html")
            .nth(section + 1)
            .map(|(offset, _)| offset)
            .or_else(|| find_css_tail_start(&self.rawml))
            .unwrap_or(self.rawml.len());
        let segment = &self.rawml[start..end];
        let body_end = segment
            .find("</html>")
            .ok_or("target section html end missing")?
            + "</html>".len();
        Ok(semantic_tree(&segment[body_end..]))
    }

    pub fn exth_text(&self, ty: u32) -> Option<String> {
        self.header.exth_text(ty)
    }

    pub fn resc_metadata(&self) -> Result<BTreeMap<String, String>, String> {
        let xml = self.resc_xml()?;
        let mut values = BTreeMap::new();
        for tag in tags_named(&xml, "meta") {
            if let Some(property) = attr(&tag, "property") {
                let start = xml.find(&tag).ok_or("RESC metadata position missing")?;
                let end = matching_tag_end(&xml, start, "meta").unwrap_or(start + tag.len());
                values.insert(property, visible_text(&xml[start + tag.len()..end]));
            }
        }
        Ok(values)
    }

    pub fn resc_spine_properties(&self) -> Result<Vec<(String, BTreeSet<String>, bool)>, String> {
        let xml = self.resc_xml()?;
        Ok(tags_named(&xml, "itemref")
            .into_iter()
            .filter_map(|tag| {
                let idref = attr(&tag, "idref")?;
                let properties = attr(&tag, "properties")
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                let linear = attr(&tag, "linear")
                    .map(|value| !value.eq_ignore_ascii_case("no"))
                    .unwrap_or(true);
                Some((idref, properties, linear))
            })
            .collect())
    }

    pub fn resc_resource_hrefs(&self) -> Result<Vec<String>, String> {
        let xml = self.resc_xml()?;
        Ok(tags_named(&xml, "*")
            .into_iter()
            .filter(|tag| matches!(tag_name(tag).as_str(), "item" | "resource" | "link"))
            .filter_map(|tag| {
                attr(&tag, "href")
                    .or_else(|| attr(&tag, "src"))
                    .or_else(|| attr(&tag, "resource"))
            })
            .collect())
    }

    fn resc_xml(&self) -> Result<String, String> {
        let record = (0..self.db.record_count())
            .find_map(|index| {
                let record = self.db.record(index).ok()?;
                record.starts_with(b"RESC").then_some(record)
            })
            .ok_or("RESC record missing")?;
        let header_length = be32(record, 12)? as usize;
        let xml_start = 16usize
            .checked_add(header_length)
            .ok_or("RESC header overflow")?;
        let payload = record.get(xml_start..).ok_or("RESC XML outside record")?;
        let xml_start = payload
            .windows(5)
            .position(|window| window == b"<?xml")
            .ok_or("RESC XML declaration missing")?;
        Ok(text(&payload[xml_start..]))
    }

    pub fn embedded_bytes(&self, number: usize) -> Result<&'a [u8], String> {
        let dual_shared_resources = self.header.version >= 8
            && self.header.record_index > 0
            && self
                .db
                .mobi_header(0)
                .is_ok_and(|header| header.version < 8 && header.first_resource != u32::MAX);
        let coordinate = if dual_shared_resources {
            "global"
        } else if self.header.version >= 8 && self.header.record_index > 0 {
            "section-relative"
        } else {
            "global"
        };
        let first = if dual_shared_resources {
            // Dual MOBI shares the binary resource pool before the KF8
            // section.  The KF8 header keeps a section-local FDST anchor for
            // resource numbering, while embed URIs still address that shared
            // pool; recover the global base from the independently parsed KF7
            // Record 0.
            self.db.mobi_header(0)?.first_resource
        } else if self.header.version >= 8 && self.header.record_index > 0 {
            self.header
                .fdst_record
                .ok_or("resource table anchor missing")?
                .checked_add(1)
                .ok_or("resource anchor overflow")?
        } else {
            self.header.first_resource
        };
        let first = self.header.global_record_index(first, coordinate)?;
        self.db.record(
            first
                .checked_add(number.checked_sub(1).ok_or("embed number zero")?)
                .ok_or("embed index overflow")?,
        )
    }

    pub fn resource_bytes(&self, number: usize) -> Result<&'a [u8], String> {
        let first = (self
            .header
            .fdst_record
            .ok_or("resource table anchor missing")? as usize)
            .checked_add(1)
            .ok_or("resource index overflow")?;
        let first = if self.header.version >= 8 && self.header.record_index > 0 {
            self.header.record_index + first
        } else {
            first
        };
        self.db.record(
            first
                .checked_add(number.checked_sub(1).ok_or("resource number zero")?)
                .ok_or("resource index overflow")?,
        )
    }

    pub fn image_records(&self) -> Vec<usize> {
        (0..self.db.record_count())
            .filter(|&i| {
                self.db
                    .record(i)
                    .map(|r| {
                        r.starts_with(&[0xff, 0xd8, 0xff])
                            || r.starts_with(&[0x89, b'P', b'N', b'G'])
                    })
                    .unwrap_or(false)
            })
            .collect()
    }

    pub fn decompressed_font(&self, index: usize) -> Result<Vec<u8>, String> {
        let record = self.db.record(index)?;
        if is_sfnt(record) {
            return Ok(record.to_vec());
        }
        if record.len() < 24 || !record.starts_with(b"FONT") {
            return Err("not a bounded FONT record".into());
        }
        let expected = u32::from_be_bytes(record[4..8].try_into().unwrap()) as usize;
        let mut out = Vec::new();
        ZlibDecoder::new(&record[24..])
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
        if out.len() != expected {
            return Err("FONT length disagrees with payload".into());
        }
        Ok(out)
    }

    /// Decode the emitted NCX main/detail/CTOC records without consulting the
    /// production index implementation.  Values use the documented KF8 VWI
    /// representation and the target's own TAGX descriptors.
    pub fn ncx_entries(&self) -> Result<Vec<TargetNcxEntry>, String> {
        let coordinate = if self.header.version >= 8 && self.header.record_index > 0 {
            "section-relative"
        } else {
            "global"
        };
        let main_index = self.header.global_record_index(
            self.header.ncx_record.ok_or("KF8 NCX pointer missing")?,
            coordinate,
        )?;
        let main = self.db.record(main_index)?;
        let detail_count = be32(main, 24)? as usize;
        let ctoc_count = be32(main, 52)? as usize;
        let tagx = be32(main, 4)? as usize;
        if tagx + 12 > main.len() || &main[tagx..tagx + 4] != b"TAGX" {
            return Err("NCX TAGX header missing".into());
        }
        let tagx_len = be32(main, tagx + 4)? as usize;
        let mut defs = Vec::new();
        let mut p = tagx + 12;
        while p + 4 <= tagx + tagx_len && p + 4 <= main.len() {
            let tag = main[p];
            let values = main[p + 1];
            let mask = main[p + 2];
            if tag == 0 && values == 0 && mask == 0 {
                break;
            }
            defs.push((tag, values, mask));
            p += 4;
        }
        let mut rows = Vec::new();
        for detail_offset in 0..detail_count {
            let detail = self.db.record(main_index + 1 + detail_offset)?;
            let count = be32(detail, 24)? as usize;
            let idxt = be32(detail, 20)? as usize;
            if idxt + 4 + count * 2 > detail.len() || &detail[idxt..idxt + 4] != b"IDXT" {
                return Err("NCX detail IDXT out of range".into());
            }
            for row in 0..count {
                let start = u16::from_be_bytes(
                    detail[idxt + 4 + row * 2..idxt + 6 + row * 2]
                        .try_into()
                        .unwrap(),
                ) as usize;
                let (text, values) = decode_index_row(detail, start, &defs)?;
                rows.push((text, values));
            }
        }
        let mut out = Vec::new();
        for (_, values) in rows {
            let label_offset = values
                .get(&3)
                .and_then(|v| v.first())
                .copied()
                .ok_or("NCX label offset missing")? as usize;
            let label =
                self.ctoc_string(main_index + 1 + detail_count, ctoc_count, label_offset)?;
            let coords = values.get(&6).ok_or("NCX position missing")?;
            if coords.len() != 2 {
                return Err("NCX position must carry sequence and offset".into());
            }
            out.push(TargetNcxEntry {
                label,
                depth: values.get(&4).and_then(|v| v.first()).copied(),
                parent: values.get(&21).and_then(|v| v.first()).copied(),
                sequence: coords[0],
                offset: coords[1],
            });
        }
        Ok(out)
    }

    pub fn guide_entries(&self) -> Result<Vec<(String, String, u32, u32)>, String> {
        let coordinate = if self.header.version >= 8 && self.header.record_index > 0 {
            "section-relative"
        } else {
            "global"
        };
        let main_index = self.header.global_record_index(
            self.header.guide_index.ok_or("KF8 Guide pointer missing")?,
            coordinate,
        )?;
        let main = self.db.record(main_index)?;
        let detail_count = be32(main, 24)? as usize;
        let ctoc_count = be32(main, 52)? as usize;
        let tagx = be32(main, 4)? as usize;
        if tagx + 12 > main.len() || &main[tagx..tagx + 4] != b"TAGX" {
            return Err("Guide TAGX header missing".into());
        }
        let tagx_len = be32(main, tagx + 4)? as usize;
        let mut defs = Vec::new();
        let mut p = tagx + 12;
        while p + 4 <= tagx + tagx_len && p + 4 <= main.len() {
            let tag = main[p];
            let values = main[p + 1];
            let mask = main[p + 2];
            if tag == 0 && values == 0 && mask == 0 {
                break;
            }
            defs.push((tag, values, mask));
            p += 4;
        }
        let mut out = Vec::new();
        for detail_offset in 0..detail_count {
            let detail = self.db.record(main_index + 1 + detail_offset)?;
            let count = be32(detail, 24)? as usize;
            let idxt = be32(detail, 20)? as usize;
            if idxt + 4 + count * 2 > detail.len() || &detail[idxt..idxt + 4] != b"IDXT" {
                return Err("Guide detail IDXT out of range".into());
            }
            for row in 0..count {
                let start = u16::from_be_bytes(
                    detail[idxt + 4 + row * 2..idxt + 6 + row * 2]
                        .try_into()
                        .unwrap(),
                ) as usize;
                let (kind, values) = decode_index_row(detail, start, &defs)?;
                let label_offset = values
                    .get(&1)
                    .and_then(|v| v.first())
                    .copied()
                    .ok_or("Guide label offset missing")?
                    as usize;
                let label =
                    self.ctoc_string(main_index + 1 + detail_count, ctoc_count, label_offset)?;
                let coords = values.get(&6).ok_or("Guide position missing")?;
                if coords.len() != 2 {
                    return Err("Guide position must carry sequence and offset".into());
                }
                out.push((kind, label, coords[0], coords[1]));
            }
        }
        Ok(out)
    }

    fn ctoc_string(&self, start: usize, count: usize, offset: usize) -> Result<String, String> {
        let page = offset / 0x1_0000;
        let within = offset % 0x1_0000;
        if page >= count {
            return Err("NCX CTOC page outside advertised count".into());
        }
        let record = self.db.record(start + page)?;
        let length = decode_vwi(record, within)?.0 as usize;
        let data_start = within + decode_vwi(record, within)?.1;
        let end = data_start
            .checked_add(length)
            .ok_or("NCX CTOC string overflow")?;
        if end > record.len() {
            return Err("NCX CTOC string outside record".into());
        }
        String::from_utf8(record[data_start..end].to_vec()).map_err(|e| e.to_string())
    }
}

fn find_css_tail_start(rawml: &str) -> Option<usize> {
    let html_end = rawml.rfind("</html>")? + "</html>".len();
    let tail = &rawml[html_end..];
    let import = tail.find("@import");
    let rule = tail.find('{').and_then(|open| {
        let close = tail[open + 1..].find('}')? + open + 1;
        tail[open + 1..close]
            .contains(':')
            .then(|| tail[..open].rfind('>').map(|end| end + 1).unwrap_or(open))
    });
    import
        .into_iter()
        .chain(rule)
        .min()
        .map(|offset| html_end + offset)
}

fn parse_section(href: &str, source: &str, body: &str, base: &str) -> SourceSection {
    let tags = tags_named(body, "*");
    let ids = tags.iter().filter_map(|tag| attr(tag, "id")).collect();
    let links = tags
        .iter()
        .filter(|tag| tag_name(tag) == "a")
        .filter_map(|tag| {
            let href_value = attr(tag, "href")?;
            let end = matching_tag_end(body, body.find(tag)?, "a").unwrap_or(body.len());
            let label = visible_text(&body[body.find(tag)?..end]);
            Some((resolve_document(href, &href_value), label))
        })
        .collect();
    let images = tags
        .iter()
        .filter(|tag| tag_name(tag) == "img")
        .filter_map(|tag| {
            Some((
                resolve_document(href, &attr(tag, "src")?),
                attr(tag, "alt").unwrap_or_default(),
            ))
        })
        .collect();
    let stylesheet_hrefs = tags
        .iter()
        .filter(|tag| {
            tag_name(tag) == "link"
                && attr(tag, "rel")
                    .unwrap_or_default()
                    .split_whitespace()
                    .any(|v| v == "stylesheet")
        })
        .filter_map(|tag| attr(tag, "href").map(|v| resolve_document(href, &v)))
        .collect();
    let tags_seen = tags
        .iter()
        .map(|tag| tag_name(tag))
        .filter(|name| !name.is_empty())
        .collect();
    let lang = tags
        .iter()
        .find(|tag| tag_name(tag) == "html")
        .and_then(|tag| attr(tag, "lang").or_else(|| attr(tag, "xml:lang")));
    let direction = tags
        .iter()
        .find(|tag| tag_name(tag) == "body")
        .and_then(|tag| attr(tag, "dir"));
    let ruby = ruby_pairs(body);
    let _ = source;
    let _ = base;
    SourceSection {
        href: href.to_owned(),
        visible_text: visible_text(body),
        ids,
        links,
        images,
        tags: tags_seen,
        lang,
        direction,
        ruby,
        stylesheet_hrefs,
        source_properties: BTreeSet::new(),
        fixed: false,
        viewport: None,
    }
}

fn parse_target_section(body: &str) -> TargetSection {
    let tags = tags_named(body, "*");
    let ids = tags.iter().filter_map(|tag| attr(tag, "id")).collect();
    let links = tags
        .iter()
        .filter(|tag| tag_name(tag) == "a")
        .filter_map(|tag| {
            let href = attr(tag, "href")?;
            let start = body.find(tag)?;
            let end = matching_tag_end(body, start, "a").unwrap_or(body.len());
            Some((href, visible_text(&body[start..end])))
        })
        .collect();
    let image_tags = tags
        .iter()
        .filter(|tag| tag_name(tag) == "img")
        .collect::<Vec<_>>();
    let images = image_tags
        .iter()
        .filter_map(|tag| attr(tag, "src"))
        .collect();
    let image_alts = image_tags
        .iter()
        .map(|tag| attr(tag, "alt").unwrap_or_default())
        .collect();
    let stylesheet_hrefs = tags
        .iter()
        .filter(|tag| {
            tag_name(tag) == "link"
                && attr(tag, "rel")
                    .unwrap_or_default()
                    .split_whitespace()
                    .any(|value| value.eq_ignore_ascii_case("stylesheet"))
        })
        .filter_map(|tag| attr(tag, "href"))
        .collect();
    let tags_seen = tags
        .iter()
        .map(|tag| tag_name(tag))
        .filter(|name| !name.is_empty())
        .collect();
    let html = tags.iter().find(|tag| tag_name(tag) == "html");
    let lang = html.and_then(|tag| attr(tag, "lang").or_else(|| attr(tag, "xml:lang")));
    let direction = tags
        .iter()
        .find(|tag| tag_name(tag) == "body")
        .and_then(|tag| attr(tag, "dir"));
    TargetSection {
        visible_text: visible_text(body),
        ids,
        links,
        images,
        image_alts,
        stylesheet_hrefs,
        tags: tags_seen,
        lang,
        direction,
        fixed: body.contains("?mime=image/svg+xml"),
        ruby: ruby_pairs(body),
    }
}

fn parse_ol(body: &str, base: &str) -> Vec<NavItem> {
    let mut output = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = body[cursor..].find("<li") {
        let start = cursor + relative;
        let Some(tag_end) = body[start..].find('>').map(|p| start + p + 1) else {
            break;
        };
        if body[start..tag_end].starts_with("</") {
            cursor = tag_end;
            continue;
        }
        let Some(end) = matching_tag_end(body, start, "li") else {
            break;
        };
        let item = &body[start..end];
        let anchor = tags_named(item, "a").into_iter().next();
        let (label, href) = anchor
            .map(|a| {
                let p = item.find(&a).unwrap_or(0);
                let q = matching_tag_end(item, p, "a").unwrap_or(item.len());
                (
                    visible_text(&item[p..q]),
                    resolve(base, &attr(&a, "href").unwrap_or_default()),
                )
            })
            .unwrap_or_else(|| (visible_text(item), String::new()));
        let children = element_named(item, "ol")
            .map(|ol| parse_ol(&ol, base))
            .unwrap_or_default();
        if !label.is_empty() {
            output.push(NavItem {
                label,
                href,
                children,
            });
        }
        cursor = end;
    }
    output
}

fn nav_section<'a>(nav: &'a str, kind: &str) -> Option<&'a str> {
    for tag in tags_named(nav, "nav") {
        if attr(&tag, "epub:type")
            .unwrap_or_default()
            .split_whitespace()
            .any(|v| v.eq_ignore_ascii_case(kind))
        {
            let start = nav.find(&tag)?;
            let end = matching_tag_end(nav, start, "nav")?;
            return Some(&nav[start + tag.len()..end]);
        }
    }
    None
}

fn ruby_pairs(body: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = body[cursor..].find("<ruby") {
        let start = cursor + relative;
        let Some(end) = matching_tag_end(body, start, "ruby") else {
            break;
        };
        let inner = &body[start..end];
        let bases = tags_named(inner, "rt")
            .into_iter()
            .filter(|tag| !tag.starts_with("</"))
            .map(|tag| {
                let p = inner.find(&tag).unwrap_or(0);
                let q = matching_tag_end(inner, p, "rt").unwrap_or(inner.len());
                visible_text(&inner[p..q])
            })
            .collect::<Vec<_>>();
        let mut base_text = inner.to_owned();
        for tag in tags_named(inner, "rt")
            .into_iter()
            .filter(|tag| !tag.starts_with("</"))
        {
            if let Some(p) = base_text.find(&tag) {
                if let Some(q) = matching_tag_end(&base_text, p, "rt") {
                    base_text.replace_range(p..q, "");
                }
            }
        }
        let base_text = visible_text(&base_text);
        if !bases.is_empty() && !base_text.is_empty() {
            let chars = base_text.chars().collect::<Vec<_>>();
            for (i, rt) in bases.into_iter().enumerate() {
                if let Some(ch) = chars.get(i) {
                    pairs.push((ch.to_string(), rt));
                }
            }
        }
        cursor = end;
    }
    pairs
}

fn tags_named(source: &str, wanted: &str) -> Vec<String> {
    let wanted = wanted.rsplit(':').next().unwrap_or(wanted);
    let mut output = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('<') {
        let start = cursor + relative;
        let Some(end_relative) = source[start..].find('>') else {
            break;
        };
        let end = start + end_relative + 1;
        let tag = &source[start..end];
        if !tag.starts_with("<!--") && !tag.starts_with("<?") && !tag.starts_with("<!") {
            let name = tag_name(tag);
            if wanted == "*" || name.eq_ignore_ascii_case(wanted) {
                output.push(tag.to_owned());
            }
        }
        cursor = end;
    }
    output
}

fn tag_name(tag: &str) -> String {
    let mut s = tag.trim_start_matches('<').trim_start_matches('/').trim();
    if let Some(end) = s.find([' ', '>', '/']) {
        s = &s[..end];
    }
    s.rsplit(':').next().unwrap_or(s).to_ascii_lowercase()
}

fn attr(tag: &str, wanted: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let wanted = wanted.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(&wanted) {
        let start = cursor + relative;
        let before = start.checked_sub(1).and_then(|p| lower.as_bytes().get(p));
        let after = lower.as_bytes().get(start + wanted.len());
        if before.is_none_or(|b| b.is_ascii_whitespace()) && after == Some(&b'=') {
            let mut value = start + wanted.len() + 1;
            while lower
                .as_bytes()
                .get(value)
                .is_some_and(u8::is_ascii_whitespace)
            {
                value += 1;
            }
            let quote = lower.as_bytes().get(value).copied()?;
            if quote == b'"' || quote == b'\'' {
                let begin = value + 1;
                let end = lower[begin..].find(quote as char)? + begin;
                return Some(unescape(&tag[begin..end]));
            }
        }
        cursor = start + wanted.len();
    }
    None
}

fn visible_text(source: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    let mut skip = 0usize;
    while cursor < source.len() {
        if let Some(relative) = source[cursor..].find('<') {
            let text_end = cursor + relative;
            if skip == 0 {
                out.push_str(&unescape(&source[cursor..text_end]));
            }
            let Some(end_relative) = source[text_end..].find('>') else {
                break;
            };
            let end = text_end + end_relative + 1;
            let name = tag_name(&source[text_end..end]);
            let closing = source[text_end..end].starts_with("</");
            if matches!(name.as_str(), "head" | "title" | "style" | "script") {
                if closing {
                    skip = skip.saturating_sub(1);
                } else {
                    skip += 1;
                }
            }
            cursor = end;
        } else if skip == 0 {
            out.push_str(&unescape(&source[cursor..]));
            break;
        } else {
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn element_body(source: &str) -> Option<String> {
    element_named(source, "body")
}

fn element_named(source: &str, wanted: &str) -> Option<String> {
    let tag = tags_named(source, wanted).into_iter().next()?;
    let start = source.find(&tag)?;
    let end = matching_tag_end(source, start, wanted)?;
    Some(source[start + tag.len()..end].to_owned())
}

fn matching_tag_end(source: &str, start: usize, wanted: &str) -> Option<usize> {
    let wanted = wanted.rsplit(':').next().unwrap_or(wanted);
    let mut depth = 0usize;
    let mut cursor = start;
    while let Some(relative) = source[cursor..].find('<') {
        let p = cursor + relative;
        let e = p + source[p..].find('>')? + 1;
        let tag = &source[p..e];
        if tag_name(tag).eq_ignore_ascii_case(wanted) {
            if tag.starts_with("</") {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(e);
                }
            } else if !tag.trim_end().ends_with("/>") {
                depth += 1;
            }
        }
        cursor = e;
    }
    None
}

fn element_text(source: &str, wanted: &str) -> Option<String> {
    element_named(source, wanted).map(|s| visible_text(&s))
}

fn element_with_attr(source: &str, wanted: &str, key: &str, value: &str) -> Option<String> {
    tags_named(source, wanted)
        .into_iter()
        .find(|tag| attr(tag, key).is_some_and(|v| v == value))
        .and_then(|tag| {
            let start = source.find(&tag)?;
            let end = matching_tag_end(source, start, wanted)?;
            Some(visible_text(&source[start + tag.len()..end]))
        })
}

fn meta_property(source: &str, wanted: &str) -> Option<String> {
    let tag = tags_named(source, "meta")
        .into_iter()
        .find(|tag| attr(tag, "property").is_some_and(|v| v == wanted))?;
    if let Some(content) = attr(&tag, "content") {
        return Some(content);
    }
    let p = source.find(&tag)?;
    let end = matching_tag_end(source, p, "meta").unwrap_or(p + tag.len());
    Some(visible_text(&source[p + tag.len()..end]))
}
fn meta_name(source: &str, wanted: &str) -> Option<String> {
    tags_named(source, "meta")
        .into_iter()
        .find(|tag| attr(tag, "name").is_some_and(|v| v.eq_ignore_ascii_case(wanted)))
        .and_then(|tag| attr(&tag, "content"))
}
fn tag_attr(source: &str, wanted: &str, key: &str) -> Option<String> {
    tags_named(source, wanted)
        .into_iter()
        .next()
        .and_then(|tag| attr(&tag, key))
}

fn declarations_for(source: &str, property: &str) -> Vec<String> {
    source
        .split(';')
        .filter_map(|piece| {
            let (key, value) = piece.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(property)
                .then(|| value.split('}').next().unwrap_or(value).trim().to_owned())
        })
        .collect()
}

/// Reconstruct CSS rules from target/source bytes for semantic comparisons.
/// This intentionally handles only the declaration model needed by the audit;
/// it is independent of the production CSS parser and preserves selector and
/// property identity instead of checking raw marker presence.
pub fn css_rule_declarations(source: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut output = BTreeMap::new();
    collect_css_rules(source, &mut output);
    output
}

fn collect_css_rules(source: &str, output: &mut BTreeMap<String, BTreeMap<String, String>>) {
    let mut cursor = 0;
    while let Some(relative_open) = source[cursor..].find('{') {
        let open = cursor + relative_open;
        let prelude_start = source[..open]
            .rfind(['}', '{', ';'])
            .map(|p| p + 1)
            .unwrap_or(0);
        let prelude = source[prelude_start..open].trim();
        let Some(close) = matching_css_brace(source, open) else {
            break;
        };
        let body = &source[open + 1..close];
        if prelude.to_ascii_lowercase().starts_with("@media") || body.contains('{') {
            collect_css_rules(body, output);
        } else if !prelude.starts_with('@') && !prelude.is_empty() {
            let declarations = body
                .split(';')
                .filter_map(|declaration| {
                    let (property, value) = declaration.split_once(':')?;
                    let property = property.trim().to_ascii_lowercase();
                    let value = value.trim().to_owned();
                    (!property.is_empty() && !value.is_empty()).then_some((property, value))
                })
                .collect::<BTreeMap<_, _>>();
            output.insert(prelude.to_owned(), declarations);
        }
        cursor = close + 1;
    }
}

/// Return the semantic contents of each media-query branch as tuples of
/// `(branch, selector, property, value)`, allowing branch application to be
/// compared without treating a token's mere presence as evidence.
pub fn css_media_semantics(source: &str) -> BTreeSet<(String, String, String, String)> {
    let mut output = BTreeSet::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].to_ascii_lowercase().find("@media") {
        let start = cursor + relative;
        let Some(open_relative) = source[start..].find('{') else {
            break;
        };
        let open = start + open_relative;
        let branch = source[start + "@media".len()..open].trim().to_owned();
        let Some(close) = matching_css_brace(source, open) else {
            break;
        };
        for (selector, declarations) in css_rule_declarations(&source[open + 1..close]) {
            for (property, value) in declarations {
                output.insert((branch.clone(), selector.clone(), property, value));
            }
        }
        cursor = close + 1;
    }
    output
}

fn matching_css_brace(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes().iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn css_urls_after(source: &str, prefix: &str) -> Vec<String> {
    source
        .split(prefix)
        .skip(1)
        .filter_map(|s| {
            let p = s.find("url(")?;
            let (value, _) = s[p + 4..].split_once(')')?;
            Some(value.trim_matches(['\'', '"', ' ']).to_owned())
        })
        .collect()
}
fn all_css_urls(source: &str) -> Vec<String> {
    source
        .split("url(")
        .skip(1)
        .filter_map(|s| s.split_once(')'))
        .map(|(s, _)| s.trim_matches(['\'', '"', ' ']).to_owned())
        .collect()
}

fn parent(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(p, _)| p.to_owned())
        .unwrap_or_default()
}
fn resolve(base: &str, href: &str) -> String {
    let path = href.split(['#', '?']).next().unwrap_or(href);
    if path.is_empty() {
        return base.to_owned();
    }
    if path.contains("://") {
        return path.to_owned();
    }
    let mut stack = if path.starts_with('/') {
        Vec::new()
    } else {
        base.split('/')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            value => stack.push(value),
        }
    }
    stack.join("/")
}

fn resolve_document(document: &str, href: &str) -> String {
    let path = href.split(['#', '?']).next().unwrap_or(href);
    if href.to_ascii_lowercase().starts_with("data:") {
        href.to_owned()
    } else if path.is_empty() {
        document.to_owned()
    } else {
        resolve(&parent(document), href)
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
fn unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}
fn find_embed_numbers(raw: &str) -> Vec<usize> {
    raw.split("kindle:embed:")
        .skip(1)
        .filter_map(|s| s.split(['?', '"', '\'', '<', ' ']).next())
        .filter_map(decode_embed_number)
        .collect()
}
pub fn decode_embed_number(value: &str) -> Option<usize> {
    let mut n = 0usize;
    for c in value.bytes() {
        let d = match c {
            b'0'..=b'9' => (c - b'0') as usize,
            b'A'..=b'V' => (c - b'A' + 10) as usize,
            _ => return None,
        };
        n = n.checked_mul(32)?.checked_add(d)?;
    }
    Some(n)
}

pub fn decode_position_href(value: &str) -> Option<(u32, u32)> {
    let value = value.strip_prefix("kindle:pos:fid:")?;
    let (sequence, offset) = value.split_once(":off:")?;
    Some((
        u32::try_from(decode_base32(sequence)?).ok()?,
        u32::try_from(decode_base32(offset)?).ok()?,
    ))
}

fn decode_base32(value: &str) -> Option<usize> {
    decode_embed_number(value)
}

fn be32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or("u32 outside target record")?;
    Ok(u32::from_be_bytes(slice.try_into().unwrap()))
}

fn is_sfnt(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some(b"OTTO") | Some(b"true") | Some(b"ttcf") | Some([0, 1, 0, 0])
    )
}

fn decode_vwi(bytes: &[u8], start: usize) -> Result<(u32, usize), String> {
    let mut value = 0u32;
    for (used, byte) in bytes
        .get(start..)
        .ok_or("VWI starts outside record")?
        .iter()
        .copied()
        .enumerate()
    {
        value = value
            .checked_shl(7)
            .ok_or("VWI overflow")?
            .checked_add((byte & 0x7f) as u32)
            .ok_or("VWI overflow")?;
        if byte & 0x80 != 0 {
            return Ok((value, used + 1));
        }
        if used >= 4 {
            return Err("VWI exceeds five bytes".into());
        }
    }
    Err("unterminated VWI".into())
}

fn decode_index_row(
    detail: &[u8],
    start: usize,
    defs: &[(u8, u8, u8)],
) -> Result<(String, BTreeMap<u8, Vec<u32>>), String> {
    let len = *detail
        .get(start)
        .ok_or("INDX row text length outside detail")? as usize;
    let text_start = start + 1;
    let control_pos = text_start.checked_add(len).ok_or("INDX row overflow")?;
    let control = *detail
        .get(control_pos)
        .ok_or("INDX row control outside detail")?;
    let text_end = control_pos;
    let mut cursor = control_pos + 1;
    let mut values: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
    for (tag, per_entry, mask) in defs {
        let shift = mask.trailing_zeros();
        let occurrences = if mask.count_ones() == 1 {
            u32::from((control & mask) != 0)
        } else {
            u32::from((control & mask) >> shift)
        };
        for _ in 0..occurrences {
            for _ in 0..*per_entry {
                let (value, used) = decode_vwi(detail, cursor)?;
                cursor += used;
                values.entry(*tag).or_default().push(value);
            }
        }
    }
    Ok((
        String::from_utf8_lossy(&detail[text_start..text_end]).into_owned(),
        values,
    ))
}

/// Independent IDPF key derivation and first-1040-byte deobfuscation.
pub fn idpf_deobfuscate(identifier: &str, bytes: &[u8]) -> Vec<u8> {
    let key = sha1(identifier.as_bytes());
    let mut out = bytes.to_vec();
    for (i, byte) in out.iter_mut().take(1040).enumerate() {
        *byte ^= key[i % key.len()];
    }
    out
}

fn sha1(input: &[u8]) -> [u8; 20] {
    let mut data = input.to_vec();
    let bit_len = (data.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    let mut h = [
        0x67452301u32,
        0xefcdab89,
        0x98badcfe,
        0x10325476,
        0xc3d2e1f0,
    ];
    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
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
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.into_iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
