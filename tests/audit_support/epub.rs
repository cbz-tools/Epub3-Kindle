//! Deterministic EPUB input recipes derived from external requirements.
//! This module intentionally does not import production EPUB parsers or helpers.

use std::io::{Cursor, Write};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ColorType, DynamicImage, ImageEncoder, RgbImage, RgbaImage};
use zip::ZipWriter;
use zip::unstable::write::FileOptionsExt;
use zip::write::SimpleFileOptions;

const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub bytes: Vec<u8>,
    pub stored: bool,
}

impl Entry {
    pub fn text(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bytes: text.into().into_bytes(),
            stored: false,
        }
    }

    pub fn binary(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            bytes,
            stored: false,
        }
    }
}

pub fn write_epub(extra: Vec<Entry>) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        let deflated =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(CONTAINER_XML.as_bytes()).unwrap();

        for entry in extra {
            let options = if entry.stored { stored } else { deflated };
            zip.start_file(entry.name, options).unwrap();
            zip.write_all(&entry.bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn nav(entries: &[(&str, &str)]) -> String {
    let mut items = String::new();
    for (href, label) in entries {
        items.push_str(&format!("<li><a href=\"{href}\">{label}</a></li>"));
    }
    let landmark_href = entries.first().map(|(href, _)| *href).unwrap_or("");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Navigation</title></head><body>
<nav epub:type="toc" id="toc"><h1>Contents</h1><ol>{items}</ol></nav>
<nav epub:type="landmarks"><ol><li><a epub:type="bodymatter" href="{landmark_href}">Start</a></li></ol></nav>
</body></html>"#
    )
}

fn package(metadata: &str, manifest: &str, spine: &str, extra_metadata: &str) -> String {
    package_with_language("en", metadata, manifest, spine, extra_metadata)
}

fn package_with_language(
    language: &str,
    metadata: &str,
    manifest: &str,
    spine: &str,
    extra_metadata: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id" xml:lang="{language}" prefix="schema: http://schema.org/ rendition: http://www.idpf.org/vocab/rendition/#">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="pub-id">urn:uuid:authority-audit-fixed-id</dc:identifier>
<dc:title>Authority Audit</dc:title>
<dc:language>{language}</dc:language>
<dc:creator>Audit Author</dc:creator>
<meta property="dcterms:modified">2026-09-13T00:00:00Z</meta>
{metadata}
{extra_metadata}
</metadata>
<manifest>
{manifest}
</manifest>
<spine page-progression-direction="ltr">
{spine}
</spine>
</package>"#
    )
}

fn xhtml(title: &str, lang: &str, body_attrs: &str, body: &str, head_extra: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{lang}" lang="{lang}">
<head><title>{title}</title>{head_extra}</head>
<body {body_attrs}>{body}</body></html>"#
    )
}

pub fn minimal_reflowable() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Chapter One")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Chapter One",
                "en",
                "",
                "<h1 id=\"ch1\">Chapter One</h1><p>AUTH_MINIMAL_ALPHA 0123456789.</p>",
                "",
            ),
        ),
    ])
}

pub fn metadata_and_spine() -> Vec<u8> {
    let opf = package(
        "<dc:publisher>Authority Publisher</dc:publisher><dc:subject>Audit Subject</dc:subject>",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="text/one.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/two.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c2"/><itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[
                ("text/two.xhtml", "Second First"),
                ("text/one.xhtml", "First Second"),
            ]),
        ),
        Entry::text(
            "EPUB/text/one.xhtml",
            xhtml("One", "en", "", "<h1>One</h1><p>AUTH_SPINE_ONE</p>", ""),
        ),
        Entry::text(
            "EPUB/text/two.xhtml",
            xhtml("Two", "en", "", "<h1>Two</h1><p>AUTH_SPINE_TWO</p>", ""),
        ),
    ])
}

pub fn metadata_date_divergence() -> Vec<u8> {
    let opf = package(
        "<dc:date>2022-10-17</dc:date>",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "Divergent dates")]),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Divergent dates",
                "en",
                "",
                "<p>AUTH_DATE_DIVERGENCE</p>",
                "",
            ),
        ),
    ])
}

pub fn css_data_uri_image() -> Vec<u8> {
    let data_uri = format!("data:image/png;base64,{}", base64(&tiny_png()));
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/data.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "CSS data image")]),
        ),
        Entry::text(
            "EPUB/styles/data.css",
            format!(".data-image{{background-image:url({data_uri})}}"),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "CSS data image",
                "en",
                "",
                "<p class=\"data-image\">AUTH_CSS_DATA_URI</p>",
                "<link rel=\"stylesheet\" href=\"../styles/data.css\"/>",
            ),
        ),
    ])
}

pub fn xhtml_data_uri_image() -> Vec<u8> {
    let data_uri = format!("data:image/png;base64,{}", base64(&tiny_png()));
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "XHTML data image")]),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "XHTML data image",
                "en",
                "",
                &format!(
                    "<p>AUTH_XHTML_DATA_URI</p><img src=\"{data_uri}\" alt=\"Inline image\"/>",
                ),
                "",
            ),
        ),
    ])
}

pub fn inline_style_data_uri_images_at_root_and_depth() -> Vec<u8> {
    let data_uri = format!("data:image/png;base64,{}", base64(&tiny_png()));
    let root_style = format!("<style>.inline-root{{background-image:url({data_uri})}}</style>");
    let deep_style = format!("<style>.inline-deep{{background-image:url({data_uri})}}</style>");
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="root" href="root.xhtml" media-type="application/xhtml+xml"/><item id="deep" href="chapters/very/deep/section.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="root"/><itemref idref="deep"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[
                ("root.xhtml", "Root inline style"),
                ("chapters/very/deep/section.xhtml", "Deep inline style"),
            ]),
        ),
        Entry::text(
            "EPUB/root.xhtml",
            xhtml(
                "Root inline style",
                "en",
                "",
                "<p class=\"inline-root\">AUTH_INLINE_CSS_ROOT</p>",
                &root_style,
            ),
        ),
        Entry::text(
            "EPUB/chapters/very/deep/section.xhtml",
            xhtml(
                "Deep inline style",
                "en",
                "",
                "<p class=\"inline-deep\">AUTH_INLINE_CSS_DEEP</p>",
                &deep_style,
            ),
        ),
    ])
}

