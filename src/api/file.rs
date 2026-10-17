use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::conversion::build_kf8;
use super::warnings::ConversionOutcome;
use crate::{ConvertOptions, Error, OutputFormat, Result, mobi};

const SUPPORTED_OUTPUT_ERROR: &str =
    "unsupported output extension; supported output extensions are .azw3 and .mobi";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Converts an EPUB file to AZW3 or Dual MOBI, replacing the destination only
/// after successful serialization.
///
/// The input extension must be `.epub`, matched without regard to ASCII letter
/// case. The output extension selects the format and is also matched without
/// regard to ASCII letter case: `.azw3` produces KF8-only AZW3, while `.mobi`
/// produces a Dual MOBI with a minimal KF7 compatibility section and the
/// canonical KF8 reading rendition. `options.format` is ignored; compression
/// comes from `options.compression`.
///
/// The output is written to a same-directory temporary file, flushed and synced,
/// and moved over the destination only after serialization succeeds. Calls to
/// distinct output paths can run concurrently. Callers must coordinate
/// concurrent writes to the same output path.
///
/// # Errors
///
/// Returns [`Error::UnsupportedInput`](crate::common::Error::UnsupportedInput)
/// or [`Error::Output`](crate::common::Error::Output) for unsupported
/// extensions; [`Error::Io`](crate::common::Error::Io) for input, temporary
/// output, sync, or replacement failures; or an error for invalid/unsupported
/// EPUB content, KF8/MOBI construction, or serialization.
pub fn convert(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: &ConvertOptions,
) -> Result<()> {
    convert_with_warnings(input, output, options).map(|_| ())
}

/// Converts an EPUB file and returns any warnings collected during conversion.
///
/// The input extension must be `.epub`. The case-insensitive output extension
/// selects KF8-only AZW3 (`.azw3`) or Dual MOBI (`.mobi`); `options.format` is
/// ignored and `options.compression` is used. The same-directory temporary is
/// moved over the destination only after serialization succeeds. Inspect
/// warnings with [`ConversionOutcome::warnings`](crate::common::ConversionOutcome::warnings);
/// the outcome value is `()` because output is written to the destination.
///
/// # Errors
///
/// Returns an unsupported-extension, I/O, invalid/unsupported EPUB, KF8/MOBI
/// construction, or serialization error. The output path is not replaced when
/// conversion or serialization fails before the replacement step.
pub fn convert_with_warnings(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: &ConvertOptions,
) -> Result<ConversionOutcome<()>> {
    let input = input.as_ref();
    let output = output.as_ref();
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
        path: format!("reading input EPUB {}", input.display()),
        source,
    })?;
    let (kf8_book, warnings) = build_kf8(&input_bytes, options.compression)?;
    // Parsing materializes all data needed by the Kindle representation.
    drop(input_bytes);

    let (temporary, file) = create_temporary_file(output)?;
    let write_result = (|| {
        let mut writer = BufWriter::new(file);
        match output_format {
            OutputFormat::Azw3 => kf8_book
                .serialize_to_writer(&mut writer, &temporary.display().to_string())
                .map_err(|error| map_serialization_error(error, &temporary)),
            OutputFormat::Mobi => {
                mobi::serialize_to_writer(kf8_book, &mut writer, &temporary.display().to_string())
                    .map_err(|error| map_serialization_error(error, &temporary))
            }
        }?;
        writer.flush().map_err(|source| Error::Io {
            path: format!("writing temporary output {}", temporary.display()),
            source,
        })?;
        writer.get_ref().sync_all().map_err(|source| Error::Io {
            path: format!("syncing temporary output {}", temporary.display()),
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
            path: format!(
                "replacing output {} with temporary file {}",
                output.display(),
                temporary.display()
            ),
            source,
        });
    }
    Ok(ConversionOutcome::from_collector((), warnings))
}

fn map_serialization_error(error: Error, temporary: &Path) -> Error {
    match error {
        Error::Io { source, .. } => Error::Io {
            path: format!("writing temporary output {}", temporary.display()),
            source,
        },
        error => Error::Container(format!(
            "writing temporary output {}: {error}",
            temporary.display()
        )),
    }
}

fn output_format(output: &Path) -> Result<OutputFormat> {
    match output.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("azw3") => Ok(OutputFormat::Azw3),
        Some(extension) if extension.eq_ignore_ascii_case("mobi") => Ok(OutputFormat::Mobi),
        _ => Err(Error::Output(SUPPORTED_OUTPUT_ERROR.to_owned())),
    }
}

fn create_temporary_file(output: &Path) -> Result<(PathBuf, std::fs::File)> {
    let directory = output.parent().unwrap_or_else(|| Path::new("."));
    output
        .file_name()
        .ok_or_else(|| Error::Output("output path must name a file".to_owned()))?;
    let process_id = std::process::id();
    for _ in 0..100 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".epub3k-{process_id}-{sequence}.tmp"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(Error::Io {
                    path: format!("creating temporary output {}", temporary.display()),
                    source,
                });
            }
        }
    }
    Err(Error::Io {
        path: format!("creating temporary output in {}", directory.display()),
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
        let existing_path = canonical_windows_file_path(temporary)?;
        let destination_path = canonical_windows_file_path(output)?;
        let existing = existing_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = destination_path
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

#[cfg(windows)]
fn canonical_windows_file_path(path: &Path) -> io::Result<PathBuf> {
    let filename = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path must name a file"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // canonicalize returns a Windows extended-length path, including for UNC
    // paths, and resolves relative parents before they reach MoveFileExW.
    Ok(std::fs::canonicalize(parent)?.join(filename))
}
