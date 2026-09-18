use std::io::{Cursor, Read};
use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::{TargetProjection, decode_position_href};
use crate::audit_support::temp::TempDir;

#[test]
fn epc008_direct_bitmap_spine_without_presentation_is_rejected() {
    let fixture = epub::epc008_intrinsic_bitmap_spine_without_viewport();
    let mut archive = zip::ZipArchive::new(Cursor::new(fixture.as_slice()))
        .expect("open the synthetic EPC-008 EPUB");
    let mut package = String::new();
    archive
        .by_name("EPUB/package.opf")
        .expect("read package document")
        .read_to_string(&mut package)
        .expect("decode package document");
    assert!(package.contains("rendition:layout"));
    assert!(package.contains("<itemref idref=\"page\""));
    assert!(package.contains("media-type=\"image/png\""));
    assert!(!package.contains("viewport"));
    assert!(!package.contains("original-resolution"));
    let mut fallback = String::new();
    archive
        .by_name("EPUB/fallback.xhtml")
        .expect("read text fallback without page metadata")
        .read_to_string(&mut fallback)
        .expect("decode fallback XHTML");
    assert!(fallback.contains("AUTH_EPC008_FALLBACK_BODY"));
    assert!(!fallback.contains("viewport"));

    let mut bitmap = Vec::new();
    archive
        .by_name("EPUB/images/page.png")
        .expect("read direct bitmap spine resource")
        .read_to_end(&mut bitmap)
        .expect("read bitmap bytes");
    let decoded_bitmap = image::load_from_memory(&bitmap).expect("inspect intrinsic PNG geometry");
    assert_eq!(
        (decoded_bitmap.width(), decoded_bitmap.height()),
        (1, 1),
        "the input contains intrinsic bitmap geometry without viewport evidence"
    );

    let temp = TempDir::new("epc008-intrinsic-bitmap-reject");
    let input = temp.write("epc008-direct-bitmap.epub", &fixture);
    let output_path = temp.path().join("epc008-direct-bitmap.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-008 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(2),
        "a direct bitmap spine item without Kindle page presentation is a fatal reject: {stderr}"
    );
    assert!(
        stderr.contains("pre-paginated section fallback.xhtml has no page presentation"),
        "the direct raster item and its text fallback cannot become a Kindle page from intrinsic dimensions: {stderr}"
    );
    assert!(
        !stderr.contains("warning["),
        "intentional EPC-008 rejection does not emit a warning: {stderr}"
    );
    assert!(
        !output_path.exists(),
        "rejected direct bitmap input does not produce an artifact carrying fabricated resolution"
    );
}

#[test]
fn epc008_direct_svg_spine_intrinsic_geometry_is_not_page_presentation() {
    let fixture = epub::epc008_intrinsic_svg_spine_without_viewport();
    let mut archive = zip::ZipArchive::new(Cursor::new(fixture.as_slice()))
        .expect("open the synthetic EPC-008 SVG EPUB");
    let mut package = String::new();
    archive
        .by_name("EPUB/package.opf")
        .expect("read package document")
        .read_to_string(&mut package)
        .expect("decode package document");
    assert!(package.contains("rendition:layout"));
    assert!(package.contains("<itemref idref=\"page\""));
    assert!(package.contains("media-type=\"image/svg+xml\""));
    assert!(!package.contains("viewport"));
    assert!(!package.contains("original-resolution"));

    let mut svg = String::new();
    archive
        .by_name("EPUB/images/page.svg")
        .expect("read direct SVG spine resource")
        .read_to_string(&mut svg)
        .expect("decode SVG source");
    assert!(svg.contains("width=\"1200\""));
    assert!(svg.contains("height=\"1577\""));
    assert!(svg.contains("viewBox=\"0 0 1200 1577\""));

    let temp = TempDir::new("epc008-intrinsic-svg-reject");
    let input = temp.write("epc008-direct-svg.epub", &fixture);
    let output_path = temp.path().join("epc008-direct-svg.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-008 SVG CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(2),
        "a direct SVG spine item without XHTML viewport is a fatal reject: {stderr}"
    );
    assert!(
        stderr.contains("has no page presentation"),
        "SVG intrinsic geometry alone must not supply page presentation: {stderr}"
    );
    assert!(
        !stderr.contains("warning["),
        "intentional EPC-008 SVG rejection does not emit a warning: {stderr}"
    );
    assert!(
        !output_path.exists(),
        "rejected direct SVG input has no artifact or fabricated original-resolution"
    );
}

#[test]
fn epc008_direct_svg_navigation_targets_resolve_to_generated_wrapper_sections() {
    let fixture = epub::epc008_direct_svg_navigation_targets(false);
    let temp = TempDir::new("direct-svg-navigation-target-mapping");
    let input = temp.write("direct-svg-navigation-target-mapping.epub", &fixture);
    let output_path = temp
        .path()
        .join("direct-svg-navigation-target-mapping.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("direct SVG navigation CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "reflowable direct SVG navigation resolves to generated wrapper sections: {stderr}"
    );
    assert!(stderr.is_empty(), "fixture has no warnings: {stderr}");
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");

    let artifact = std::fs::read(&output_path).expect("read generated AZW3");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    let target_section_index = target
        .sections
        .iter()
        .position(|section| section.ids.contains("svg-target"))
        .expect("target SVG id is carried by its generated wrapper section");
    let first_section_index = target
        .sections
        .iter()
        .position(|section| section.ids.contains("svg-first"))
        .expect("first SVG id is carried by its own generated wrapper section");
    assert_ne!(
        first_section_index, target_section_index,
        "distinct source SVGs retain distinct generated section identities"
    );
    let entries = target
        .ncx_entries()
        .expect("inspect emitted TOC destinations");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].label, "Target SVG page");
    assert_eq!(
        entries[0].sequence as usize, target_section_index,
        "document#fragment navigation resolves to the generated target SVG section"
    );
    let page_list_position = target
        .sections
        .iter()
        .flat_map(|section| &section.links)
        .find(|(_, label)| label == "SVG page target")
        .and_then(|(href, _)| decode_position_href(href))
        .expect("page-list target is rewritten to a Kindle position");
    assert_eq!(
        page_list_position,
        (entries[0].sequence, entries[0].offset),
        "page-list and TOC resolve to the same generated SVG fragment"
    );
    assert!(
        target.sections[target_section_index]
            .visible_text
            .contains("AUTH_DIRECT_SVG_POSITION_TARGET")
    );
    assert!(
        !target.rawml.contains("../Content/page.svg#svg-target")
            && !target.rawml.contains("Content/page.svg#svg-target"),
        "source navigation paths are materialized as Kindle positions"
    );
}

