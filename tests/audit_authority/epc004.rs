use std::process::Command;

use crate::audit_support::epub;
use crate::audit_support::semantic::TargetProjection;
use crate::audit_support::temp::TempDir;

#[test]
fn epc004_pronunciation_lexicon_survives_without_becoming_a_kindle_resource() {
    let temp = TempDir::new("epc004-pronunciation-lexicon");
    let input = temp.write(
        "epc004-pronunciation.epub",
        &epub::epc004_pronunciation_lexicon(),
    );
    let output_path = temp.path().join("epc004-pronunciation.azw3");
    let cli = Command::new(env!("CARGO_BIN_EXE_epub3-kindle"))
        .arg(&input)
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("EPC-004 CLI process starts");

    assert_eq!(
        cli.status.code(),
        Some(1),
        "the omitted pronunciation lexicon produces a warning-success exit: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(
        output_path.is_file(),
        "warning-success conversion writes the AZW3 artifact"
    );
    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        stderr.matches("warning[W002]:").count(),
        1,
        "omitting one or more PLS resources emits W002 exactly once: {stderr}"
    );
    assert!(stderr.contains("PLS pronunciation lexicon semantics were dropped for Kindle output"));

    let artifact = std::fs::read(&output_path).expect("CLI writes the AZW3 artifact");
    let target = TargetProjection::parse(&artifact).expect("inspect generated Kindle content");
    assert!(
        target
            .body_text()
            .contains("AUTH_EPC004_PRONUNCIATION_BODY"),
        "the readable chapter body survives"
    );
    assert!(target.rawml.contains(
        "<link rel=\"pronunciation\" type=\"application/pls+xml\" href=\"../lexicon/en.pls\"/>"
    ));
    assert!(target.rawml.contains(
        "<link rel=\"alternate pronunciation\" type=\"application/pls+xml\" href=\"../lexicon/de.pls\"/>"
    ));
    assert!(
        target
            .rawml
            .contains("ssml:ph=\"AUTH_PHONEME\" ssml:alphabet=\"ipa\"")
    );
    assert!(
        target.rawml.contains("href=\"kindle:pos:fid:"),
        "ordinary local anchors continue to be rewritten"
    );

    assert!(
        target
            .resc_resource_hrefs()
            .expect("inspect output resource references")
            .iter()
            .all(|href| !href.ends_with(".pls"))
    );
    let fdst = target
        .header
        .fdst_record
        .expect("generated KF8 has an FDST record") as usize;
    let resc = (0..target.db.record_count())
        .find(|&index| {
            target
                .db
                .record(index)
                .is_ok_and(|record| record.starts_with(b"RESC"))
        })
        .expect("generated KF8 has a RESC record");
    assert_eq!(
        resc,
        fdst + 1,
        "the output resource list is empty for this fixture"
    );
    assert!(
        !artifact
            .windows(b"AUTH_EPC004_PLS_PAYLOAD".len())
            .any(|window| window == b"AUTH_EPC004_PLS_PAYLOAD"),
        "the PLS payload is not embedded in the Kindle artifact"
    );
    assert!(
        !artifact
            .windows(b"AUTH_EPC004_PLS_PAYLOAD_DE".len())
            .any(|window| window == b"AUTH_EPC004_PLS_PAYLOAD_DE"),
        "the second PLS payload is not embedded in the Kindle artifact"
    );
}