pub fn unsafe_data_uri_images() -> Vec<u8> {
    let png = base64(&tiny_png());
    let corrupt = base64(b"not an image");
    let oversized = base64(&oversized_png());
    let css = format!(
        ".css-unsafe-unsupported{{background-image:url(data:image/gif;base64,{png})}}\n\
.css-unsafe-mismatched{{background-image:url(data:image/jpeg;base64,{png})}}\n\
.css-unsafe-malformed{{background-image:url(data:image/png;base64,not-base64!!)}}\n\
.css-unsafe-corrupt{{background-image:url(data:image/png;base64,{corrupt})}}\n\
.css-unsafe-oversized{{background-image:url(data:image/png;base64,{oversized})}}"
    );
    let body = format!(
        "<p>AUTH_UNSAFE_DATA_URI_BEGIN</p>\
<p class=\"css-unsafe-unsupported\">CSS_UNSAFE_UNSUPPORTED</p>\
<p class=\"css-unsafe-mismatched\">CSS_UNSAFE_MISMATCHED</p>\
<p class=\"css-unsafe-malformed\">CSS_UNSAFE_MALFORMED</p>\
<p class=\"css-unsafe-corrupt\">CSS_UNSAFE_CORRUPT</p>\
<p class=\"css-unsafe-oversized\">CSS_UNSAFE_OVERSIZED</p>\
<p>XHTML_UNSAFE_UNSUPPORTED <img class=\"xhtml-unsafe-unsupported\" src=\"data:image/gif;base64,{png}\" alt=\"Unsupported media\"/></p>\
<p>XHTML_UNSAFE_MISMATCHED <img class=\"xhtml-unsafe-mismatched\" src=\"data:image/jpeg;base64,{png}\" alt=\"Mismatched media\"/></p>\
<p>XHTML_UNSAFE_MALFORMED <img class=\"xhtml-unsafe-malformed\" src=\"data:image/png;base64,not-base64!!\" alt=\"Malformed base64\"/></p>\
<p>XHTML_UNSAFE_CORRUPT <img class=\"xhtml-unsafe-corrupt\" src=\"data:image/png;base64,{corrupt}\" alt=\"Corrupt image\"/></p>\
<p>XHTML_UNSAFE_OVERSIZED <img class=\"xhtml-unsafe-oversized\" src=\"data:image/png;base64,{oversized}\" alt=\"Oversized image\"/></p>"
    );
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/unsafe.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "Unsafe data images")]),
        ),
        Entry::text("EPUB/styles/unsafe.css", css),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Unsafe data images",
                "en",
                "",
                &body,
                "<link rel=\"stylesheet\" href=\"../styles/unsafe.css\"/>",
            ),
        ),
    ])
}

pub fn css_namespace_selector() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/namespace.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Namespace")])),
        Entry::text(
            "EPUB/styles/namespace.css",
            "@namespace epub \"http://www.idpf.org/2007/ops\"; *[epub|type~=\"note\"] { color: red; }",
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Namespace",
                "en",
                "",
                "<aside epub:type=\"note\">AUTH_CSS_NAMESPACE</aside>",
                "<link rel=\"stylesheet\" href=\"../styles/namespace.css\"/>",
            ),
        ),
    ])
}

pub fn nested_navigation() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/><itemref idref="c2"/>"#,
        "",
    );
    let nav_doc = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Nav</title></head><body>
<nav epub:type="toc"><ol>
<li><a href="text/ch1.xhtml#top">Part A</a><ol><li><a href="text/ch1.xhtml#sub">Nested A.1</a></li></ol></li>
<li><a href="text/ch2.xhtml#top">Part B</a></li>
</ol></nav>
<nav epub:type="landmarks"><ol><li><a epub:type="bodymatter" href="text/ch1.xhtml#top">Beginning</a></li></ol></nav>
</body></html>"#;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav_doc),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "A",
                "en",
                "",
                "<h1 id=\"top\">Part A</h1><h2 id=\"sub\">Nested A.1</h2><p><a href=\"ch2.xhtml#top\">AUTH_LINK_TO_B</a></p>",
                "",
            ),
        ),
        Entry::text(
            "EPUB/text/ch2.xhtml",
            xhtml(
                "B",
                "en",
                "",
                "<h1 id=\"top\">Part B</h1><p><a href=\"ch1.xhtml#sub\">AUTH_LINK_BACK</a></p>",
                "",
            ),
        ),
    ])
}

pub fn vertical_japanese() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="css" href="styles/book.css" media-type="text/css"/>
<item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let css = r#"html { writing-mode: vertical-rl; -webkit-writing-mode: vertical-rl; }
.tcy { text-combine-upright: all; }
.em { -webkit-text-emphasis-style: sesame; }
.upright { -webkit-text-orientation: upright; }"#;
    let body = "<h1>縦書き</h1><p>AUTH_JA_開始。<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>、<span class=\"tcy\">12</span>、<span class=\"em\">圏点</span>、<span class=\"upright\">A</span>。終端。</p>";
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "縦書き")])),
        Entry::text("EPUB/styles/book.css", css),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "縦書き",
                "ja",
                "",
                body,
                "<link rel=\"stylesheet\" href=\"../styles/book.css\"/>",
            ),
        ),
    ])
}

pub fn unicode_stress() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let body =
        "<p>AUTH_UNICODE: 葛󠄀 辻󠄀 が き゚ 😀 𠮷野家 Café Ελληνικά 한국어 简体中文 —―…「」『』。</p>";
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Unicode")])),
        Entry::text("EPUB/text/ch1.xhtml", xhtml("Unicode", "ja", "", body, "")),
    ])
}

pub fn resource_graph() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="css" href="styles/main.css" media-type="text/css"/>
<item id="css2" href="styles/sub/more.css" media-type="text/css"/>
<item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/sub/ch2.xhtml" media-type="application/xhtml+xml"/>
<item id="img" href="images/shared.png" media-type="image/png"/>"#,
        r#"<itemref idref="c1"/><itemref idref="c2"/>"#,
        "",
    );
    let png = tiny_png();
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "One"), ("text/sub/ch2.xhtml", "Two")]),
        ),
        Entry::text(
            "EPUB/styles/main.css",
            "@import url('sub/more.css'); .shared{background-image:url('../images/shared.png');}",
        ),
        Entry::text("EPUB/styles/sub/more.css", ".more{font-weight:bold;}"),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "One",
                "en",
                "",
                "<p class=\"shared\">AUTH_RESOURCE_ONE</p><img src=\"../images/shared.png\" alt=\"Shared marker\"/><a href=\"sub/ch2.xhtml#two\">to two</a>",
                "<link rel=\"stylesheet\" href=\"../styles/main.css\"/>",
            ),
        ),
        Entry::text(
            "EPUB/text/sub/ch2.xhtml",
            xhtml(
                "Two",
                "en",
                "",
                "<p id=\"two\" class=\"more\">AUTH_RESOURCE_TWO</p><img src=\"../../images/shared.png\" alt=\"Shared marker 2\"/>",
                "<link rel=\"stylesheet\" href=\"../../styles/main.css\"/>",
            ),
        ),
        Entry::binary("EPUB/images/shared.png", png),
    ])
}

