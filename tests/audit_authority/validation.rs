use std::path::Path;

use epub3_kindle::{
    Compression, ConvertOptions, WarningCode, convert_bytes, convert_bytes_with_warnings,
    convert_file,
};

use crate::audit_support::epub;
use crate::audit_support::palm::{PalmDb, assert_record_pointer};
use crate::audit_support::temp::TempDir;

#[test]
fn conversion_is_deterministic_for_identical_input_and_options() {
    // REQ: VAL-001, VAL-004
    let input = epub::minimal_reflowable();
    let options = ConvertOptions {
        compression: Compression::PalmDoc,
    };
    let a = convert_bytes(&input, &options).unwrap();
    let b = convert_bytes(&input, &options).unwrap();
    assert_eq!(
        a, b,
        "same input/options must produce structurally comparable deterministic output"
    );
}

#[test]
fn file_api_accepts_only_epub_input_and_declared_azw3_or_mobi_outputs() {
    // REQ: API-001..API-004
    let tmp = TempDir::new("extensions");
    let src = tmp.write("input.epub", &epub::minimal_reflowable());
    let bad_input = tmp.write("input.bin", &epub::minimal_reflowable());
    let options = ConvertOptions {
        compression: Compression::PalmDoc,
    };
    let input_error = convert_file(&bad_input, tmp.path().join("bad-input.azw3"), &options)
        .expect_err("non-EPUB input extension must be rejected before conversion");
    assert!(
        input_error.to_string().contains("unsupported input"),
        "input extension failure must be classified as unsupported input: {input_error}"
    );
    assert!(convert_file(&src, tmp.path().join("book.azw3"), &options).is_ok());
    assert!(convert_file(&src, tmp.path().join("book.mobi"), &options).is_ok());
    for bad in ["book.bin", "book.azw", "book.txt", "book"] {
        assert!(
            convert_file(&src, tmp.path().join(bad), &options).is_err(),
            "unsupported output extension {bad} must be explicit error"
        );
    }
}

#[test]
fn failed_conversion_does_not_destroy_an_existing_destination() {
    // REQ: API-005
    let tmp = TempDir::new("atomic");
    let src = tmp.write("broken.epub", &epub::broken_spine());
    let dst = tmp.write("book.azw3", b"KEEP_EXISTING_OUTPUT");
    let options = ConvertOptions {
        compression: Compression::PalmDoc,
    };
    assert!(convert_file(&src, &dst, &options).is_err());
    assert_eq!(
        std::fs::read(&dst).unwrap(),
        b"KEEP_EXISTING_OUTPUT",
        "failure must not truncate/replace a previously valid destination"
    );
}

#[test]
fn amazon_individual_html_size_boundary_warns_and_converts_at_and_above_limit() {
    // REQ: AMZ-QA-001, SEC-005
    let (below, at) = epub::html_size_boundary(29_999_999, 30_000_000);
    assert!(
        convert_bytes(&below, &ConvertOptions::default()).is_ok(),
        "an individual XHTML document below Amazon's decimal 30 MB limit must convert"
    );
    let at_outcome = convert_bytes_with_warnings(&at, &ConvertOptions::default())
        .expect("an individual XHTML document at the guidance limit must convert");
    assert!(
        !at_outcome.value().is_empty()
            && at_outcome
                .warnings()
                .iter()
                .any(|warning| warning.code == WarningCode::W006),
        "the exact-boundary output must exist with one publishing warning"
    );
    let (_, above) = epub::html_size_boundary(30_000_000, 30_000_001);
    let above_outcome = convert_bytes_with_warnings(&above, &ConvertOptions::default())
        .expect("an individual XHTML document above the guidance limit must convert");
    assert!(!above_outcome.value().is_empty());
    assert!(
        above_outcome
            .warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::W006)
    );
}

#[test]
fn amazon_html_document_count_boundary_warns_and_converts_at_and_above_limit() {
    // REQ: AMZ-QA-002, SEC-006
    let accepted = epub::html_file_count(299);
    assert!(
        convert_bytes(&accepted, &ConvertOptions::default()).is_ok(),
        "299 HTML/XHTML content documents must be accepted"
    );
    let accepted_with_resources = epub::html_file_count_with_non_html_resources(299, 64);
    assert!(
        convert_bytes(&accepted_with_resources, &ConvertOptions::default()).is_ok(),
        "non-HTML ZIP resources must not count toward Amazon's HTML document limit"
    );
    for count in [300, 538] {
        let outcome =
            convert_bytes_with_warnings(&epub::html_file_count(count), &ConvertOptions::default())
                .unwrap_or_else(|error| {
                    panic!("{count} HTML/XHTML documents must convert: {error}")
                });
        assert!(!outcome.value().is_empty());
        assert!(
            outcome
                .warnings()
                .iter()
                .any(|warning| warning.code == WarningCode::W006)
        );
    }
}

