# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