/// Deterministic image corpus for the Kindle LD image-size contract.
///
/// The pixel stream is deliberately high-entropy so the encoded JPEG and PNG
/// both remain above the 128 KiB source boundary while the small controls stay
/// well below it.
pub fn large_image_resources(comic: bool) -> Vec<u8> {
    let book_type = if comic {
        r#"<meta name="book-type" content="  CoMiC  "/>"#
    } else {
        ""
    };
    let metadata = format!(r#"<meta name="cover" content="large-jpeg"/>{book_type}"#);
    let opf = package(
        &metadata,
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
<item id="large-jpeg" href="images/large.jpg" media-type="image/jpeg"/>
<item id="large-png" href="images/large.png" media-type="image/png"/>
<item id="undershoot-jpeg" href="images/undershoot.jpg" media-type="image/jpeg"/>
<item id="boundary-png" href="images/boundary.png" media-type="image/png"/>
<item id="medium-png" href="images/medium.png" media-type="image/png"/>
<item id="small-jpeg" href="images/small.jpg" media-type="image/jpeg"/>
<item id="small-png" href="images/small.png" media-type="image/png"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "Image resources")]),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Image resources",
                "en",
                "",
                "<img src=\"../images/large.jpg\"/><img src=\"../images/large.png\"/><img src=\"../images/undershoot.jpg\"/><img src=\"../images/boundary.png\"/><img src=\"../images/medium.png\"/><img src=\"../images/small.jpg\"/><img src=\"../images/small.png\"/><p>AUTH_LD_IMAGE_RESOURCES</p>",
                "",
            ),
        ),
        Entry::binary("EPUB/images/large.jpg", audit_large_jpeg()),
        Entry::binary("EPUB/images/large.png", audit_large_png()),
        Entry::binary("EPUB/images/undershoot.jpg", audit_undershoot_jpeg()),
        Entry::binary("EPUB/images/boundary.png", audit_boundary_png()),
        Entry::binary("EPUB/images/medium.png", audit_medium_png()),
        Entry::binary("EPUB/images/small.jpg", audit_small_jpeg()),
        Entry::binary("EPUB/images/small.png", tiny_png()),
    ])
}

pub fn audit_large_jpeg() -> Vec<u8> {
    let image = audit_rgb_image(640, 480);
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 95)
        .encode_image(&DynamicImage::ImageRgb8(image))
        .unwrap();
    assert!(encoded.len() > 131_072);
    encoded
}

pub fn audit_large_png() -> Vec<u8> {
    let image = audit_rgba_image(480, 640);
    let mut encoded = Vec::new();
    PngEncoder::new(&mut encoded)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ColorType::Rgba8.into(),
        )
        .unwrap();
    assert!(encoded.len() > 131_072);
    encoded
}

pub fn audit_undershoot_jpeg() -> Vec<u8> {
    let image = RgbImage::from_fn(1024, 768, |x, y| {
        if (x + y) % 2 == 0 {
            image::Rgb([0, 0, 0])
        } else {
            image::Rgb([255, 255, 255])
        }
    });
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 95)
        .encode_image(&DynamicImage::ImageRgb8(image))
        .unwrap();
    assert!(encoded.len() > 131_072);
    encoded
}

pub fn audit_boundary_png() -> Vec<u8> {
    audit_padded_png(131_072)
}

pub fn audit_medium_png() -> Vec<u8> {
    audit_padded_png(120_000)
}

fn audit_padded_png(target_length: usize) -> Vec<u8> {
    let mut png = tiny_png();
    let iend_start = png
        .len()
        .checked_sub(12)
        .expect("fixed PNG must contain an IEND chunk");
    let data_length = target_length
        .checked_sub(png.len() + 12)
        .expect("fixed PNG must leave room for an ancillary chunk");
    let mut ancillary = Vec::with_capacity(data_length + 12);
    ancillary.extend_from_slice(&(data_length as u32).to_be_bytes());
    ancillary.extend_from_slice(b"aaAa");
    ancillary.resize(8 + data_length, 0);
    ancillary.extend_from_slice(&png_crc32(&ancillary[4..]).to_be_bytes());
    png.splice(iend_start..iend_start, ancillary);
    assert_eq!(png.len(), target_length);
    assert!(image::load_from_memory(&png).is_ok());
    png
}

pub fn audit_small_jpeg() -> Vec<u8> {
    let image = RgbImage::from_fn(32, 24, |x, y| {
        image::Rgb([(x * 7) as u8, (y * 11) as u8, ((x + y) * 13) as u8])
    });
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 90)
        .encode_image(&DynamicImage::ImageRgb8(image))
        .unwrap();
    assert!(encoded.len() <= 131_072);
    encoded
}

fn audit_rgb_image(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        let mut state = 0x9e37_79b9_u32 ^ x.wrapping_mul(0x85eb_ca6b) ^ y;
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        image::Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
    })
}

fn audit_rgba_image(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let mut state = 0x243f_6a88_u32 ^ x.wrapping_mul(0x27d4_eb2d) ^ y;
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        image::Rgba([state as u8, (state >> 8) as u8, (state >> 16) as u8, 0])
    })
}

pub fn fixed_layout() -> Vec<u8> {
    fixed_layout_with_original_resolution(true)
}

/// The fixed-layout authority input deliberately omits Amazon's legacy
/// `original-resolution` metadata.  Each fixed XHTML page still carries the
/// same numeric viewport, so the converter must derive one publication-level
/// resolution from all fixed pages rather than from a first-page shortcut.
pub fn fixed_layout_inferred_resolution() -> Vec<u8> {
    fixed_layout_with_original_resolution(false)
}

fn fixed_layout_with_original_resolution(explicit_original_resolution: bool) -> Vec<u8> {
    let original_resolution = if explicit_original_resolution {
        r#"<meta name="original-resolution" content="1200x800"/>"#
    } else {
        ""
    };
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="p1" href="text/p1.xhtml" media-type="application/xhtml+xml"/>
<item id="p2" href="text/p2.xhtml" media-type="application/xhtml+xml"/>
<item id="page" href="images/page.png" media-type="image/png"/>"#,
        r#"<itemref idref="p1" properties="page-spread-left"/><itemref idref="p2" properties="page-spread-right"/>"#,
        &format!(
            r#"<meta property="rendition:layout">pre-paginated</meta><meta property="rendition:orientation">landscape</meta><meta property="rendition:spread">both</meta><meta property="rendition:viewport">width=1200,height=800</meta>{original_resolution}"#
        ),
    );
    let head = "<meta name=\"viewport\" content=\"width=1200,height=800\"/><style>html,body{margin:0;width:1200px;height:800px}</style>";
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/p1.xhtml", "Page 1"), ("text/p2.xhtml", "Page 2")]),
        ),
        Entry::text(
            "EPUB/text/p1.xhtml",
            xhtml(
                "P1",
                "en",
                "",
                "<img src=\"../images/page.png\"/><p>AUTH_FIXED_PAGE_1</p>",
                head,
            ),
        ),
        Entry::text(
            "EPUB/text/p2.xhtml",
            xhtml(
                "P2",
                "en",
                "",
                "<img src=\"../images/page.png\"/><p>AUTH_FIXED_PAGE_2</p>",
                head,
            ),
        ),
        Entry::binary("EPUB/images/page.png", tiny_png()),
    ])
}

pub fn fixed_layout_with_degraded_viewport() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="p1" href="text/p1.xhtml" media-type="application/xhtml+xml"/>
<item id="page" href="images/page.png" media-type="image/png"/>"#,
        r#"<itemref idref="p1"/>"#,
        r#"<meta property="rendition:layout">pre-paginated</meta><meta property="rendition:viewport">width=1200,height=800</meta>"#,
    );
    let head = r#"<meta name="viewport" content="width=1200,height=800,initial-scale=1"/><style>html,body{margin:0;width:1200px;height:800px}</style>"#;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/p1.xhtml", "Page 1")])),
        Entry::text(
            "EPUB/text/p1.xhtml",
            xhtml(
                "P1",
                "en",
                "",
                "<img src=\"../images/page.png\"/><p>AUTH_DEGRADED_VIEWPORT</p>",
                head,
            ),
        ),
        Entry::binary("EPUB/images/page.png", tiny_png()),
    ])
}

