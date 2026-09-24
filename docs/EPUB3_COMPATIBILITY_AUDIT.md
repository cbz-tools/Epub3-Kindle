# EPUB3 Compatibility Audit

> Subordinate audit of `docs/CONVERSION_AUDIT.md`. This document audits Epub3-Kindle compatibility against the IDPF EPUB 3 sample corpus. Production source is the system under test, not a requirement authority.

## 1. Audit relationship

Parent audit:

```text
docs/CONVERSION_AUDIT.md
```

Reference corpus:

```text
https://github.com/IDPF/epub3-samples
```

Target corpus:

```text
30/
```

The `EPC-*` identifiers are subordinate compatibility requirements. They do not replace or redefine product-level requirements in `CONVERSION_AUDIT.md`.

Canonical direction:

```text
Parent product contract / EPUB authority / Kindle target evidence
        ↓
EPUB3 compatibility requirement
        ↓
IDPF sample corpus observation
        ↓
Independent integration E2E
        ↓
PASS / FAIL / PENDING / GAP / N/A
```

KindleGen same-input output is compatibility evidence, not a byte-for-byte implementation target.

Unsupported or partially supported semantics follow the parent audit handling order:

```text
PRESERVE
    ↓
TRANSPORT
    ↓
FALLBACK
    ↓
DROP-FEATURE
    ↓
DEGRADE
    ↓
REJECT
```

A whole-publication rejection is appropriate only when a safe, meaningful Kindle result cannot be established under the parent audit contract.

Legacy character-encoding compatibility is evidence-driven. Acceptance of one corpus sample does not establish language-wide or encoding-wide autodetection.

## 2. Corpus scope

The audit covers the IDPF EPUB 3 sample corpus under `30/`.


Compatibility requirements may also be frozen from external same-input KindleGen characterization when they define an input-acceptance boundary not represented by the official `30/` corpus. Such characterization requirements do not change the corpus totals in this section unless a corresponding corpus item exists; they remain `PENDING` until independent integration E2E fixes the behavior.


| Corpus item | Result |
|---|---:|
| Sample directories | 45 |
| Officially packable EPUBs | 44 |
| Official corpus pack exception | 1 |
| Converter SUCCESS | 17 |
| Converter WARNING-SUCCESS | 23 |
| Converter EXPECTED-REJECT | 4 |
| Converter UNEXPECTED-FAILURE | 0 |
| **Total** | **45** |

`georgia-cfi` is the single official corpus pack exception. Its compatibility behavior is covered independently by EPC-012 E2E evidence and is not counted as a converter run in the corpus totals.

All generated converter artifacts in the corpus audit were non-empty and passed PalmDB/MOBI structural inspection. The four expected converter rejections are the EPC-008 fixed-layout boundary cases.

## 3. Compatibility requirement table

