use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::warnings::{ConversionOutcome, WarningCollector};
use crate::{Compression, ConvertOptions, Error, Result, epub, kf8, kindle, mobi};

const SUPPORTED_OUTPUT_ERROR: &str =
    "unsupported output extension; supported output extensions are .azw3 and .mobi";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Converts an EPUB file to AZW3 or Dual MOBI, replacing the destination atomically.
///
/// The output extension selects the format: `.azw3` produces KF8-only AZW3,
/// while `.mobi` produces a Dual MOBI with a minimal KF7 compatibility section
/// and the canonical KF8 reading rendition. A same-directory temporary file is
/// replaced into the destination only after serialization succeeds.
///
/// # Errors
///
/// Returns an error for an unsupported input or output extension, input or
/// output I/O failure, unsupported or invalid EPUB content, or KF8/MOBI
/// construction and serialization failure.
pub fn convert_file(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: &ConvertOptions,
) -> Result<()> {
    convert_file_with_warnings(input, output, options).map(|_| ())
}

/// Converts an EPUB file and returns any warnings collected during conversion.
///
/// The output extension selects the format: `.azw3` produces KF8-only AZW3,
/// while `.mobi` produces a Dual MOBI with a minimal KF7 compatibility section
/// and the canonical KF8 reading rendition. Inspect warnings with
/// [`ConversionOutcome::warnings`]; the outcome value is `()` because output is
/// written to the destination path.
///
/// # Errors
///
/// Returns an error for an unsupported input or output extension, input or
/// output I/O failure, unsupported or invalid EPUB content, or KF8/MOBI
/// construction and serialization failure.
pub fn convert_file_with_warnings(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: &ConvertOptions,
) -> Result<ConversionOutcome<()>> {
    let input = input.as_ref();
    let output = output.as_ref();
    let mut warnings = WarningCollector::new();
    let output_format = output_format(output)?;
    if input
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("epub"))
    {
        return Err(Error::UnsupportedInput(
            "only EPUB input is supported".to_owned(),
        ));
    }
    let input_bytes = std::fs::read(input).map_err(|source| Error::Io {
        path: input.display().to_string(),
        source,
    })?;
    let book = epub::parse_epub_with_warnings(&input_bytes, &mut warnings)?;
    // parse_epub materializes all data needed by Book, so the input ZIP buffer
    // no longer needs to remain live during normalization and serialization.
    drop(input_bytes);
    let kindle_book = kindle::normalize(book);
    let compression = match options.compression {
        Compression::None => kf8::TextCompression::None,
        Compression::PalmDoc => kf8::TextCompression::PalmDoc,
    };
    let kf8_book = kf8::build(kindle_book, compression, &mut warnings)
        .map_err(|error| Error::Kf8Build(error.to_string()))?;

    let (temporary, file) = create_temporary_file(output)?;
    let write_result = (|| {
        let mut writer = BufWriter::new(file);
        match output_format {
            OutputFormat::Kf8 => kf8_book
                .serialize_to_writer(&mut writer, &temporary.display().to_string())
                .map_err(|error| map_serialization_error(error, &temporary)),
            OutputFormat::Mobi => {
                mobi::serialize_to_writer(kf8_book, &mut writer, &temporary.display().to_string())
                    .map_err(|error| map_serialization_error(error, &temporary))
            }
        }?;
        writer.flush().map_err(|source| Error::Io {
            path: temporary.display().to_string(),
            source,
        })?;
        writer.get_ref().sync_all().map_err(|source| Error::Io {
            path: temporary.display().to_string(),
            source,
        })?;
        Ok::<(), Error>(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    if let Err(source) = replace_file(&temporary, output) {
        let _ = std::fs::remove_file(&temporary);
        return Err(Error::Io {
            path: output.display().to_string(),
            source,
        });
    }
    Ok(ConversionOutcome::from_collector((), warnings))
}

fn map_serialization_error(error: Error, temporary: &Path) -> Error {
    match error {
        Error::Io { source, .. } => Error::Io {
            path: temporary.display().to_string(),
            source,
        },
        error => Error::Container(error.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Kf8,
    Mobi,
}

fn output_format(output: &Path) -> Result<OutputFormat> {
    match output.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("azw3") => Ok(OutputFormat::Kf8),
        Some(extension) if extension.eq_ignore_ascii_case("mobi") => Ok(OutputFormat::Mobi),
        _ => Err(Error::Output(SUPPORTED_OUTPUT_ERROR.to_owned())),
    }
}

fn create_temporary_file(output: &Path) -> Result<(PathBuf, std::fs::File)> {
    let directory = output.parent().unwrap_or_else(|| Path::new("."));
    let filename = output
        .file_name()
        .ok_or_else(|| Error::Output("output path must name a file".to_owned()))?
        .to_string_lossy();
    let process_id = std::process::id();
    for _ in 0..100 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".{filename}.{process_id}.{sequence}.tmp"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(Error::Io {
                    path: temporary.display().to_string(),
                    source,
                });
            }
        }
    }
    Err(Error::Io {
        path: directory.display().to_string(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary output name collision",
        ),
    })
}

fn replace_file(temporary: &Path, output: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
        unsafe extern "system" {
            fn MoveFileExW(
                existing_file_name: *const u16,
                new_file_name: *const u16,
                flags: u32,
            ) -> i32;
        }
        let existing = temporary
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = output
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: both UTF-16 buffers are NUL-terminated and remain alive for
        // the duration of the operating-system call.
        if unsafe {
            MoveFileExW(
                existing.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(temporary, output)
    }
}