pub fn mixed_layout() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="r" href="text/r.xhtml" media-type="application/xhtml+xml"/><item id="f" href="text/f.xhtml" media-type="application/xhtml+xml"/><item id="page" href="images/page.png" media-type="image/png"/>"#,
        r#"<itemref idref="r"/><itemref idref="f" properties="rendition:layout-pre-paginated"/>"#,
        r#"<meta property="rendition:layout">reflowable</meta>"#,
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/r.xhtml", "Reflow"), ("text/f.xhtml", "Fixed")]),
        ),
        Entry::text(
            "EPUB/text/r.xhtml",
            xhtml("R", "en", "", "<p>AUTH_MIX_REFLOW</p>", ""),
        ),
        Entry::text(
            "EPUB/text/f.xhtml",
            xhtml(
                "F",
                "en",
                "",
                "<img src=\"../images/page.png\"/><p>AUTH_MIX_FIXED</p>",
                "<meta name=\"viewport\" content=\"width=600,height=800\"/>",
            ),
        ),
        Entry::binary("EPUB/images/page.png", tiny_png()),
    ])
}

pub fn accessibility_structure() -> Vec<u8> {
    let opf = package(
        "<meta property=\"schema:accessMode\">textual</meta>",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/a11y.xhtml" media-type="application/xhtml+xml"/><item id="img" href="images/a.png" media-type="image/png"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let body = r##"<main><h1>AUTH_A11Y_HEADING</h1><ul><li>Item A</li><li>Item B</li></ul><figure><img src="../images/a.png" alt="Meaningful alternative"/><figcaption>AUTH_A11Y_CAPTION</figcaption></figure><table><thead><tr><th scope="col">Header</th></tr></thead><tbody><tr><td>Cell</td></tr></tbody></table><p><a href="#target">Descriptive internal target</a></p><h2 id="target">Target</h2></main>"##;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/a11y.xhtml", "Accessible chapter")]),
        ),
        Entry::text("EPUB/text/a11y.xhtml", xhtml("A11y", "en", "", body, "")),
        Entry::binary("EPUB/images/a.png", tiny_png()),
    ])
}

pub fn svg_reference_graph() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="image" href="images/shared.png" media-type="image/png"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let body = r##"<p>AUTH_SVG_GRAPH</p><img src="../images/shared.png" alt="Raster edge"/><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image href="../images/shared.png" xlink:href="../images/shared.png" width="1" height="1"/></svg>"##;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "SVG graph")])),
        Entry::text("EPUB/text/ch1.xhtml", xhtml("SVG", "en", "", body, "")),
        Entry::binary("EPUB/images/shared.png", tiny_png()),
    ])
}

pub fn cover_png() -> Vec<u8> {
    let opf = package(
        "<meta name=\"cover\" content=\"cover-image\"/>",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="cover-page" href="text/cover.xhtml" media-type="application/xhtml+xml"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="cover-image" href="images/cover.png" media-type="image/png" properties="cover-image"/>"#,
        r#"<itemref idref="cover-page"/><itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Body")])),
        Entry::text(
            "EPUB/text/cover.xhtml",
            xhtml(
                "Cover",
                "en",
                "",
                "<img src=\"../images/cover.png\" alt=\"Authority Audit Cover\"/>",
                "",
            ),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Body", "en", "", "<p>AUTH_COVER_BODY</p>", ""),
        ),
        Entry::binary("EPUB/images/cover.png", tiny_png()),
    ])
}

pub fn cover_jpeg() -> Vec<u8> {
    let opf = package(
        "<meta name=\"cover\" content=\"cover-image\"/>",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="cover-page" href="text/cover.xhtml" media-type="application/xhtml+xml"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="cover-image" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>"#,
        r#"<itemref idref="cover-page"/><itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Body")])),
        Entry::text(
            "EPUB/text/cover.xhtml",
            xhtml(
                "Cover",
                "en",
                "",
                "<img src=\"../images/cover.jpg\" alt=\"Authority Audit JPEG Cover\"/>",
                "",
            ),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Body", "en", "", "<p>AUTH_COVER_JPEG_BODY</p>", ""),
        ),
        Entry::binary("EPUB/images/cover.jpg", audit_small_jpeg()),
    ])
}

pub fn embedded_font(obfuscated: bool) -> Vec<u8> {
    let font = include_bytes!("../fixtures/embedded-font/fonts/EBGaramond12-Bold.ttf");
    let identifier = "urn:uuid:authority-audit-fixed-id";
    let font_bytes = if obfuscated {
        idpf_obfuscate(font, identifier)
    } else {
        font.to_vec()
    };
    let encryption = if obfuscated {
        vec![Entry::text(
            "META-INF/encryption.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
<enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/><enc:CipherData><enc:CipherReference URI="EPUB/fonts/audit.ttf"/></enc:CipherData></enc:EncryptedData>
</encryption>"#,
        )]
    } else {
        vec![]
    };
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/font.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="font" href="fonts/audit.ttf" media-type="font/ttf"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let mut entries = vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Font")])),
        Entry::text(
            "EPUB/styles/font.css",
            "@font-face{font-family:'AuthorityAudit';src:url('../fonts/audit.ttf');} .fonted{font-family:'AuthorityAudit';}",
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Font",
                "en",
                "",
                "<p class=\"fonted\">AUTH_FONT_TEXT</p><p>AUTH_FONT_UNRELATED</p>",
                "<link rel=\"stylesheet\" href=\"../styles/font.css\"/>",
            ),
        ),
        Entry::binary("EPUB/fonts/audit.ttf", font_bytes),
    ];
    entries.extend(encryption);
    write_epub(entries)
}

pub fn scripting() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml" properties="scripted"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Scripted")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Script",
                "en",
                "onload=\"alert('x')\"",
                "<p>AUTH_SCRIPT</p><script>document.body.dataset.x='1'</script>",
                "",
            ),
        ),
    ])
}

pub fn unsupported_css() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/bad.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "CSS")])),
        Entry::text(
            "EPUB/styles/bad.css",
            "h1 + p { color:red } p::before { content:'invented'; } ol{counter-reset:item} li::before{counter-increment:item;content:counter(item)}",
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "CSS",
                "en",
                "",
                "<h1>Heading</h1><p>AUTH_BAD_CSS</p>",
                "<link rel=\"stylesheet\" href=\"../styles/bad.css\"/>",
            ),
        ),
    ])
}

fn unsupported_css_case(css: &str, marker: &str) -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/bad.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "CSS")])),
        Entry::text("EPUB/styles/bad.css", css),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "CSS",
                "en",
                "",
                &format!("<h1>Heading</h1><p>{marker}</p>"),
                "<link rel=\"stylesheet\" href=\"../styles/bad.css\"/>",
            ),
        ),
    ])
}

pub fn unsupported_css_selector() -> Vec<u8> {
    unsupported_css_case("h1 + p { color:red }", "AUTH_BAD_SELECTOR")
}