| ID | Group | Level | Requirement | Corpus sample(s) | Handling | Warning / Exit | Evidence linkage | Status | Result |
|---|---|---|---|---|---|---|---|---|---|
| `EPC-001` | `PATH-NAV` | COMPAT | Internal document, navigation, and fragment paths must resolve to the intended generated section without repeated source-relative resolution. | `cc-shared-culture`; `epub30-spec`; `GhV-oeb-page`; `kusamakura-japanese-vertical-writing`; `kusamakura-preview`; `kusamakura-preview-embedded`; `linear-algebra`; `regime-anticancer-arabic`; `WCAG`; `haruko-ahl`; `haruko-html-jpeg` | `TRANSPORT` | none / 0 attributable to EPC-001 | `epc001_canonical_internal_and_navigation_targets_preserve_fragments` | **PASS** | Canonical document and fragment destinations remain resolvable. |
| `EPC-002` | `EPUB-PACKAGE` | COMPAT | OPF package, metadata, manifest, and spine elements must be recognized by namespace URI and local name independent of namespace prefix spelling. | `jlreq-in-english`; `jlreq-in-japanese` | `TRANSPORT` | none / 0 attributable to EPC-002 | `epc002::epc002_prefixed_opf_namespace_parses_manifest_spine_and_body` | **PASS** | Prefixed OPF package structures convert without package-namespace rejection. |
| `EPC-003` | `EPUB-FONT` | COMPAT | IDPF-obfuscated WOFF resources using the legacy `application/font-woff` media type must remain valid font resources without type confusion or unnecessary transcoding. | `wasteland-woff-obf` | `TRANSPORT` | none / 0 | `epc003::epc003_legacy_woff_media_type_preserves_idpf_obfuscated_font_resource` | **PASS** | Legacy and current WOFF media-type spellings are accepted in the font-obfuscation path. |
| `EPC-004` | `RICH-CONTENT` | COMPAT | PLS pronunciation lexicons may be omitted from active Kindle resources when readable XHTML remains; pronunciation-specific semantics may degrade without whole-publication failure. | `accessible_epub_3`; `georgia-pls-ssml` | `DROP-FEATURE / DEGRADE` | `W002 / 1` | `epc004::epc004_pronunciation_lexicon_survives_without_becoming_a_kindle_resource` | **PASS** | Readable publication content survives while unsupported PLS semantics are omitted explicitly. |
| `EPC-005` | `EPUB-PRESENTATION` | COMPAT | Adobe XPGT page-template payloads may be dropped when they cannot be projected to Kindle, while surrounding readable content and ordinary resources remain usable. | `indexing-for-eds-and-auths-3f`; `indexing-for-eds-and-auths-3md` | `DROP-FEATURE / DEGRADE` | `W004 / 1` | `epc005_xpgt_page_template_is_dropped_with_warning_and_body_preserved` | **PASS** | Unsupported XPGT presentation semantics do not cause publication failure. |
| `EPC-006` | `STRUCTURED-SEMANTICS` | COMPAT | Unsupported MathML may be reduced to meaningful ordered descendant text when a readable representation remains. | `linear-algebra` | `DEGRADE` | `W005 / 1` | `epc006_mathml_reduces_to_ordered_readable_descendant_text` | **PASS** | Active MathML structure is removed while readable formula text remains in document order. |
| `EPC-007` | `EPUB-LAYOUT` | COMPAT | Explicit positive XHTML viewport dimensions in pre-paginated content must provide Kindle fixed-layout page presentation without inferring geometry from unrelated raster/SVG intrinsic dimensions. | `page-blanche` | `TRANSPORT` | none / 0 | `epc007_explicit_xhtml_viewport_creates_fixed_layout_page_flows` | **PASS** | Explicit viewport information provides fixed-layout page presentation. |
| `EPC-008` | `EPUB-LAYOUT` | COMPAT | Direct bitmap/SVG spine content without sufficient explicit Kindle page presentation must not fabricate fixed-layout geometry from intrinsic image/SVG dimensions. | `haruko-jpeg`; `page-blanche-bitmaps-in-spine`; `sous-le-vent_svg-in-spine`; `svg-in-spine` | `REJECT` | none / 2 | EPC-008 integration E2E set | **PASS** | The four corpus cases reject at the no-page-presentation boundary. |
| `EPC-009` | `EPUB-CSS` | COMPAT | CSS contained in XHTML `<style>` CDATA wrappers must be normalized as stylesheet content without treating the CDATA delimiters as CSS syntax. | `sous-le-vent` | `TRANSPORT` | none / 0 | `epc009_inline_style_cdata_delimiters_are_removed_and_css_survives` | **PASS** | Inline stylesheet content survives after CDATA-wrapper normalization. |
| `EPC-010` | `EPUB-CSS` | COMPAT | A stylesheet that cannot be decoded safely as supported text may be omitted when readable publication content remains; valid stylesheets and XHTML must continue. No language-based legacy-encoding autodetection is implied. | `horizontally-scrollable-emakimono` | `DROP-FEATURE / DEGRADE` | `W004 / 1` | `epc010_invalid_utf8_stylesheets_are_dropped_without_losing_valid_content` | **PASS** | Invalid stylesheet bytes do not force whole-publication failure. |
| `EPC-011` | `EPUB-NAV` | COMPAT | Valid TOC and ordinary internal fragment targets, including valid body IDs, must resolve when Epub3-Kindle can represent them safely even when KindleGen rejects the same input. | `cole-voyage-of-life`; `cole-voyage-of-life-tol` | `TRANSPORT` | none / 0 attributable to EPC-011 | `epc011_valid_body_id_toc_fragments_resolve_to_generated_positions` | **PASS** | Valid fragment navigation is retained as a documented KindleGen compatibility exception. |
| `EPC-012` | `EPUB-NAV` | COMPAT | EPUB CFI page-list destinations that cannot be projected to the Kindle page-map model may be dropped without affecting ordinary navigation or readable content. | `georgia-cfi` | `DROP-FEATURE` | none / 0 attributable to EPC-012 | `epc012_cfi_page_list_is_silently_dropped_and_normal_navigation_survives` | **PASS** | CFI page-list entries are omitted while ordinary navigation remains valid. |
| `EPC-013` | `EPUB-COVER-NAV` | COMPAT | Cover navigation must resolve to the package-selected native cover resource when one exists; otherwise a valid source cover document in the spine must remain available as the navigation destination. | `indexing-for-eds-and-auths-3f`; `indexing-for-eds-and-auths-3md`; `kusamakura-japanese-vertical-writing`; `kusamakura-preview`; `kusamakura-preview-embedded` | `FALLBACK` | none / 0 attributable to EPC-013 | `epc013_suppressed_cover_navigation_uses_native_cover_destination`; `epc013_without_native_cover_resource_keeps_source_cover_destination` | **PASS** | Both native-cover and source-cover navigation paths remain valid without fabricating missing cover resources. |
| `EPC-014` | `EPUB-HYPERLINK` | COMPAT | Unresolvable root-relative web-style hyperlinks must not abort conversion when readable link text and the surrounding document remain usable. Valid EPUB-local root-relative targets must continue to resolve normally. | `horizontally-scrollable-emakimono` | `DROP-FEATURE / DEGRADE` | `W006 / 1` | `epc014_unresolvable_root_relative_web_links_degrade_without_aborting` | **PASS** | Unresolvable web-style destinations are removed while readable link text remains. |
| `EPC-015` | `EPUB-CONTAINER` | COMPAT | A readable EPUB container must not be rejected solely because the `mimetype` entry is not the first ZIP entry. When the container can still be interpreted safely, conversion must continue to the actual package/content compatibility checks. | synthetic `mimetype_not_first()` regression input | `TRANSPORT` | success or warning-success / 0 or 1; not 2 solely for entry order | `epc015_017::epc015_nonfirst_mimetype_container_converts_to_structurally_valid_output` | **PASS** | E2E independently confirms the fixture's first-entry order, then CLI conversion generates a non-empty AZW3 whose PalmDB structure and KF8-readable content pass independent inspection. |
| `EPC-016` | `EPUB-CONTAINER` | COMPAT | A readable EPUB container must not be rejected solely because the `mimetype` entry is ZIP-compressed rather than stored. When the container can still be interpreted safely, conversion must continue to the actual package/content compatibility checks. | synthetic `compressed_mimetype()` regression input | `TRANSPORT` | success or warning-success / 0 or 1; not 2 solely for compression of `mimetype` | `epc015_017::epc016_compressed_mimetype_container_converts_to_structurally_valid_output` | **PASS** | E2E independently confirms `mimetype` uses Deflate, then CLI conversion generates a non-empty AZW3 whose PalmDB structure and KF8-readable content pass independent inspection. |
| `EPC-017` | `EPUB-PACKAGE` | COMPAT | OPF `package version="2.0"` must not cause whole-publication rejection solely because of the version value. The converter must continue processing package/content structures it can interpret safely and reject only when an actual blocking incompatibility or invalid condition prevents a usable Kindle result. This requirement does not declare full EPUB 2 compatibility. | synthetic `package_version_2()` regression input | `TRANSPORT / DEGRADE` | success or warning-success / 0 or 1 when usable; 2 only for an actual blocking condition, not the version value alone | `epc015_017::epc017_opf_package_version_2_is_not_an_immediate_rejection` | **PASS** | E2E verifies the OPF declares `version="2.0"`; the CLI produces a non-empty AZW3 that passes independent PalmDB/KF8 inspection with readable content. This establishes only that the version value alone is not an entry rejection; full EPUB 2 feature support is not claimed. |

