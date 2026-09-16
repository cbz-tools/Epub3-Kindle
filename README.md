# Epub3-Kindle

[English](README.md) | [日本語](README.ja.md)

A Rust library and CLI for converting [EPUB](https://www.w3.org/TR/epub-33/) to AZW3 or MOBI.

It supports EPUB layout, ruby, navigation, embedded fonts, CSS, and related features, with a particular focus on Japanese vertical writing.

The supported conversion scope from [EPUB 3.3](https://www.w3.org/TR/epub-33/) is also audited and validated.

## Download

Download the latest release from [Releases](https://github.com/cbz-tools/Epub3-Kindle/releases/latest).

| Platform | Package |
|---|---|
| Windows x64 | `epub3-kindle-vX.Y.Z-windows-x64.zip` |
| Linux x64 | `epub3-kindle-vX.Y.Z-linux-x64.tar.gz` |
| macOS Apple Silicon | `epub3-kindle-vX.Y.Z-macos-arm64.tar.gz` |

Extract the archive and run `epub3-kindle` directly.

### Install with Cargo

Rust 1.85 or newer is required.

```bash
cargo install epub3-kindle
```

## Quick Start

Convert an EPUB:

```bash
epub3-kindle input.epub
```

If the output path is omitted, the input extension is replaced with `.mobi`.

## Features

- AZW3 output
- MOBI output
- Japanese vertical writing
- Right-to-left page progression
- Ruby and emphasis marks
- Kindle-oriented navigation and reading order
- Embedded fonts
- Kindle-oriented CSS conversion
- PalmDOC compression
- Cover resources and in-book cover display
- EPUB 3.3 compatibility within the supported scope
- Rust library API in addition to the CLI

## Screenshots

The following photos show the same generic EPUB 3 documentation sample rendered on a physical Kindle.

### Epub3-Kindle

| Cover | Table of Contents | Vertical Text |
|---|---|---|
| [![Cover](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-01.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-01.JPEG) | [![Table of Contents](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-02.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-02.JPEG) | [![Vertical Text](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-03.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-03.JPEG) |

### KindleGen

| Cover | Table of Contents | Vertical Text |
|---|---|---|
| [![Cover](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-01.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-01.JPEG) | [![Table of Contents](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-02.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-02.JPEG) | [![Vertical Text](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-03.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-03.JPEG) |

No reader-visible differences were observed in this comparison.

## Why

KindleGen has been unavailable from Amazon for many years and is no longer a practical dependency for new tooling.

Epub3-Kindle targets current-generation Kindle devices. Devices earlier than Kindle Touch (4th Generation) are outside the supported scope, so `.mobi` output does not need to contain another complete reading rendition for older Kindle devices.

Instead, `.mobi` output contains a small compatibility section used for MOBI recognition and a newer Kindle reading rendition used for actual reading.

| Output | Internal structure |
|---|---|
| AZW3 | Newer Kindle reading content only |
| Epub3-Kindle MOBI | Minimal compatibility section + newer Kindle reading content |
| KindleGen MOBI | Older Kindle reading content + newer Kindle reading content |

Internally, the minimal compatibility section is KF7 and the actual reading rendition is KF8.

KindleGen compatibility aims for equivalent reading behavior within the supported scope. It does not guarantee support for every EPUB or every KindleGen behavior.

## Replacing KindleGen

Epub3-Kindle is designed to make existing KindleGen-based workflows easier to replace.

By renaming `epub3-kindle.exe` to `kindlegen.exe` and replacing the existing `kindlegen.exe`, it can be used by existing tools and scripts that invoke KindleGen.

However, Epub3-Kindle is not a complete replacement implementation of KindleGen. Supported inputs, options, and output formats are limited to the scope defined in this README and the audit.

## Compared with KindleGen

| | KindleGen | epub3-kindle |
|---|---|---|
| Input | EPUB, HTML/OPF, and other supported sources | EPUB |
| Output | Legacy format + KF8 Kindle format | AZW3 (KF8) or MOBI (minimal KF7 compatibility + KF8) |
| Japanese vertical writing / ruby | Supported | Supported and verified on physical Kindle hardware |
| In-book cover display | Supported | Supported and verified on physical Kindle hardware |
| Kindle library cover thumbnail | Supported | MOBI: supported / AZW3: not supported |
| [Devices earlier than Kindle Touch (4th Generation)](https://digprjsurvey.amazon.com/csad/help/node/GK33S847NN4V6Y83) | Supported | Not supported |
| Availability | No longer distributed by Amazon | Open source and actively maintained |

## Performance

The following is one illustrative real-world measurement.

Input preparation:

| Stage | Size |
|---|---:|
| Source TXT (text only, no images) | 49.58 MiB (51,992,309 bytes) |
| AozoraEpub3-generated EPUB | 17.92 MiB (18,790,298 bytes) |

The same generated EPUB was used for all three conversions.

| | KindleGen MOBI | Epub3-Kindle MOBI | Epub3-Kindle AZW3 |
|---|---:|---:|---:|
| Conversion time | 90.173 s | 2.754 s | 2.752 s |
| Output size | 83.71 MiB (87,771,662 bytes) | 47.41 MiB (49,710,628 bytes) | 47.40 MiB (49,701,730 bytes) |

Each conversion was run five times on the same EPUB. Exactly one fastest run and one slowest run were discarded, and the arithmetic mean of the remaining three runs is reported as the conversion time.

In this measurement, Epub3-Kindle MOBI was about 32.7x faster and 43.36% smaller than KindleGen MOBI, while Epub3-Kindle AZW3 was about 32.8x faster and 43.37% smaller.

Both Epub3-Kindle MOBI and AZW3 were generated with the default PalmDOC compression.

Results depend on the input, tool versions, and hardware. TXT-to-EPUB generation time is not included in the conversion time.

## CLI

```text
epub3-kindle <input.epub> [-o <output.azw3|output.mobi>] [-c0 | -c1]
    [-verbose] [-dont_append_source] [-donotaddsource]
```

`-c1` is the default and uses PalmDOC compression. `-c0` stores text records uncompressed. `-dont_append_source` and `-donotaddsource` are accepted as KindleGen-compatible no-op options. This converter never embeds the source EPUB. `-c2` (HUFF/CDIC) and unrelated KindleGen options are intentionally unsupported.

If `-o` is omitted, the input extension is replaced with `.mobi`. An explicit `.mobi` output contains a minimal KF7 compatibility section plus KF8 reading content, while an explicit `.azw3` output produces AZW3.

## Library

```rust
use epub3_kindle::{convert_bytes, Compression, ConvertOptions};

let azw3 = convert_bytes(
    &epub_bytes,
    &ConvertOptions { compression: Compression::PalmDoc },
)?;
```

The crate also provides `convert_file(input, output, options)`. The file API selects the output format from the destination extension: `.azw3` produces AZW3, while `.mobi` produces MOBI with a minimal KF7 compatibility section plus KF8 reading content. The destination is replaced from a same-directory temporary file only after serialization succeeds.

`convert_file` serializes the result to a temporary file in the same directory and atomically replaces the destination after successful serialization. It uses no global mutable conversion state, shared temporary file names, current-directory changes, or internal parallel runtime. Independent conversions can be invoked safely in parallel. As with ordinary file writing, coordinating multiple concurrent conversions to the exact same output path is the caller's responsibility.

## Limitations

Kindle devices earlier than Kindle Touch (4th Generation) are not supported. `.azw3` produces AZW3, while `.mobi` produces MOBI with a minimal KF7 compatibility section plus KF8 reading content. The `.mobi` compatibility section is not a complete legacy MOBI7 reading rendition.

In-book cover display is supported. Kindle library cover thumbnails are supported for MOBI but not for AZW3.

## Validation

Validation uses a broad compatibility corpus including generic and synthetic EPUB fixtures, external producer interoperability sources, same-input KindleGen comparisons, independent parsing, and physical Kindle verification.

The supported conversion scope from EPUB is audited. This does not claim complete EPUB 3.3 coverage or complete coverage of every KindleGen edge case.

EPUB behavior is evaluated against the [EPUB 3.3 specification](https://www.w3.org/TR/epub-33/).

Kindle-specific conversion and compatibility are evaluated using the [Amazon Kindle Publishing Guidelines](https://kindlegen.s3.amazonaws.com/AmazonKindlePublishingGuidelines.pdf), same-input KindleGen output, and physical Kindle hardware.

Reader-visible behavior including reading order, navigation, text fidelity, Japanese vertical writing, and ruby has also been verified on physical Kindle devices.

For the complete supported scope, validation methodology, compatibility boundaries, E2E results, and device verification, see the [EPUB to KF8/AZW3 Conversion Audit](docs/CONVERSION_AUDIT.md).

The audit is the formal technical record of the converter's validated behavior.

## Acknowledgements

The KF8/MOBI format analysis and implementation in this project benefited greatly from the following open-source projects and technical references:

- [MobileRead Wiki — MOBI](https://wiki.mobileread.com/wiki/MOBI) — an important reference for understanding PalmDOC, MOBI/EXTH headers, indexes, record structures, and related details.
- [Kindling](https://github.com/ciscoriordan/kindling) — a Rust Kindle toolkit and a useful reference implementation for understanding KF8/MOBI structures.
- [calibre](https://github.com/kovidgoyal/calibre) — its Kindle/MOBI implementation has been an invaluable reference for format behavior and implementation details.
- [KindleUnpack](https://github.com/kevinhendricks/KindleUnpack) — extensively used to inspect and analyze KindleGen-generated files.

Many thanks to the authors, contributors, and communities who made this work and information publicly available.

This project is an independent implementation and is not affiliated with Amazon, MobileRead, Kindling, calibre, or KindleUnpack.

## License

Licensed under the MIT License.

See [LICENSE](LICENSE) for details.

See [THIRDPARTY_LICENSES.md](THIRDPARTY_LICENSES.md) for the third-party dependency overview.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release history.