pub fn unsupported_css_pseudo() -> Vec<u8> {
    unsupported_css_case("p::before { content:'invented'; }", "AUTH_BAD_PSEUDO")
}

pub fn unsupported_css_counter() -> Vec<u8> {
    unsupported_css_case("ol { counter-reset:item; }", "AUTH_BAD_COUNTER")
}

pub fn paired_css_semantics() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="base" href="styles/base.css" media-type="text/css"/><item id="main" href="styles/main.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let base = ".base{color:rgb(1,2,3)}";
    let main = r#"@import url('base.css'); html{writing-mode:vertical-rl;-webkit-writing-mode:vertical-rl} .horizontal{writing-mode:horizontal-tb;-webkit-writing-mode:horizontal-tb} .tcy{text-combine-upright:all;-webkit-text-combine:horizontal} .em{text-emphasis-style:sesame;-webkit-text-emphasis-style:sesame} .upright{text-orientation:upright;-webkit-text-orientation:upright} ruby{ruby-position:over}"#;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "CSS pair")])),
        Entry::text("EPUB/styles/base.css", base),
        Entry::text("EPUB/styles/main.css", main),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "CSS pair",
                "en",
                "",
                "<h1>縦書き</h1><p class=\"horizontal\">横書き</p><p class=\"tcy\">12</p><p class=\"em\">圏点</p><p class=\"upright\">A</p><ruby>漢<rt>かん</rt></ruby>",
                "<link rel=\"stylesheet\" href=\"../styles/main.css\"/>",
            ),
        ),
    ])
}

pub fn css_cascade_stress() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="base" href="styles/base.css" media-type="text/css"/><item id="main" href="styles/main.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let base = "p { color: red; } .base { color: blue; }";
    let main = "@import url('base.css'); p { color: green; } .base { color: orange; } #target { color: purple; } @media amzn-kf8 { #target { font-weight: bold; } } @media amzn-mobi { #target { font-weight: normal; } }";
    let body = r#"<p id="target" class="base" style="color: black">AUTH_CASCADE_TARGET</p><p class="base">AUTH_CASCADE_CLASS</p>"#;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Cascade")])),
        Entry::text("EPUB/styles/base.css", base),
        Entry::text("EPUB/styles/main.css", main),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Cascade",
                "en",
                "",
                body,
                "<link rel=\"stylesheet\" href=\"../styles/main.css\"/>",
            ),
        ),
    ])
}

pub fn remote_resource() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Remote")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Remote",
                "en",
                "",
                "<p>AUTH_REMOTE</p><img src=\"https://example.invalid/never-fetch.png\" alt=\"remote\"/>",
                "",
            ),
        ),
    ])
}

pub fn non_ascii_ocf_path() -> Vec<u8> {
    let opf_path = "EPUB/資料/パッケージ.opf";
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="本文/章.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let mut mimetype = Entry::binary("mimetype", b"application/epub+zip".to_vec());
    mimetype.stored = true;
    write_zip_entries(vec![
        mimetype,
        Entry::text(
            "META-INF/container.xml",
            CONTAINER_XML.replace("EPUB/package.opf", opf_path),
        ),
        Entry::text(opf_path, opf),
        Entry::text("EPUB/資料/nav.xhtml", nav(&[("本文/章.xhtml", "非ASCII")])),
        Entry::text(
            "EPUB/資料/本文/章.xhtml",
            xhtml("非ASCII", "ja", "", "<p>AUTH_NON_ASCII_OCF_PATH</p>", ""),
        ),
    ])
}

pub fn internal_two_dot_paths() -> Vec<u8> {
    let opf_path = "EPUB/nested/package.opf";
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="../text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let mut mimetype = Entry::binary("mimetype", b"application/epub+zip".to_vec());
    mimetype.stored = true;
    write_zip_entries(vec![
        mimetype,
        Entry::text(
            "META-INF/container.xml",
            CONTAINER_XML.replace("EPUB/package.opf", opf_path),
        ),
        Entry::text(opf_path, opf),
        Entry::text(
            "EPUB/nested/nav.xhtml",
            nav(&[("../text/ch1.xhtml", "Internal dot")]),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Internal dot", "en", "", "<p>AUTH_INTERNAL_TWO_DOT</p>", ""),
        ),
    ])
}

pub fn unicode_normalization_collision() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    entries.push(Entry::binary("EPUB/é.txt", b"NFC".to_vec()));
    entries.push(Entry::binary("EPUB/e\u{301}.txt", b"NFD".to_vec()));
    write_zip_entries(entries)
}

pub fn case_fold_collision() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    entries.push(Entry::binary("EPUB/Case.txt", b"upper".to_vec()));
    entries.push(Entry::binary("EPUB/case.txt", b"lower".to_vec()));
    write_zip_entries(entries)
}

pub fn malformed_truncated_zip() -> Vec<u8> {
    let mut bytes = minimal_reflowable();
    bytes.truncate(bytes.len().saturating_sub(9));
    bytes
}

pub fn encrypted_zip_entry() -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();
        let deflated =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(CONTAINER_XML.as_bytes()).unwrap();
        let encrypted = deflated.with_deprecated_encryption(b"audit-password");
        zip.start_file("EPUB/package.opf", encrypted).unwrap();
        zip.write_all(
            package(
                "",
                r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
                r#"<itemref idref="c1"/>"#,
                "",
            )
            .as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

pub fn missing_container_xml() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    entries.retain(|entry| entry.name != "META-INF/container.xml");
    write_zip_entries(entries)
}

pub fn malformed_container_xml() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    let container = entries
        .iter_mut()
        .find(|entry| entry.name == "META-INF/container.xml")
        .unwrap();
    container.bytes = b"<container><rootfiles>".to_vec();
    write_zip_entries(entries)
}

pub fn rootfile_missing_package_document() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    let container = entries
        .iter_mut()
        .find(|entry| entry.name == "META-INF/container.xml")
        .unwrap();
    container.bytes = CONTAINER_XML
        .replace("EPUB/package.opf", "EPUB/missing.opf")
        .into_bytes();
    write_zip_entries(entries)
}

pub fn mimetype_not_first() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    let mimetype = entries.remove(0);
    entries.insert(1, mimetype);
    write_zip_entries(entries)
}

pub fn wrong_mimetype_payload() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    entries[0].bytes = b"application/epub+zip\n".to_vec();
    write_zip_entries(entries)
}

pub fn compressed_mimetype() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    entries[0].stored = false;
    write_zip_entries(entries)
}

pub fn invalid_unique_identifier() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    let opf = entries
        .iter_mut()
        .find(|entry| entry.name == "EPUB/package.opf")
        .unwrap();
    opf.bytes = String::from_utf8(opf.bytes.clone())
        .unwrap()
        .replace(
            "unique-identifier=\"pub-id\"",
            "unique-identifier=\"missing-id\"",
        )
        .into_bytes();
    write_zip_entries(entries)
}

pub fn unsupported_package_version() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    let opf = entries
        .iter_mut()
        .find(|entry| entry.name == "EPUB/package.opf")
        .unwrap();
    opf.bytes = String::from_utf8(opf.bytes.clone())
        .unwrap()
        .replace("version=\"3.0\"", "version=\"2.0\"")
        .into_bytes();
    write_zip_entries(entries)
}

