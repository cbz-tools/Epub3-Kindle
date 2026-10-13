use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use epub3_kindle::{ConvertOptions, convert_file};

use crate::audit_support::palm::PalmDb;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let root = std::env::temp_dir();
        for _ in 0..100 {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "epub3-kindle-file-paths-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory {}: {error}", path.display()),
            }
        }
        panic!(
            "could not allocate a unique test directory under {}",
            root.display()
        );
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn assert_valid_output(input: &Path, output: &Path) -> Vec<u8> {
    convert_file(input, output, &ConvertOptions::default())
        .unwrap_or_else(|error| panic!("convert {}: {error}", output.display()));
    let bytes = fs::read(output).expect("read converted output");
    assert!(!bytes.is_empty(), "converted output must not be empty");
    PalmDb::parse(&bytes).expect("converted output must be a readable PalmDB");
    bytes
}

fn assert_no_temporary_outputs(directory: &Path) {
    for entry in fs::read_dir(directory).expect("read output directory") {
        let entry = entry.expect("read output directory entry");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        assert!(
            !(name.starts_with(".epub3k-") && name.ends_with(".tmp")),
            "temporary output remains: {}",
            entry.path().display()
        );
    }
}

#[test]
fn file_conversion_handles_long_names_paths_and_atomic_replacement() {
    let temp = TestDirectory::new();
    let mut deep_directory = temp.path().to_path_buf();
    for level in 0..4 {
        deep_directory.push(format!("level-{level}-{}", "x".repeat(55)));
        fs::create_dir(&deep_directory).expect("create deep test directory");
    }

    let input = deep_directory.join("source-日本語.epub");
    fs::write(&input, crate::audit_support::epub::minimal_reflowable()).expect("write test EPUB");
    assert!(
        input.to_string_lossy().encode_utf16().count() > 260,
        "input EPUB path must exceed the traditional Windows path limit"
    );

    let regular = temp.path().join("book.mobi");
    assert_valid_output(&input, &regular);

    let japanese = temp.path().join("日本語の本.mobi");
    assert_valid_output(&input, &japanese);

    // Leave the destination below the Windows component limit; the old
    // filename-based temporary suffix would have pushed it past that limit.
    let long_filename = format!("{}.mobi", "a".repeat(245));
    assert_eq!(long_filename.encode_utf16().count(), 250);
    let long_name_output = temp.path().join(long_filename);
    assert_valid_output(&input, &long_name_output);

    let replacement = temp.path().join("replace-existing.mobi");
    fs::write(&replacement, b"old output").expect("create existing output");
    let replacement_bytes = assert_valid_output(&input, &replacement);
    assert_ne!(replacement_bytes, b"old output");

    let deep_mobi = deep_directory.join("deep-output.mobi");
    assert!(
        deep_mobi.to_string_lossy().encode_utf16().count() > 260,
        "MOBI output path must exceed the traditional Windows path limit"
    );
    assert_valid_output(&input, &deep_mobi);
    let deep_azw3 = deep_directory.join("deep-output.azw3");
    assert_valid_output(&input, &deep_azw3);

    let preserved = temp.path().join("preserved.mobi");
    fs::write(&preserved, b"keep existing output").expect("create preserved output");
    let invalid_input = temp.path().join("invalid.epub");
    fs::write(&invalid_input, b"not an EPUB").expect("create invalid EPUB");
    assert!(convert_file(&invalid_input, &preserved, &ConvertOptions::default()).is_err());
    assert_eq!(
        fs::read(&preserved).expect("read preserved output"),
        b"keep existing output"
    );

    let blocked_output = deep_directory.join("blocked.mobi");
    fs::create_dir(&blocked_output).expect("create replacement failure target");
    let error = convert_file(&input, &blocked_output, &ConvertOptions::default())
        .expect_err("replacing a directory with an output file must fail");
    assert!(
        error.to_string().contains("replacing output"),
        "replacement failure must identify its stage and path: {error}"
    );

    assert_no_temporary_outputs(temp.path());
    assert_no_temporary_outputs(&deep_directory);
    assert!(blocked_output.is_dir());
}
