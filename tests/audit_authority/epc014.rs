use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::{TargetProjection, decode_position_href};
use crate::audit_support::temp::TempDir;

#[test]
fn epc014_unresolvable_root_relative_web_links_degrade_without_aborting() {
    let temp = TempDir::new("epc014-root-relative-web-links");
    let input = temp.write(
        "epc014-root-relative-web-links.epub",
        &epub::epc014_unresolvable_root_relative_web_links(),
    );
    let output_path = temp.path().join("epc014-root-relative-web-links.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-014 CLI process starts");

    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        cli.status.code(),
        Some(1),
        "unresolvable root-relative web links degrade with warning-success: {stderr}"
    );
    assert!(output_path.is_file(), "CLI writes the AZW3 artifact");
    let warning_lines = stderr
        .lines()
        .filter(|line| line.contains("warning["))
        .collect::<Vec<_>>();
    assert_eq!(
        warning_lines.len(),
        1,
        "multiple unresolved root-relative links emit one warning: {stderr}"
    );
    assert!(warning_lines[0].contains("warning[W006]:"));
    assert!(stderr.contains("unresolvable root-relative web-style hyperlinks"));
    assert!(
        !stderr.contains("does not resolve to a generated document"),
        "unresolved web-style links do not abort position construction: {stderr}"
    );

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert!(target.body_text().contains("ROOT-W-INDEX"));
    assert!(target.body_text().contains("ROOT-W-INDEX-FRAGMENT"));
    assert!(target.body_text().contains("ROOT-WIKI"));
    assert!(target.body_text().contains("ROOT-WIKI-QUERY-FRAGMENT"));
    assert!(
        target
            .body_text()
            .contains("AUTH_EPC014_CHAPTER_TWO_TARGET")
    );
    assert!(target.rawml.contains("ROOT-W-INDEX</a>"));
    assert!(target.rawml.contains("ROOT-WIKI</a>"));
    for unresolved in [
        "/w/index.php?title=Example",
        "/w/index.php?title=Example#edit",
        "/wiki/Example",
        "/wiki/Second?source=epub#target",
    ] {
        assert!(
            !target.rawml.contains(unresolved),
            "unresolved root-relative destination is absent from active RawML: {unresolved}"
        );
    }

    let first = &target.sections[0];
    let internal_link = |label: &str| {
        first
            .links
            .iter()
            .find(|(_, text)| text == label)
            .map(|(href, _)| href.as_str())
            .unwrap_or_else(|| panic!("generated link {label} is present"))
    };
    assert_eq!(
        internal_link("ABSOLUTE-HTTPS"),
        "https://example.com/absolute",
        "absolute external URL handling is unchanged"
    );
    assert_eq!(
        internal_link("ABSOLUTE-MAILTO"),
        "mailto:authors@example.com",
        "mailto external URL handling is unchanged"
    );
    let local_position = decode_position_href(internal_link("LOCAL-FRAGMENT"))
        .expect("local fragment resolves to a Kindle position");
    let relative_position = decode_position_href(internal_link("RELATIVE-INTERNAL"))
        .expect("relative document query/fragment resolves to a Kindle position");
    let root_relative_position = decode_position_href(internal_link("ROOT-RELATIVE-INTERNAL"))
        .expect("valid root-relative EPUB document resolves to a Kindle position");
    assert_eq!(local_position.0, 0, "local fragment targets chapter one");
    assert_eq!(relative_position.0, 1, "relative href targets chapter two");
    assert_eq!(root_relative_position, relative_position);
    assert!(target.sections[1].ids.contains("target"));
    assert_eq!(
        target.ncx_entries().expect("generated navigation parses")[0].label,
        "Chapter One",
        "normal TOC navigation remains available"
    );
    assert!(
        !first.links.iter().any(|(_, label)| {
            matches!(
                label.as_str(),
                "ROOT-W-INDEX" | "ROOT-W-INDEX-FRAGMENT" | "ROOT-WIKI" | "ROOT-WIKI-QUERY-FRAGMENT"
            )
        }),
        "degraded anchors retain their text without active hyperlink destinations"
    );
}