pub fn unsupported_package_namespace() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    let opf = entries
        .iter_mut()
        .find(|entry| entry.name == "EPUB/package.opf")
        .unwrap();
    opf.bytes = String::from_utf8(opf.bytes.clone())
        .unwrap()
        .replace(
            "xmlns=\"http://www.idpf.org/2007/opf\"",
            "xmlns=\"urn:authority-audit:unsupported-opf\"",
        )
        .into_bytes();
    write_zip_entries(entries)
}

pub fn duplicate_manifest_id() -> Vec<u8> {
    let mut entries = valid_ocf_entries();
    let opf = entries
        .iter_mut()
        .find(|entry| entry.name == "EPUB/package.opf")
        .unwrap();
    let mut source = String::from_utf8(opf.bytes.clone()).unwrap();
    source = source.replace(
        "</manifest>",
        "<item id=\"nav\" href=\"duplicate.xhtml\" media-type=\"application/xhtml+xml\"/></manifest>",
    );
    opf.bytes = source.into_bytes();
    write_zip_entries(entries)
}

pub fn foreign_with_fallback() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="foreign" href="data/object.bin" media-type="application/x-audit-foreign" fallback="c1"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="foreign"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Fallback")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Fallback", "en", "", "<p>AUTH_VALID_FALLBACK</p>", ""),
        ),
        Entry::binary("EPUB/data/object.bin", b"AUTH_FOREIGN_PAYLOAD".to_vec()),
    ])
}

pub fn foreign_binary_with_fallback() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="foreign-image" href="images/foreign.avif" media-type="image/avif" fallback="fallback-image"/><item id="fallback-image" href="images/fallback.png" media-type="image/png"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "Fallback image")]),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Fallback image",
                "en",
                "",
                "<img src=\"../images/foreign.avif\" alt=\"Fallback\"/><p>AUTH_BINARY_FALLBACK</p>",
                "",
            ),
        ),
        Entry::binary(
            "EPUB/images/foreign.avif",
            b"AUTH_UNSUPPORTED_PRIMARY_IMAGE".to_vec(),
        ),
        Entry::binary("EPUB/images/fallback.png", tiny_png()),
    ])
}

pub fn package_relative_paths() -> Vec<u8> {
    let opf_path = "EPUB/nested/package.opf";
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="../text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let mut mimetype = Entry::binary("mimetype", b"application/epub+zip".to_vec());
    mimetype.stored = true;
    write_zip_entries(vec![
        mimetype,
        Entry::text(
            "META-INF/container.xml",
            CONTAINER_XML.replace("EPUB/package.opf", opf_path),
        ),
        Entry::text(opf_path, opf),
        Entry::text(
            "EPUB/nested/nav.xhtml",
            nav(&[("../text/ch1.xhtml", "Package base")]),
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Package base", "en", "", "<p>AUTH_PACKAGE_BASE</p>", ""),
        ),
    ])
}

pub fn broken_spine() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#,
        r#"<itemref idref="missing"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[])),
    ])
}

pub fn foreign_without_fallback() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="foreign" href="data/object.bin" media-type="application/x-audit-foreign"/>"#,
        r#"<itemref idref="foreign"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[])),
        Entry::binary("EPUB/data/object.bin", b"AUTH_FOREIGN".to_vec()),
    ])
}

pub fn large_text(bytes: usize) -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let mut body = String::from("<h1>Large</h1><p>AUTH_LARGE_BEGIN ");
    while body.len() < bytes {
        body.push_str("abcdefghij 日本語 0123456789 ");
    }
    body.push_str(" AUTH_LARGE_END</p>");
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Large")])),
        Entry::text("EPUB/text/ch1.xhtml", xhtml("Large", "ja", "", &body, "")),
    ])
}

fn exact_xhtml_document(bytes: usize, language: &str) -> Vec<u8> {
    let start = "<p>AUTH_HTML_BOUNDARY_BEGIN ";
    let end = " AUTH_HTML_BOUNDARY_END</p>";
    let base = xhtml("Boundary", language, "", &format!("{start}{end}"), "");
    assert!(
        bytes >= base.len(),
        "boundary target must fit XHTML wrapper"
    );
    let filler = "a".repeat(bytes - base.len());
    let document = xhtml(
        "Boundary",
        language,
        "",
        &format!("{start}{filler}{end}"),
        "",
    )
    .into_bytes();
    assert_eq!(
        document.len(),
        bytes,
        "boundary XHTML payload must have the requested exact byte length"
    );
    document
}

pub fn html_size_boundary(below: usize, at: usize) -> (Vec<u8>, Vec<u8>) {
    fn fixture(document: Vec<u8>) -> Vec<u8> {
        let opf = package(
            "",
            r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
            r#"<itemref idref="c1"/>"#,
            "",
        );
        write_epub(vec![
            Entry::text("EPUB/package.opf", opf),
            Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Boundary")])),
            Entry::binary("EPUB/text/ch1.xhtml", document),
        ])
    }

    (
        fixture(exact_xhtml_document(below, "en")),
        fixture(exact_xhtml_document(at, "en")),
    )
}

fn publication_language(language: &'static str) -> Vec<u8> {
    let opf = package_with_language(
        language,
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let marker = format!("AUTH_LANG_{language}");
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Language")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Language", language, "", &format!("<p>{marker}</p>"), ""),
        ),
    ])
}

pub fn publication_languages() -> Vec<(&'static str, Vec<u8>)> {
    ["ja", "en-US", "zh-CN", "zh-TW", "ko"]
        .into_iter()
        .map(|language| (language, publication_language(language)))
        .collect()
}

pub fn html_file_count(count: usize) -> Vec<u8> {
    html_file_count_with_non_html_resources(count, 0)
}

pub fn html_file_count_with_non_html_resources(count: usize, resource_count: usize) -> Vec<u8> {
    let mut manifest = String::from(
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>",
    );
    let mut spine = String::new();
    let mut nav_entries = Vec::new();
    let mut entries = Vec::new();
    for i in 0..count {
        manifest.push_str(&format!(
            "<item id=\"c{i}\" href=\"text/c{i}.xhtml\" media-type=\"application/xhtml+xml\"/>"
        ));
        spine.push_str(&format!("<itemref idref=\"c{i}\"/>"));
        nav_entries.push((format!("text/c{i}.xhtml"), format!("Chapter {i}")));
        entries.push(Entry::text(
            format!("EPUB/text/c{i}.xhtml"),
            xhtml(
                &format!("C{i}"),
                "en",
                "",
                &format!("<p>AUTH_COUNT_{i}</p>"),
                "",
            ),
        ));
    }
    for i in 0..resource_count {
        match i % 3 {
            0 => {
                manifest.push_str(&format!(
                    "<item id=\"asset{i}\" href=\"assets/resource{i}.png\" media-type=\"image/png\"/>"
                ));
                entries.push(Entry::binary(
                    format!("EPUB/assets/resource{i}.png"),
                    tiny_png(),
                ));
            }
            1 => {
                manifest.push_str(&format!(
                    "<item id=\"asset{i}\" href=\"assets/resource{i}.css\" media-type=\"text/css\"/>"
                ));
                entries.push(Entry::text(
                    format!("EPUB/assets/resource{i}.css"),
                    "body { color: #000; }",
                ));
            }
            _ => {
                manifest.push_str(&format!(
                    "<item id=\"asset{i}\" href=\"assets/resource{i}.ttf\" media-type=\"font/ttf\"/>"
                ));
                entries.push(Entry::binary(
                    format!("EPUB/assets/resource{i}.ttf"),
                    include_bytes!("../fixtures/embedded-font/fonts/EBGaramond12-Bold.ttf")
                        .to_vec(),
                ));
            }
        }
    }
    let opf = package("", &manifest, &spine, "");
    let refs: Vec<(&str, &str)> = nav_entries
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    entries.push(Entry::text("EPUB/package.opf", opf));
    entries.push(Entry::text("EPUB/nav.xhtml", nav(&refs)));
    write_epub(entries)
}

