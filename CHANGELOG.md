# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-10-17

### Added

- Added in-memory MOBI generation to the Rust library Memory API.

### Changed

- Organized the Rust library API into Memory, File, and Common modules.
- Increased the EPUB extraction limit from 1 GiB to 4 GiB, allowing larger
  image-heavy EPUB files to be converted.
- Raised the non-comic image optimization target from 128 KiB to 512 KiB,
  making it easier to retain high-resolution images. The policy applies to
  MOBI and AZW3; comic image behavior is unchanged.
- Applied EXIF Orientation when re-encoding oversized JPEG and PNG images.

### Removed

- Breaking change for Rust library users only: removed the old top-level
  conversion functions. The CLI interface and behavior are unchanged; see the
  [v0.4.x to v0.5.0 migration guide on docs.rs](https://docs.rs/epub3-kindle/0.5.0/epub3_kindle/#migration-guide).

## [0.4.3] - 2026-10-13

### Fixed

- Fixed conversion failures for EPUBs with long file names and Windows paths
  longer than 260 characters.

## [0.4.2] - 2026-10-02

### Changed

- Improved compatibility with valid parent-relative internal links in EPUB content documents.

## [0.4.1] - 2026-09-24

### Changed

- Improved compatibility with EPUBs whose `mimetype` entry is not first in the
  ZIP archive or is compressed.
- OPF package version `2.0` alone no longer causes immediate rejection when
  the publication can still be processed safely.

## [0.4.0] - 2026-09-18

### Changed

- Kept warning retrieval available through `ConversionOutcome` while hiding the
  warning collection type and representation, and clarified library guidance on
  conversion failures and warning use.
- Refactored internal EPUB/KF8 structure and improved conversion efficiency for
  navigation-heavy and fixed-layout books without changing generated Kindle output.

## [0.3.0] - 2026-09-16

### Added

- Added `.mobi` output support for Kindle devices.
- Added Dual MOBI output while keeping `.azw3` available for KF8-only output.

### Changed

- Renamed the project to `Epub3-Kindle`.
- Renamed the CLI to `epub3-kindle`.
- Expanded support from AozoraEpub3-generated EPUBs to KindleGen-compatible EPUB 3 files.
- Improved compatibility for vertical writing, layout, navigation, fonts, covers, and other EPUB features.
- Strengthened validation of generated AZW3 and MOBI files.

### Fixed

- Fixed Dual MOBI compatibility metadata needed for modern Kindle recognition.
- Fixed Start Reading metadata in the KF7 compatibility section.
- Removed synthetic fallback layout CSS that could alter the EPUB's original
  layout semantics.

## [0.2.0] - 2026-09-11

### Added

- Broad EPUB 3.3 semantic coverage for KF8/AZW3 conversion, including improved
  metadata, navigation, CSS, embedded-font, and layout handling.
- Expanded KF8 RESC projection for EPUB rendition orientation, spread, flow, and
  alignment semantics.
- Safer handling of unsupported EPUB features.
- A unified technical audit documenting conversion coverage, validation methods,
  and compatibility boundaries.

## [0.1.0] - 2026-09-07

### Added

- Initial release.