#[test]
fn audit_sources_do_not_import_private_production_parsers_or_serializer_helpers() {
    // REQ: VAL-002, VAL-005
    let sources = [
        include_str!("../audit_support/palm.rs"),
        include_str!("../audit_support/epub.rs"),
        include_str!("epub_input.rs"),
        include_str!("format.rs"),
        include_str!("semantics.rs"),
        include_str!("batch2.rs"),
        include_str!("../audit_support/semantic.rs"),
        include_str!("dual.rs"),
        include_str!("validation.rs"),
    ];
    let forbidden = [
        "::".to_owned() + "kf8::",
        "::".to_owned() + "mobi::",
        "::".to_owned() + "container::",
        "::".to_owned() + "epub::",
        "::".to_owned() + "kindle::",
        "include!(\"../../src/".to_owned(),
        "include_str!(\"../../src/".to_owned(),
    ];
    for source in sources {
        for needle in &forbidden {
            assert!(
                !source.contains(needle.as_str()),
                "authority-first audit imports/reuses private production implementation: {needle}"
            );
        }
    }
}

#[test]
fn public_cli_exposes_one_canonical_mobi_path_without_characterization_switches() {
    // REQ: DUAL-009, API-006
    let exe = env!("CARGO_BIN_EXE_epub3-kindle");
    let output = std::process::Command::new(exe)
        .arg("--help")
        .output()
        .expect("run CLI --help");
    assert!(output.status.success(), "CLI --help must succeed");
    let help = String::from_utf8_lossy(&output.stdout);
    let removed_variant_switch = ["mobi", "variant"].join("-");
    assert!(
        !help.contains(&removed_variant_switch),
        "characterization-only MOBI variant switch must not be public"
    );
}

fn assert_dual_mobi(path: &Path) {
    let bytes = std::fs::read(path).expect("read Dual MOBI output");
    let db = PalmDb::parse(&bytes).expect("parse Dual MOBI output");
    let kf7 = db.mobi_header(0).expect("Dual MOBI KF7 header");
    assert!(kf7.version < 8, "MOBI output must begin with a KF7 section");
    let kf8_index = kf7
        .exth_u32(121)
        .expect("Dual MOBI must identify the KF8 section") as usize;
    assert_record_pointer(&db, kf8_index as u32, "EXTH 121 KF8 boundary", false).unwrap();
    assert!(
        kf8_index > 0,
        "Dual MOBI KF8 section must follow KF7 records"
    );
    assert_eq!(
        db.record(kf8_index - 1).unwrap(),
        b"BOUNDARY",
        "Dual MOBI must place BOUNDARY immediately before KF8 Record 0"
    );
    assert!(
        db.mobi_header(kf8_index).unwrap().version >= 8,
        "Dual MOBI boundary must point to a KF8 section"
    );
}

fn assert_kf8_only(path: &Path) {
    let bytes = std::fs::read(path).expect("read KF8-only output");
    let db = PalmDb::parse(&bytes).expect("parse KF8-only output");
    let header = db.mobi_header(0).expect("KF8-only MOBI header");
    assert!(header.version >= 8, "AZW3 output must begin with KF8");
    assert!(
        header.exth_u32(121).is_none(),
        "KF8-only output must not expose a Dual MOBI KF8 boundary"
    );
}