fn idpf_obfuscate(font: &[u8], identifier: &str) -> Vec<u8> {
    // SHA-1 of the whitespace-stripped unique identifier, per EPUB 3.3 font obfuscation.
    let key = sha1(identifier.as_bytes());
    let mut out = font.to_vec();
    for i in 0..out.len().min(1040) {
        out[i] ^= key[i % key.len()];
    }
    out
}

fn sha1(data: &[u8]) -> [u8; 20] {
    // Small independent SHA-1 implementation used only to construct the normative test vector.
    let mut h0 = 0x67452301u32;
    let mut h1 = 0xEFCDAB89u32;
    let mut h2 = 0x98BADCFEu32;
    let mut h3 = 0x10325476u32;
    let mut h4 = 0xC3D2E1F0u32;
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
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
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, h) in [h0, h1, h2, h3, h4].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&h.to_be_bytes());
    }
    out
}

fn tiny_png() -> Vec<u8> {
    // 1x1 opaque white PNG, fixed bytes. The image format is externally defined and not derived from production code.
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5,
        0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64,
        0xf8, 0x0f, 0x00, 0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

fn oversized_png() -> Vec<u8> {
    let mut png = tiny_png();
    png[16..20].copy_from_slice(&16_385u32.to_be_bytes());
    png[20..24].copy_from_slice(&1u32.to_be_bytes());
    let crc = png_crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in data.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(ALPHABET[(first >> 2) as usize] as char);
        output.push(ALPHABET[((first & 0x03) << 4 | second >> 4) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((second & 0x0f) << 2 | third >> 6) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

pub fn malformed_xhtml() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Bad XML")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>AUTH_BAD_XML</body></html>",
        ),
    ])
}

pub fn missing_navigation() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("No Nav", "en", "", "<p>AUTH_NO_NAV</p>", ""),
        ),
    ])
}

pub fn duplicate_toc_navigation() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let nav_doc = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Dup</title></head><body><nav epub:type="toc"><ol><li><a href="text/ch1.xhtml">A</a></li></ol></nav><nav epub:type="toc"><ol><li><a href="text/ch1.xhtml">B</a></li></ol></nav></body></html>"#;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav_doc),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("One", "en", "", "<p>AUTH_DUP_TOC</p>", ""),
        ),
    ])
}

pub fn media_overlay() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml" media-overlay="smil1"/><item id="smil1" href="overlays/ch1.smil" media-type="application/smil+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let smil = r#"<smil xmlns="http://www.w3.org/ns/SMIL" version="3.0"><body><seq><par><text src="../text/ch1.xhtml#p1"/><audio src="audio.mp3" clipBegin="0s" clipEnd="1s"/></par></seq></body></smil>"#;
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Overlay")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Overlay",
                "en",
                "",
                "<p id=\"p1\">AUTH_MEDIA_OVERLAY</p>",
                "",
            ),
        ),
        Entry::text("EPUB/overlays/ch1.smil", smil),
    ])
}

pub fn path_traversal_resource() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="../../escape.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("../../escape.xhtml", "Escape")])),
        Entry::text(
            "escape.xhtml",
            xhtml("Escape", "en", "", "<p>AUTH_ESCAPE</p>", ""),
        ),
    ])
}

pub fn invalid_font_encryption() -> Vec<u8> {
    // Rebuild with an unsupported encryption algorithm instead of mutating the valid fixture.
    let font = include_bytes!("../fixtures/embedded-font/fonts/EBGaramond12-Bold.ttf").to_vec();
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/font.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="font" href="fonts/audit.ttf" media-type="font/ttf"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Font")])),
        Entry::text(
            "EPUB/styles/font.css",
            "@font-face{font-family:'Audit';src:url('../fonts/audit.ttf')}",
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Font",
                "en",
                "",
                "<p>AUTH_BAD_ENCRYPTION</p>",
                "<link rel=\"stylesheet\" href=\"../styles/font.css\"/>",
            ),
        ),
        Entry::binary("EPUB/fonts/audit.ttf", font),
        Entry::text(
            "META-INF/encryption.xml",
            r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:EncryptedData><enc:EncryptionMethod Algorithm="urn:example:unsupported-cipher"/><enc:CipherData><enc:CipherReference URI="EPUB/fonts/audit.ttf"/></enc:CipherData></enc:EncryptedData></encryption>"#,
        ),
    ])
}

pub fn zero_byte_font() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/font.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/><item id="font" href="fonts/zero.ttf" media-type="font/ttf"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Zero font")])),
        Entry::text(
            "EPUB/styles/font.css",
            "@font-face{font-family:'Zero';src:url('../fonts/zero.ttf')} p{font-family:'Zero'}",
        ),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "Zero",
                "en",
                "",
                "<p>AUTH_ZERO_FONT</p>",
                "<link rel=\"stylesheet\" href=\"../styles/font.css\"/>",
            ),
        ),
        Entry::binary("EPUB/fonts/zero.ttf", vec![]),
    ])
}

pub fn media_queries() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="styles/mq.css" media-type="text/css"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    let css = ".common{font-style:italic}@media amzn-kf8{.kf8{font-weight:bold}}@media amzn-mobi{.mobi{font-size:3em}}";
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "MQ")])),
        Entry::text("EPUB/styles/mq.css", css),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "MQ",
                "en",
                "",
                "<p class=\"common kf8 mobi\">AUTH_MEDIA_QUERY</p>",
                "<link rel=\"stylesheet\" href=\"../styles/mq.css\"/>",
            ),
        ),
    ])
}

