# Requirement-derived regression traceability

This manifest is an audit index, not an authority by itself.  Each row names
the external/product contract and the executable regression that exercises it.

| observed defect | authority / product contract | requirement | regression E2E |
|---|---|---|---|
| synthetic fallback layout CSS leaked into reflowable output | source-derived layout contract; no invented book-wide layout rule | `LAYOUT-008`, `SEM-013` | `batch2::batch2_layout_cover_font_and_file_api_artifacts_are_independently_projected` |
| KF7 EXTH 116 was stale or outside the stub | Dual KF7 Start Reading must target its own stub text | `DUAL-007`, `FMT-EXTH-003` | `batch3::batch3_dual_common_invariants_run_identically_across_all_structural_inputs` |
| Dual/KF8 pointers used the wrong section base | section-local pointers resolve against their own MOBI section | `DUAL-005`, `FMT-MOBI-004` | `batch3::batch3_palmdb_and_mobi_pointer_inventory_use_actual_record_geometry` |
| cover normalization/resource addressing lost logical identity | source cover image remains a valid target cover resource | `FMT-COVER-001`, `FMT-RES-001`, `AMZ-COVER-001` | `batch2::batch2_layout_cover_font_and_file_api_artifacts_are_independently_projected` |
| decimal 30 MB boundary was off by one | individual XHTML is strictly below the declared decimal limit | `AMZ-QA-001`, `SEC-005` | `validation::amazon_individual_html_size_boundary_warns_and_converts_at_and_above_limit` |
| 300 HTML document boundary counted non-HTML resources | HTML/XHTML content-document limit only | `AMZ-QA-002`, `SEC-006` | `validation::amazon_html_document_count_boundary_warns_and_converts_at_and_above_limit` |
| publication language metadata was replaced by a default | Package Document language is the source metadata contract | `PKG-003`, `SEM-016` | `batch2::batch2_common_projection_closes_metadata_text_sections_ids_links_resources_and_a11y` |
| EXTH 106 conflated publication date and last-modified metadata | EPUB `dc:date` and `dcterms:modified` are independent semantic fields; EXTH 106 is publication date | `FMT-EXTH-009` | `batch2::batch2_publication_date_and_modified_remain_independent_through_exth_projection` |
| inline image data remained literal in Kindle output or unsafe inline data was exposed as a resource target | bounded supported `data:image/...` payloads use the existing Kindle resource/index/embed flow; inline-style CSS is resolved from its owning XHTML document while external CSS retains its stylesheet base; invalid or unsafe data degrades without publication failure or readable-content loss | `RES-009`, `FMT-RES-001` | `batch2::batch2_data_uri_css_image_is_materialized_and_resolvable; batch2::batch2_inline_style_data_images_use_their_owning_document_base; batch2::batch2_data_uri_xhtml_image_is_materialized_and_resolvable; batch2::batch2_negative_data_uri_images_are_safely_degraded_in_css_and_xhtml` |
| namespace-qualified CSS compatibility vector regressed | KindleGen characterization preserves the namespace declaration and qualified selector | `CHAR-CSS-NAMESPACE-001` | `batch2::batch2_css_namespace_declaration_and_qualified_selector_are_retained` |
