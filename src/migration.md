## Migration guide

This guide covers the Rust library API change from v0.4.x to v0.5.0. It does
not apply to CLI users: the CLI options, output selection, and conversion
behavior are unchanged.

### Function mapping

| v0.4.x | v0.5.0 |
| --- | --- |
| `convert_bytes(input, options)` | `memory::convert(input, options)` |
| `convert_bytes_with_warnings(input, options)` | `memory::convert_with_warnings(input, options)` |
| `convert_file(input, output, options)` | `file::convert(input, output, options)` |
| `convert_file_with_warnings(input, output, options)` | `file::convert_with_warnings(input, output, options)` |

The old conversion functions were crate-root functions. In v0.5.0, the
functions live in `epub3_kindle::memory` and `epub3_kindle::file`. Shared types
are organized under `epub3_kindle::common`. Existing crate-root re-exports of
those types remain available and refer to the same definitions.

### Options and format selection

`ConvertOptions` gains a `format` field for the Memory API. Its default is
`OutputFormat::Azw3`; compression still defaults to `Compression::PalmDoc`.
Memory conversions select AZW3 or MOBI with `ConvertOptions.format`. File
conversions continue to select the output representation from the `.azw3` or
`.mobi` destination extension, case-insensitively, and ignore `format`.

### Before: v0.4.x

This example is written for v0.4.x and is not compiled against v0.5.0:

```text
use epub3_kindle::{convert_bytes, convert_file, ConvertOptions};

let options = ConvertOptions::default();
let azw3 = convert_bytes(epub_bytes, &options)?;
convert_file("book.epub", "book.mobi", &options)?;
```

The byte API produced AZW3 only. File output selected AZW3 or Dual MOBI from
the destination extension.

### After: v0.5.0

```rust
use epub3_kindle::{common::{ConvertOptions, OutputFormat}, file, memory};

# fn convert(epub_bytes: &[u8]) -> epub3_kindle::Result<(Vec<u8>, Vec<u8>)> {
let azw3_options = ConvertOptions {
    format: OutputFormat::Azw3,
    ..ConvertOptions::default()
};
let azw3 = memory::convert(epub_bytes, &azw3_options)?;

let mobi_options = ConvertOptions {
    format: OutputFormat::Mobi,
    ..azw3_options
};
let mobi = memory::convert(epub_bytes, &mobi_options)?;

file::convert("book.epub", "book.azw3", &azw3_options)?;
file::convert("book.epub", "book.mobi", &mobi_options)?;
# Ok((azw3, mobi))
# }
```

For warning details, use `memory::convert_with_warnings` or
`file::convert_with_warnings`. The former returns
`ConversionOutcome<Vec<u8>>`; the latter returns `ConversionOutcome<()>` because
the converted bytes are written to the destination path. A successful outcome
may contain non-fatal warnings.