pub fn large_navigation_index() -> Vec<u8> {
    let mut manifest = String::from(
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#,
    );
    let mut spine = String::new();
    let mut content = Vec::new();
    for section in 0..299 {
        manifest.push_str(&format!(
            "<item id=\"c{section}\" href=\"text/c{section}.xhtml\" media-type=\"application/xhtml+xml\"/>"
        ));
        spine.push_str(&format!("<itemref idref=\"c{section}\"/>"));
        content.push(Entry::text(
            format!("EPUB/text/c{section}.xhtml"),
            xhtml(
                &format!("Large {section}"),
                "en",
                "",
                &format!("<h1 id=\"target{section}\">AUTH_LARGE_INDEX_{section}</h1>"),
                "",
            ),
        ));
    }
    let mut items = String::new();
    for group in 0..20 {
        let first_section = group * 15;
        let last_section = (first_section + 15).min(299);
        if first_section == last_section {
            break;
        }
        items.push_str(&format!(
            "<li><a href=\"text/c{first_section}.xhtml#target{first_section}\">Group {group:02}</a><ol>"
        ));
        for section in first_section..last_section {
            items.push_str(&format!(
                "<li><a href=\"text/c{section}.xhtml#target{section}\">Section {section:03}</a><ol>"
            ));
            for item in 0..20 {
                items.push_str(&format!(
                    "<li><a href=\"text/c{section}.xhtml#target{section}\">Large navigation label {section:03}-{item:02} with deterministic boundary payload</a></li>"
                ));
            }
            items.push_str("</ol></li>");
        }
        items.push_str("</ol></li>");
    }
    let nav_doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Large</title></head><body><nav epub:type="toc"><ol>{items}</ol></nav><nav epub:type="landmarks"><ol><li><a epub:type="bodymatter" href="text/c0.xhtml#target0">Start</a></li></ol></nav></body></html>"#
    );
    let opf = package("", &manifest, &spine, "");
    let mut output = vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav_doc),
    ];
    output.extend(content);
    write_epub(output)
}

pub fn unsupported_layout_flow() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        r#"<meta property="rendition:flow">unsupported-scrolling-flow</meta>"#,
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Flow")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Flow", "en", "", "<p>AUTH_SCROLLED_FLOW</p>", ""),
        ),
    ])
}

pub fn malformed_package() -> Vec<u8> {
    write_epub(vec![Entry::text(
        "EPUB/package.opf",
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata><"#,
    )])
}

pub fn linear_semantics() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="text/one.xhtml" media-type="application/xhtml+xml"/>
<item id="aux" href="text/aux.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/two.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/><itemref idref="aux" linear="no"/><itemref idref="c2"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text(
            "EPUB/nav.xhtml",
            nav(&[("text/one.xhtml", "One"), ("text/two.xhtml", "Two")]),
        ),
        Entry::text(
            "EPUB/text/one.xhtml",
            xhtml("One", "en", "", "<p>AUTH_LINEAR_ONE</p>", ""),
        ),
        Entry::text(
            "EPUB/text/aux.xhtml",
            xhtml("Aux", "en", "", "<p>AUTH_LINEAR_AUXILIARY</p>", ""),
        ),
        Entry::text(
            "EPUB/text/two.xhtml",
            xhtml("Two", "en", "", "<p>AUTH_LINEAR_TWO</p>", ""),
        ),
    ])
}

pub fn metadata_extension() -> Vec<u8> {
    let opf = package(
        r##"<meta property="schema:accessMode">textual</meta><meta property="audit:unknown" refines="#pub-id">AUTH_UNKNOWN_META</meta>"##,
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "Meta")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("Meta", "en", "", "<p>AUTH_METADATA_EXTENSION_BODY</p>", ""),
        ),
    ])
}

pub fn duplicate_container_path() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    raw_stored_zip(vec![
        ("mimetype", b"application/epub+zip".to_vec()),
        ("META-INF/container.xml", CONTAINER_XML.as_bytes().to_vec()),
        ("EPUB/package.opf", opf.into_bytes()),
        (
            "EPUB/nav.xhtml",
            nav(&[("text/ch1.xhtml", "One")]).into_bytes(),
        ),
        (
            "EPUB/text/ch1.xhtml",
            xhtml("One", "en", "", "<p>AUTH_DUPLICATE_PATH_A</p>", "").into_bytes(),
        ),
        (
            "EPUB/text/ch1.xhtml",
            xhtml(
                "One duplicate",
                "en",
                "",
                "<p>AUTH_DUPLICATE_PATH_B</p>",
                "",
            )
            .into_bytes(),
        ),
    ])
}

fn valid_ocf_entries() -> Vec<Entry> {
    let mut mimetype = Entry::binary("mimetype", b"application/epub+zip".to_vec());
    mimetype.stored = true;
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    );
    vec![
        mimetype,
        Entry::text("META-INF/container.xml", CONTAINER_XML),
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "One")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml("One", "en", "", "<p>AUTH_VALID_OCF</p>", ""),
        ),
    ]
}

fn write_zip_entries(entries: Vec<Entry>) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflated =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for entry in entries {
            let options = if entry.stored { stored } else { deflated };
            zip.start_file(entry.name, options).unwrap();
            zip.write_all(&entry.bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn raw_stored_zip(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    raw_zip_with_methods(
        entries
            .into_iter()
            .map(|(name, data)| (name, data, 0))
            .collect(),
    )
}

fn raw_zip_with_methods(entries: Vec<(&str, Vec<u8>, u16)>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    let mut count = 0u16;
    for (name, data, method) in entries {
        count = count.checked_add(1).expect("ZIP entry count");
        let name = name.as_bytes();
        let offset = bytes.len() as u32;
        let crc = crc32(&data);
        bytes.extend_from_slice(b"PK\x03\x04");
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, method);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, crc);
        push_u32(&mut bytes, data.len() as u32);
        push_u32(&mut bytes, data.len() as u32);
        push_u16(&mut bytes, name.len() as u16);
        push_u16(&mut bytes, 0);
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&data);

        central.extend_from_slice(b"PK\x01\x02");
        push_u16(&mut central, 20);
        push_u16(&mut central, 20);
        push_u16(&mut central, 0);
        push_u16(&mut central, method);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, crc);
        push_u32(&mut central, data.len() as u32);
        push_u32(&mut central, data.len() as u32);
        push_u16(&mut central, name.len() as u16);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, 0);
        push_u32(&mut central, offset);
        central.extend_from_slice(name);
    }
    let central_offset = bytes.len() as u32;
    bytes.extend_from_slice(&central);
    bytes.extend_from_slice(b"PK\x05\x06");
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, count);
    push_u16(&mut bytes, count);
    push_u32(&mut bytes, central.len() as u32);
    push_u32(&mut bytes, central_offset);
    push_u16(&mut bytes, 0);
    bytes
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

pub fn unsupported_zip_compression() -> Vec<u8> {
    let entries = valid_ocf_entries();
    raw_zip_with_methods(
        entries
            .iter()
            .map(|entry| {
                let method = if entry.name == "EPUB/package.opf" {
                    99
                } else {
                    0
                };
                (entry.name.as_str(), entry.bytes.clone(), method)
            })
            .collect(),
    )
}

pub fn rtl_progression() -> Vec<u8> {
    let opf = package(
        "",
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="c1"/>"#,
        "",
    ).replace("<spine page-progression-direction=\"ltr\">", "<spine page-progression-direction=\"rtl\">");
    write_epub(vec![
        Entry::text("EPUB/package.opf", opf),
        Entry::text("EPUB/nav.xhtml", nav(&[("text/ch1.xhtml", "RTL")])),
        Entry::text(
            "EPUB/text/ch1.xhtml",
            xhtml(
                "RTL",
                "ja",
                "dir=\"rtl\"",
                "<p>AUTH_RTL_PROGRESSION</p>",
                "<style>html{writing-mode:vertical-rl;-webkit-writing-mode:vertical-rl}</style>",
            ),
        ),
    ])
}