#[test]
fn epc008_direct_svg_navigation_reaches_no_page_presentation_reject() {
    let fixture = epub::epc008_direct_svg_navigation_targets(true);
    let temp = TempDir::new("epc008-direct-svg-navigation-reject");
    let input = temp.write("epc008-direct-svg-navigation-reject.epub", &fixture);
    let output_path = temp.path().join("epc008-direct-svg-navigation-reject.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("pre-paginated direct SVG navigation CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(2),
        "direct SVG without explicit XHTML viewport remains an EPC-008 reject: {stderr}"
    );
    assert!(
        stderr.contains("has no page presentation"),
        "intrinsic geometry cannot satisfy the fixed-layout requirement: {stderr}"
    );
    assert!(
        !stderr.contains("unresolved position target")
            && !stderr.contains("position target does not resolve"),
        "a valid source SVG navigation target is not misreported as unresolved: {stderr}"
    );
    assert!(
        !stderr.contains("warning["),
        "reject is warning-free: {stderr}"
    );
    assert!(
        !output_path.exists(),
        "intentional EPC-008 reject has no artifact"
    );
}

#[test]
fn svg_fragment_only_css_url_reaches_the_epc008_reject() {
    let fixture = epub::epc008_svg_spine_with_css_urls("");
    let temp = TempDir::new("epc008-svg-fragment-only-css-url");
    let input = temp.write("fragment-only-svg-url.epub", &fixture);
    let output_path = temp.path().join("fragment-only-svg-url.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("fragment-only SVG CSS URL CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(2),
        "direct SVG without explicit XHTML viewport remains an EPC-008 reject: {stderr}"
    );
    assert!(
        stderr.contains("has no page presentation"),
        "fragment-only url(#paint) must reach the EPC-008 page-presentation rejection: {stderr}"
    );
    assert!(
        !stderr.contains("CSS resource path"),
        "same-document url(#paint) is not an EPUB resource path: {stderr}"
    );
    assert!(
        !stderr.contains("warning["),
        "the EPC-008 reject remains warning-free: {stderr}"
    );
    assert!(!output_path.exists(), "the rejected SVG has no artifact");
}

#[test]
fn svg_css_path_fragment_and_relative_urls_keep_normal_path_validation() {
    let fixture = epub::epc008_svg_spine_with_css_urls(
        "stroke: url(file.svg#external-paint); marker: url(../valid/inside.png);",
    );
    let temp = TempDir::new("epc008-svg-css-path-fragment");
    let input = temp.write("path-fragment-svg-url.epub", &fixture);
    let output_path = temp.path().join("path-fragment-svg-url.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("path-plus-fragment SVG CSS URL CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(2),
        "valid local resource paths still advance to EPC-008 rejection: {stderr}"
    );
    assert!(
        stderr.contains("has no page presentation"),
        "path.svg#fragment and a valid relative URL pass path validation: {stderr}"
    );
    assert!(
        !stderr.contains("CSS resource path"),
        "valid local paths do not become CSS path failures: {stderr}"
    );
    assert!(!stderr.contains("warning["));
    assert!(!output_path.exists());
}

#[test]
fn svg_css_epub_root_escape_is_still_rejected() {
    let fixture = epub::epc008_svg_spine_with_css_urls("stroke: url(../../outside.png);");
    let temp = TempDir::new("epc008-svg-css-root-escape");
    let input = temp.write("escaping-svg-url.epub", &fixture);
    let output_path = temp.path().join("escaping-svg-url.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("escaping SVG CSS URL CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(cli.status.code(), Some(2));
    assert!(
        stderr.contains("CSS resource path ../../outside.png escapes the EPUB root"),
        "path-bearing CSS URLs still go through EPUB-root escape validation: {stderr}"
    );
    assert!(!stderr.contains("warning["));
    assert!(!output_path.exists());
}

#[test]
fn epc008_explicit_viewport_xhtml_with_inline_svg_remains_supported() {
    let fixture = epub::epc008_fixed_layout_xhtml_with_inline_svg();
    let temp = TempDir::new("epc008-explicit-viewport-inline-svg");
    let input = temp.write("epc008-inline-svg-with-viewport.epub", &fixture);
    let output_path = temp.path().join("epc008-inline-svg-with-viewport.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("explicit-viewport XHTML SVG CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(0),
        "XHTML viewport authorizes the inline SVG page flow: {stderr}"
    );
    assert!(stderr.is_empty(), "explicit-viewport SVG is warning-free");
    assert!(
        output_path.is_file(),
        "the explicit-viewport page is generated"
    );
}