#[test]
fn public_cli_defaults_to_dual_mobi_and_preserves_explicit_outputs_and_options() {
    let exe = Path::new(env!("CARGO_BIN_EXE_epub3-kindle"));
    let tmp = TempDir::new("cli-contract");
    let input_dir = tmp.path().join("input");
    let output_dir = tmp.path().join("outputs");
    std::fs::create_dir_all(&input_dir).unwrap();
    std::fs::create_dir_all(&output_dir).unwrap();
    let input = tmp.write("input/input.epub", &epub::minimal_reflowable());

    let default = std::process::Command::new(exe)
        .arg(&input)
        .output()
        .expect("run CLI with default output");
    assert!(
        default.status.success(),
        "default CLI conversion must succeed: {}",
        String::from_utf8_lossy(&default.stderr)
    );
    let default_mobi = input.with_extension("mobi");
    assert!(
        default_mobi.is_file(),
        "default output must replace .epub with .mobi: {}",
        default_mobi.display()
    );
    assert_dual_mobi(&default_mobi);

    let explicit_mobi = output_dir.join("explicit.mobi");
    let explicit = std::process::Command::new(exe)
        .arg("-o")
        .arg(&explicit_mobi)
        .arg(&input)
        .output()
        .expect("run CLI with explicit MOBI output");
    assert!(
        explicit.status.success(),
        "explicit MOBI conversion must succeed: {}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    assert!(
        explicit_mobi.is_file(),
        "explicit full MOBI path must be honored"
    );
    assert_dual_mobi(&explicit_mobi);

    let explicit_azw3 = output_dir.join("explicit.azw3");
    let azw3 = std::process::Command::new(exe)
        .arg(&input)
        .arg("-o")
        .arg(&explicit_azw3)
        .output()
        .expect("run CLI with explicit AZW3 output");
    assert!(
        azw3.status.success(),
        "explicit AZW3 conversion must succeed: {}",
        String::from_utf8_lossy(&azw3.stderr)
    );
    assert!(
        explicit_azw3.is_file(),
        "explicit full AZW3 path must be honored"
    );
    assert_kf8_only(&explicit_azw3);

    let options_output = output_dir.join("options.azw3");
    let options = std::process::Command::new(exe)
        .args(["-c0", "-verbose", "-dont_append_source", "-donotaddsource"])
        .arg("-o")
        .arg(&options_output)
        .arg(&input)
        .output()
        .expect("run CLI with preserved KindleGen-compatible options");
    assert!(
        options.status.success(),
        "preserved CLI options must succeed: {}",
        String::from_utf8_lossy(&options.stderr)
    );
    assert!(
        options_output.is_file(),
        "option conversion must write output"
    );
    let output_path = options_output.to_string_lossy();
    assert!(
        String::from_utf8_lossy(&options.stderr).contains(&*output_path),
        "-verbose must report the full output path"
    );
    assert_kf8_only(&options_output);

    let c1_output = output_dir.join("c1.mobi");
    let c1 = std::process::Command::new(exe)
        .args(["-c1", "-o"])
        .arg(&c1_output)
        .arg(&input)
        .output()
        .expect("run CLI with -c1");
    assert!(c1.status.success(), "-c1 must remain accepted");
    assert_dual_mobi(&c1_output);

    let c2 = std::process::Command::new(exe)
        .arg("-c2")
        .arg(&input)
        .output()
        .expect("run CLI with unsupported -c2");
    assert_eq!(
        c2.status.code(),
        Some(2),
        "-c2 must remain an explicit error"
    );
    assert!(
        String::from_utf8_lossy(&c2.stderr).contains("HUFF-CDIC compression is not supported"),
        "-c2 error must explain the unsupported compression"
    );

    let combined = std::process::Command::new(exe)
        .args(["-c0", "-c1"])
        .arg(&input)
        .output()
        .expect("run CLI with conflicting compression options");
    assert_eq!(
        combined.status.code(),
        Some(2),
        "-c0 and -c1 must remain mutually exclusive"
    );
}

#[test]
fn public_cli_preserves_help_and_version_aliases() {
    let exe = Path::new(env!("CARGO_BIN_EXE_epub3-kindle"));
    for flag in ["-h", "--help"] {
        let output = std::process::Command::new(exe)
            .arg(flag)
            .output()
            .expect("run CLI help alias");
        assert!(output.status.success(), "{flag} must succeed");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Usage: epub3-kindle"),
            "{flag} must print usage"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("input.epub -> input.mobi"),
            "{flag} must document the default .mobi output"
        );
    }
    for flag in ["-V", "--version"] {
        let output = std::process::Command::new(exe)
            .arg(flag)
            .output()
            .expect("run CLI version alias");
        assert!(output.status.success(), "{flag} must succeed");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            env!("CARGO_PKG_VERSION"),
            "{flag} must print the package version"
        );
    }
}

#[test]
fn public_cli_distinguishes_clean_warning_and_failure_exit_statuses() {
    let exe = Path::new(env!("CARGO_BIN_EXE_epub3-kindle"));
    let tmp = TempDir::new("cli-warning-status");

    let clean_input = tmp.write("clean.epub", &epub::minimal_reflowable());
    let clean_output = tmp.path().join("clean.azw3");
    let clean = std::process::Command::new(exe)
        .arg(&clean_input)
        .arg("-o")
        .arg(&clean_output)
        .output()
        .expect("run clean CLI conversion");
    assert_eq!(clean.status.code(), Some(0));
    assert!(clean_output.is_file(), "clean conversion must write output");
    assert!(!String::from_utf8_lossy(&clean.stderr).contains("warning["));

    let warning_input = tmp.write("warning.epub", &epub::scripting());
    let warning_output = tmp.path().join("warning.azw3");
    let warning = std::process::Command::new(exe)
        .arg(&warning_input)
        .arg("-o")
        .arg(&warning_output)
        .output()
        .expect("run warning CLI conversion");
    assert_eq!(warning.status.code(), Some(1));
    assert!(
        warning_output.is_file(),
        "warning-success conversion must write output"
    );
    assert!(String::from_utf8_lossy(&warning.stderr).contains("warning[W001]:"));

    let failure_input = tmp.write("failure.epub", &epub::malformed_xhtml());
    let failure_output = tmp.path().join("failure.azw3");
    let failure = std::process::Command::new(exe)
        .arg(&failure_input)
        .arg("-o")
        .arg(&failure_output)
        .output()
        .expect("run failing CLI conversion");
    assert_eq!(failure.status.code(), Some(2));
    assert!(
        !failure_output.exists(),
        "failed conversion must not leave an output artifact"
    );
    assert!(String::from_utf8_lossy(&failure.stderr).contains("error:"));
}
