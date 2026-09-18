#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutSemantic {
    #[default]
    Reflowable,
    PrePaginated,
}

#[derive(Debug, Clone, Default)]
pub struct ContentDocument {
    pub id: String,
    // These source-level fields are retained across parsing and normalization;
    // KF8 section assembly consumes `source_xhtml` and the resolved style IDs.
    #[allow(dead_code)]
    pub href: String,
    #[allow(dead_code)]
    pub media_type: String,
    pub source_xhtml: String,
    /// Effective source rendition layout resolved from the OPF spine itemref
    /// and publication-level rendition metadata.
    pub layout: LayoutSemantic,
    /// Numeric dimensions from this fixed-layout XHTML document's explicit
    /// viewport declaration. Intrinsic image and SVG dimensions are not used.
    pub page_viewport: Option<String>,
    pub rendition: crate::book::RenditionSemantics,
    /// Original EPUB spine itemref properties, including page-spread aliases.
    pub source_properties: Vec<String>,
    /// Zero-based source spine position used to relate this document to the
    /// emitted Kindle section/SKEL topology.
    pub source_spine_index: usize,
    /// Whether the source document was classified as a cover during EPUB
    /// parsing. The detailed semantic parse is intentionally short-lived in
    /// the conversion pipeline; retaining it here duplicates large text and
    /// annotation buffers that KF8 normalization does not consume.
    pub is_cover: bool,
    pub referenced_styles: Vec<String>,
    /// Canonical hrefs of linked stylesheets intentionally omitted because
    /// their bytes could not be decoded by the supported text decoder.
    pub dropped_stylesheets: Vec<String>,
}

impl ContentDocument {
    pub(crate) fn set_layout_pre_paginated(&mut self, pre_paginated: bool) {
        self.layout = if pre_paginated {
            LayoutSemantic::PrePaginated
        } else {
            LayoutSemantic::Reflowable
        };
    }

    pub(crate) fn is_pre_paginated(&self) -> bool {
        self.layout == LayoutSemantic::PrePaginated
    }
}

#[derive(Debug, Clone, Default)]
pub struct SemanticDocument {
    /// Image/SVG resource references retained for document-level semantic
    /// classification, such as legacy EPUB cover documents.
    pub image_references: Vec<String>,
    /// EPUB semantic body typing identifies this document as a cover without
    /// conflating it with the binary cover-image resource.
    pub is_cover: bool,
}

pub(crate) fn plain_display_text(value: &str) -> String {
    let value = decode_display_entities(value);
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < value.len() {
        if value.as_bytes()[cursor] != b'<' {
            let character = value[cursor..]
                .chars()
                .next()
                .expect("cursor remains on a UTF-8 boundary");
            output.push(character);
            cursor += character.len_utf8();
            continue;
        }
        let mut tag_cursor = cursor + 1;
        let mut quote = None;
        while tag_cursor < value.len() {
            let byte = value.as_bytes()[tag_cursor];
            match (quote, byte) {
                (Some(expected), byte) if byte == expected => quote = None,
                (Some(_), _) => {}
                (None, b'"' | b'\'') => quote = Some(byte),
                (None, b'>') => break,
                _ => {}
            }
            tag_cursor += 1;
        }
        if tag_cursor == value.len() {
            output.push_str(&value[cursor..]);
            break;
        }
        cursor = tag_cursor + 1;
    }
    output.trim().to_owned()
}

fn decode_display_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