## 4. Corpus result index

This section is a compact result index for exceptional outcomes in the official `30/` corpus. It is not an execution log.

### EXPECTED-REJECT

- `haruko-jpeg` — EPC-008
- `page-blanche-bitmaps-in-spine` — EPC-008
- `sous-le-vent_svg-in-spine` — EPC-008
- `svg-in-spine` — EPC-008

### PACK-FAIL

- `georgia-cfi` — official corpus pack exception; EPC-012 is covered by isolated compatibility E2E.

### UNEXPECTED-FAILURE

- None.

All other officially packable samples are classified as SUCCESS or WARNING-SUCCESS according to the parent audit warning contract. Sample-to-requirement linkage is recorded in the compatibility requirement table above.

## 5. Evidence and closure rule

Each `EPC-*` requirement is considered closed only when:

1. the behavior is stated independently of the production implementation,
2. at least one integration E2E verifies the compatibility contract rather than only process exit status,
3. affected corpus samples behave consistently with the requirement, or, when the requirement is derived from an external characterization boundary not represented in the corpus, the fixed synthetic regression input behaves consistently with that requirement,
4. the behavior does not weaken parent audit safety or structural-validity requirements.

Corpus sample names identify where a requirement is observed. They are not requirement identities, and one sample may exercise multiple `EPC-*` requirements.

The corpus audit does not imply that every EPUB outside the referenced sample set is accepted. It demonstrates compatibility behavior for the audited IDPF sample corpus under the parent product contract.

## 6. Relationship to the parent audit

`CONVERSION_AUDIT.md` remains the product-level requirement authority.

This document is the subordinate audit for the IDPF EPUB 3 sample corpus:

```text
CONVERSION_AUDIT.md
    product-level requirements and policy
        ↓
EPUB3_COMPATIBILITY_AUDIT.md
    IDPF epub3-samples compatibility requirements and corpus evidence
```

Stable compatibility behavior is represented here with `EPC-*` identifiers and linked integration E2E evidence. Product-level policy, warning taxonomy, output format requirements, security boundaries, and general EPUB validity requirements remain defined by the parent audit.
