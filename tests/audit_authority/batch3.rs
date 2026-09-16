use std::collections::BTreeSet;

use epub3_kindle::{Compression, ConvertOptions, convert_bytes, convert_file};

use crate::audit_support::epub;
use crate::audit_support::palm::{
    PalmDb, decode_indx_rows, decode_vwi, encode_vwi_audit, parse_aux_record, parse_fdst,
    parse_indx, reconstruct_text_raw, text_record_indices,
};
use crate::audit_support::semantic::{
    NavItem, SourceModel, TargetProjection, decode_embed_number, decode_position_href,
};
use crate::audit_support::temp::TempDir;

fn options(compression: Compression) -> ConvertOptions {
    ConvertOptions { compression }
}

fn convert(input: &[u8], compression: Compression) -> Vec<u8> {
    convert_bytes(input, &options(compression)).expect("audit fixture converts")
}

fn dual(input: &[u8], label: &str) -> Vec<u8> {
    let tmp = TempDir::new(label);
    let src = tmp.write("input.epub", input);
    let dst = tmp.path().join("output.mobi");
    convert_file(&src, &dst, &options(Compression::PalmDoc)).expect("Dual fixture converts");
    std::fs::read(dst).expect("read Dual artifact")
}

fn assert_palmdb_geometry(db: &PalmDb<'_>) {
    assert!(db.record_table_end() <= db.first_record_offset());
    for index in 0..db.record_count() {
        let (start, end) = db.record_bounds(index).expect("record bounds");
        assert!(start >= db.record_table_end());
        assert!(end <= db.bytes_len());
        if let Some((_, next)) = (index + 1 < db.record_count())
            .then(|| db.record_bounds(index + 1).expect("next record bounds"))
        {
            assert!(end <= next, "PalmDB records must not overlap");
        }
    }
}

fn assert_pointer_inventory(db: &PalmDb<'_>, header: &crate::audit_support::palm::MobiHeader<'_>) {
    for (field, raw, coordinate) in header.pointer_inventory() {
        let global = if coordinate == "resource-relative" {
            let anchor = if header.version >= 8 && header.record_index > 0 {
                header
                    .global_record_index(
                        header.fdst_record.expect("resource anchor"),
                        "section-relative",
                    )
                    .unwrap()
                    + 1
            } else {
                header.first_resource as usize
            };
            anchor
                .checked_add(raw as usize)
                .expect("resource pointer overflow")
        } else {
            header
                .global_record_index(raw, coordinate)
                .expect("pointer coordinate")
        };
        assert!(
            global < db.record_count(),
            "{field} raw={raw} coordinate={coordinate} resolves outside actual record count"
        );
    }
}

fn assert_index_family(
    db: &PalmDb<'_>,
    main_index: usize,
) -> Vec<crate::audit_support::palm::IndxRow> {
    let main = parse_indx(db.record(main_index).expect("INDX main record")).expect("INDX main");
    assert!(main.entry_count > 0);
    assert_eq!(main.detail_count, main.row_offsets.len());
    let mut rows = Vec::new();
    for detail_number in 0..main.detail_count {
        let detail = parse_indx(
            db.record(main_index + 1 + detail_number)
                .expect("INDX detail record"),
        )
        .expect("INDX detail");
        let mut detail_with_defs = detail.clone();
        detail_with_defs.tagx = main.tagx.clone();
        let detail_rows = decode_indx_rows(
            db.record(main_index + 1 + detail_number)
                .expect("detail bytes"),
            &detail_with_defs,
        )
        .expect("INDX rows");
        rows.extend(detail_rows);
    }
    assert_eq!(rows.len(), main.entry_count);
    rows
}

fn assert_fdst_coverage(db: &PalmDb<'_>, header: &crate::audit_support::palm::MobiHeader<'_>) {
    let Some(fdst_record) = header.fdst_record else {
        return;
    };
    let raw = reconstruct_text_raw(db, header).expect("FDST raw-text arena");
    let fdst = parse_fdst(
        db.record(
            header
                .global_record_index(
                    fdst_record,
                    if header.version >= 8 && header.record_index > 0 {
                        "section-relative"
                    } else {
                        "global"
                    },
                )
                .expect("FDST pointer"),
        )
        .expect("FDST record"),
    )
    .expect("FDST");
    assert_eq!(fdst.flow_count, fdst.ranges.len());
    let mut end = 0u32;
    for (start, finish) in &fdst.ranges {
        assert!(*start < *finish);
        assert_eq!(*start, end, "FDST flows must be ordered and gap-free");
        end = *finish;
    }
    assert_eq!(end, raw.len() as u32);
}

fn flatten_nav(items: &[NavItem], output: &mut Vec<String>) {
    for item in items {
        if !item.href.is_empty() {
            output.push(item.label.clone());
        }
        flatten_nav(&item.children, output);
    }
}

fn flatten_nav_targets(items: &[NavItem], output: &mut Vec<String>) {
    for item in items {
        if !item.href.is_empty() {
            output.push(item.href.clone());
        }
        flatten_nav_targets(&item.children, output);
    }
}

#[test]
fn batch3_palmdb_and_mobi_pointer_inventory_use_actual_record_geometry() {
    // REQ: FMT-PDB-004, FMT-PDB-005, FMT-MOBI-004, FMT-EXTH-002.
    for input in [
        epub::minimal_reflowable(),
        epub::large_text(250_000),
        epub::cover_png(),
        epub::embedded_font(false),
    ] {
        let bytes = convert(&input, Compression::PalmDoc);
        let db = PalmDb::parse(&bytes).expect("PalmDB");
        assert_palmdb_geometry(&db);
        let header = db.mobi_header(0).expect("MOBI Record 0");
        assert_pointer_inventory(&db, &header);
        if let Some(kf8_boundary) = header.exth_u32(121) {
            let kf8 = db.mobi_header(kf8_boundary as usize).expect("KF8 Record 0");
            assert_pointer_inventory(&db, &kf8);
        }
    }
    let bytes = dual(&epub::resource_graph(), "batch3-pdb-pointer");
    let db = PalmDb::parse(&bytes).expect("Dual PalmDB");
    assert_palmdb_geometry(&db);
    let kf7 = db.mobi_header(0).expect("KF7 Record 0");
    assert_pointer_inventory(&db, &kf7);
    let kf8_index = kf7.exth_u32(121).expect("KF8 boundary") as usize;
    let kf8 = db.mobi_header(kf8_index).expect("KF8 Record 0");
    assert_pointer_inventory(&db, &kf8);
}

#[test]
fn batch3_palmdoc_decodes_exact_length_count_and_trailing_data() {
    // REQ: FMT-PDOC-002, FMT-PDOC-003, FMT-PDOC-005, FMT-TBS-001.
    for compression in [Compression::None, Compression::PalmDoc] {
        let bytes = convert(&epub::large_text(180_000), compression);
        let db = PalmDb::parse(&bytes).expect("PalmDB");
        let header = db.mobi_header(0).expect("MOBI");
        let records = text_record_indices(&db, &header).expect("independent text record scan");
        assert_eq!(records.len(), header.text_record_count);
        assert_eq!(
            reconstruct_text_raw(&db, &header).unwrap().len(),
            header.text_length
        );
        assert_eq!(
            records.last().copied().unwrap() + 1,
            header.first_non_text.unwrap() as usize - 1
        );
        assert_eq!(db.record(records.last().unwrap() + 1).unwrap(), [0, 0]);
        for record_index in records {
            let record = db.record(record_index).unwrap();
            let trailer =
                crate::audit_support::palm::parse_trailer(record, header.extra_record_data_flags)
                    .expect("advertised TBS/trailer framing");
            assert!(trailer.payload_end < record.len());
            assert!(trailer.trailer_byte_count >= trailer.reverse_vwi_bytes);
            assert!(trailer.trailer_byte_count <= record.len());
            assert!(
                !trailer.advertised_tbs.is_empty(),
                "TBS must be emitted for navigation records"
            );
            let mut cursor = 0;
            while cursor < trailer.advertised_tbs.len() {
                let (_, used) = decode_vwi(&trailer.advertised_tbs, cursor).expect("TBS VWI");
                cursor += used;
            }
        }
    }
    let bytes = dual(&epub::minimal_reflowable(), "batch3-palmdoc-dual");
    let db = PalmDb::parse(&bytes).unwrap();
    let header = db.mobi_header(0).unwrap();
    let records = text_record_indices(&db, &header).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        reconstruct_text_raw(&db, &header).unwrap().len(),
        header.text_length
    );
    assert_eq!(header.text_record_count, 1);
}

#[test]
fn batch3_fdst_indx_and_aux_records_have_bounded_reconstructable_geometry() {
    // REQ: FMT-FDST-001..003, FMT-INDX-001..003, FMT-AUX-001..003.
    let bytes = convert(&epub::large_text(300_000), Compression::PalmDoc);
    let db = PalmDb::parse(&bytes).unwrap();
    let header = db.mobi_header(0).unwrap();
    let raw = reconstruct_text_raw(&db, &header).unwrap();
    assert_fdst_coverage(&db, &header);
    let fdst = parse_fdst(db.record(header.fdst_record.unwrap() as usize).unwrap()).expect("FDST");
    assert_eq!(fdst.ranges.first().unwrap().0, 0);

    let ncx_rows = assert_index_family(&db, header.ncx_record.unwrap() as usize);
    assert!(!ncx_rows.is_empty());
    let skel_rows = assert_index_family(&db, header.skeleton_index.unwrap() as usize);
    assert_eq!(skel_rows.len(), 1);
    for row in &skel_rows {
        let values = row.values.get(&6).expect("SKEL range values");
        assert_eq!(values.len(), 4);
        assert!(values[0] + values[1] <= raw.len() as u32);
    }
    let frag_rows = assert_index_family(&db, header.index_record.unwrap() as usize);
    assert!(!frag_rows.is_empty());
    let mut per_file = std::collections::BTreeMap::new();
    for row in &frag_rows {
        let file = row.values.get(&3).unwrap()[0];
        let sequence = row.values.get(&4).unwrap()[0];
        let range = row.values.get(&6).unwrap();
        assert_eq!(range.len(), 2);
        let previous = per_file.entry(file).or_insert((0u32, 0u32));
        assert!(sequence >= previous.1);
        assert_eq!(range[0], previous.0);
        previous.0 = range[0] + range[1];
        previous.1 = sequence;
    }
    assert!(
        ncx_rows
            .iter()
            .any(|row| row.values.values().flatten().any(|v| *v > 127)),
        "actual output must cross a VWI boundary"
    );
    for value in [0, 127, 128, 16_383, 16_384, u32::MAX] {
        let encoded = encode_vwi_audit(value);
        assert_eq!(decode_vwi(&encoded, 0).unwrap(), (value, encoded.len()));
    }

    let mut seen = BTreeSet::new();
    for index in 0..db.record_count() {
        let record = db.record(index).unwrap();
        if matches!(record.get(..4), Some(b"FCIS" | b"FLIS" | b"DATP")) {
            let (kind, declared) = parse_aux_record(record).expect("bounded AUX record");
            assert!(declared <= record.len());
            seen.insert(kind);
            if kind == "FCIS" {
                assert_eq!(
                    u32::from_be_bytes(record[20..24].try_into().unwrap()),
                    header.text_length as u32
                );
            }
        }
    }
    assert!(seen.contains("FCIS") && seen.contains("FLIS"));
    for input in [
        epub::minimal_reflowable(),
        epub::resource_graph(),
        epub::mixed_layout(),
        epub::fixed_layout(),
    ] {
        let output = convert(&input, Compression::None);
        let output_db = PalmDb::parse(&output).unwrap();
        let output_header = output_db.mobi_header(0).unwrap();
        assert_fdst_coverage(&output_db, &output_header);
    }
    let dual_bytes = dual(&epub::resource_graph(), "batch3-datp-bounds");
    let dual_db = PalmDb::parse(&dual_bytes).unwrap();
    let datp = (0..dual_db.record_count())
        .find(|index| dual_db.record(*index).unwrap().starts_with(b"DATP"))
        .expect("Dual output DATP record");
    let (kind, declared) = parse_aux_record(dual_db.record(datp).unwrap()).unwrap();
    assert_eq!(kind, "DATP");
    assert!(declared <= dual_db.record(datp).unwrap().len());
}

#[test]
fn batch3_reconstructs_ncx_guide_resc_and_position_geometry_from_target_records() {
    // REQ: FMT-SKEL-001, FMT-FRAG-001, FMT-NCX-001, FMT-GUIDE-001,
    // FMT-POS-001, FMT-TBS-001, FMT-RESC-001, RES-007, CONT-004,
    // AMZ-NAV-003..004.
    let source = SourceModel::parse(&epub::nested_navigation()).unwrap();
    let bytes = convert(&epub::nested_navigation(), Compression::None);
    let target = TargetProjection::parse(Box::leak(bytes.into_boxed_slice())).unwrap();
    let ncx = target.ncx_entries().unwrap();
    let db = &target.db;
    let ncx_main = parse_indx(
        db.record(target.header.ncx_record.unwrap() as usize)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ncx_main.entry_count, ncx.len());
    assert_index_family(db, target.header.ncx_record.unwrap() as usize);
    assert_eq!(ncx.len(), 3);
    assert!(
        ncx.windows(2)
            .all(|pair| pair[0].sequence <= pair[1].sequence)
    );
    assert!(
        ncx.iter()
            .all(|entry| entry.offset < target.header.text_length as u32)
    );
    assert!(ncx.iter().any(|entry| entry.parent.is_some()));
    let guide = target.guide_entries().unwrap();
    assert!(!guide.is_empty());
    for (_, _, sequence, offset) in guide {
        assert!((sequence as usize) < source.sections.len());
        assert!(offset < target.header.text_length as u32);
    }
    let guide_main = target.header.guide_index.unwrap() as usize;
    let guide_index = parse_indx(db.record(guide_main).unwrap()).unwrap();
    assert_eq!(guide_index.entry_count, 1);
    assert_index_family(db, guide_main);
    let body_start = target.rawml.find("<body").unwrap();
    let start_reading = target.header.exth_u32(116).unwrap() as usize;
    assert!(start_reading >= body_start && start_reading <= body_start + 64);

    let fixed_source = SourceModel::parse(&epub::fixed_layout()).unwrap();
    let fixed = TargetProjection::parse(Box::leak(
        convert(&epub::fixed_layout(), Compression::None).into_boxed_slice(),
    ))
    .unwrap();
    let resc = fixed.resc_metadata().unwrap();
    assert_eq!(
        resc.get("rendition:orientation"),
        fixed_source.orientation.as_ref()
    );
    assert_eq!(resc.get("rendition:spread"), fixed_source.spread.as_ref());
    assert_eq!(
        resc.get("rendition:viewport"),
        fixed_source.viewport.as_ref()
    );
    let spine = fixed.resc_spine_properties().unwrap();
    assert_eq!(spine.len(), fixed_source.sections.len());
    for (id, properties, linear) in spine {
        let section = fixed_source
            .sections
            .iter()
            .find(|section| section.href.contains(&id))
            .unwrap_or(&fixed_source.sections[0]);
        assert_eq!(properties, section.source_properties);
        assert!(linear);
    }
    let fixed_db = fixed.db;
    let resc_record = (0..fixed_db.record_count())
        .find(|index| fixed_db.record(*index).unwrap().starts_with(b"RESC"))
        .unwrap();
    let record = fixed_db.record(resc_record).unwrap();
    let header_len = u32::from_be_bytes(record[12..16].try_into().unwrap()) as usize;
    assert!(16 + header_len <= record.len());

    let large = TargetProjection::parse(Box::leak(
        convert(&epub::large_text(180_000), Compression::PalmDoc).into_boxed_slice(),
    ))
    .unwrap();
    let mut previous = 0usize;
    for index in text_record_indices(&large.db, &large.header).unwrap() {
        let trailer = crate::audit_support::palm::parse_trailer(
            large.db.record(index).unwrap(),
            large.header.extra_record_data_flags,
        )
        .unwrap();
        assert!(trailer.payload_end > 0);
        assert!(index >= previous);
        previous = index;
    }
}

#[test]
fn batch3_exth_cover_thumbnail_and_fixed_values_resolve_in_their_declared_coordinates() {
    // REQ: FMT-EXTH-003, FMT-EXTH-005, FMT-EXTH-007, FMT-RES-001..003,
    // FMT-COVER-001..002.
    let source = SourceModel::parse(&epub::cover_png()).unwrap();
    let bytes: &'static [u8] =
        Box::leak(convert(&epub::cover_png(), Compression::None).into_boxed_slice());
    let target = TargetProjection::parse(bytes).unwrap();
    let first = target.header.first_resource as usize;
    let cover_offset = target.header.exth_u32(201).unwrap() as usize;
    let thumbnail_offset = target.header.exth_u32(202).unwrap() as usize;
    let cover_record = target.db.record(first + cover_offset).unwrap();
    let thumbnail_record = target.db.record(first + thumbnail_offset).unwrap();
    let source_image = source.resources.get("EPUB/images/cover.png").unwrap();
    let source_decoded = image::load_from_memory(&source_image.1).unwrap();
    let cover_decoded = image::load_from_memory(cover_record).unwrap();
    let thumbnail_decoded = image::load_from_memory(thumbnail_record).unwrap();
    assert_eq!(
        (cover_decoded.width(), cover_decoded.height()),
        (source_decoded.width(), source_decoded.height())
    );
    assert_eq!(
        (thumbnail_decoded.width(), thumbnail_decoded.height()),
        (source_decoded.width(), source_decoded.height())
    );
    assert_ne!(first + cover_offset, first + thumbnail_offset);
    let thumbnail_uri = target
        .header
        .exth_text(129)
        .expect("EXTH 129 thumbnail URI");
    let decoded_thumbnail_offset = thumbnail_uri
        .strip_prefix("kindle:embed:")
        .and_then(decode_embed_number)
        .unwrap();
    assert_eq!(
        decoded_thumbnail_offset, thumbnail_offset,
        "EXTH 129 embed coordinate must equal the EXTH 202 thumbnail offset"
    );
    let exth_129_record_index = first
        .checked_add(decoded_thumbnail_offset)
        .expect("EXTH 129 resource index");
    assert_eq!(
        exth_129_record_index,
        first + thumbnail_offset,
        "EXTH 129 must resolve in the same First Image-relative coordinate system as EXTH 202"
    );
    assert_eq!(
        target.db.record(exth_129_record_index).unwrap(),
        thumbnail_record,
        "EXTH 129 must resolve to the EXTH 202 thumbnail resource"
    );
    assert_ne!(
        exth_129_record_index,
        first + cover_offset,
        "distinct cover/thumbnail fixture must not let EXTH 129 regress to EXTH 201"
    );
    assert!(target.header.exth_u32(116).unwrap() < target.header.text_length as u32);

    let fixed = TargetProjection::parse(Box::leak(
        convert(&epub::fixed_layout(), Compression::None).into_boxed_slice(),
    ))
    .unwrap();
    assert_eq!(fixed.exth_text(122).as_deref(), Some("true"));
    assert!(
        fixed.exth_text(123).is_none(),
        "no source book-type must not invent EXTH 123"
    );
    assert_eq!(fixed.exth_text(124).as_deref(), Some("landscape"));
    assert_eq!(fixed.exth_text(126).as_deref(), Some("1200x800"));
}

fn assert_dual_common_semantics(input: Vec<u8>, marker: &str, label: &str) {
    let source = SourceModel::parse(&input).unwrap();
    let bytes: &'static [u8] = Box::leak(dual(&input, label).into_boxed_slice());
    let db = PalmDb::parse(bytes).unwrap();
    let kf7 = db.mobi_header(0).unwrap();
    let kf8_index = kf7.exth_u32(121).unwrap() as usize;
    let target = TargetProjection::parse_at(bytes, kf8_index).unwrap();
    assert_eq!(target.sections.len(), source.sections.len());
    assert_eq!(
        target.body_text(),
        source
            .sections
            .iter()
            .map(|section| section.visible_text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert!(target.rawml.contains(marker));
    assert_eq!(
        target.exth_text(503).as_deref(),
        Some(source.title.as_str())
    );
    assert_eq!(
        target.exth_text(524).as_deref(),
        Some(source.language.as_str())
    );
    assert_eq!(
        target.exth_text(106).as_deref(),
        source.publication_date.as_deref()
    );
    for (source_section, target_section) in source.sections.iter().zip(&target.sections) {
        assert_eq!(target_section.visible_text, source_section.visible_text);
        assert_eq!(
            target_section.ids,
            source_section.ids.iter().cloned().collect()
        );
        assert_eq!(target_section.fixed, source_section.fixed);
        assert_eq!(target_section.links.len(), source_section.links.len());
        assert_eq!(target_section.images.len(), source_section.images.len());
        for (index, (href, label)) in source_section.links.iter().enumerate() {
            let (sequence, offset) = decode_position_href(&target_section.links[index].0)
                .expect("Dual internal links must reconstruct to Kindle positions");
            let destination = href.split(['#', '?']).next().unwrap_or(href);
            let expected_section = source
                .sections
                .iter()
                .position(|section| section.href == destination)
                .expect("Dual source link destination must identify a source spine section");
            assert_eq!(sequence as usize, expected_section);
            assert_eq!(target_section.links[index].1, *label);
            assert!(offset < target.header.text_length as u32);
            if let Some(fragment) = href.split_once('#').map(|(_, fragment)| fragment) {
                assert!(
                    source.sections[expected_section]
                        .ids
                        .iter()
                        .any(|id| id == fragment)
                );
                assert!(target.sections[expected_section].ids.contains(fragment));
            }
        }
        for (index, (source_href, alt)) in source_section.images.iter().enumerate() {
            let number = target_section.images[index]
                .strip_prefix("kindle:embed:")
                .and_then(|value| decode_embed_number(value.split('?').next().unwrap_or(value)))
                .expect("Dual image occurrence must use embed URI");
            assert_eq!(target_section.image_alts[index], *alt);
            if source_href.ends_with("cover.png") {
                let source_image =
                    image::load_from_memory(&source.resources.get(source_href).unwrap().1)
                        .expect("source cover image");
                let target_image = image::load_from_memory(target.embedded_bytes(number).unwrap())
                    .expect("Dual cover image");
                assert_eq!(
                    (target_image.width(), target_image.height()),
                    (source_image.width(), source_image.height()),
                    "Dual cover occurrence must preserve logical image dimensions"
                );
            } else {
                assert_eq!(
                    target.embedded_bytes(number).unwrap(),
                    &source.resources.get(source_href).unwrap().1
                );
            }
        }
    }
    let mut expected_labels = Vec::new();
    flatten_nav(&source.toc, &mut expected_labels);
    let expected_targets = {
        let mut targets = Vec::new();
        flatten_nav_targets(&source.toc, &mut targets);
        targets
    };
    let target_nav = target.ncx_entries().unwrap();
    let target_labels = target_nav
        .iter()
        .map(|entry| entry.label.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        target_labels, expected_labels,
        "Dual KF8 navigation must use the same source-derived hierarchy labels"
    );
    assert_eq!(target_nav.len(), expected_targets.len());
    assert!(
        target_nav
            .windows(2)
            .all(|pair| pair[0].sequence <= pair[1].sequence)
    );
    for (entry, href) in target_nav.iter().zip(expected_targets) {
        let expected_section = source
            .sections
            .iter()
            .position(|section| section.href == href.split(['#', '?']).next().unwrap_or(&href))
            .expect("Dual TOC target must identify a source spine section");
        assert_eq!(entry.sequence as usize, expected_section);
        assert!(entry.offset < target.header.text_length as u32);
    }
    for css in &source.css {
        for (property, value) in &css.declarations {
            assert!(
                target.css.replace(' ', "").contains(&format!(
                    "{}:{}",
                    property,
                    value.replace(' ', "")
                )),
                "Dual CSS semantic declaration lost: {property}:{value}"
            );
        }
    }
    let kf7_raw = reconstruct_text_raw(&db, &kf7).unwrap();
    let kf7_start = kf7.exth_u32(116).expect("KF7 Start Reading");
    let kf7_body_start = String::from_utf8_lossy(&kf7_raw)
        .find("<body")
        .expect("KF7 stub must contain the semantic body start");
    assert!(
        (kf7_start as usize) >= kf7_body_start && (kf7_start as usize) <= kf7_body_start + 64,
        "KF7 Go-to-Beginning must target the stub body, not merely an in-range byte"
    );
    assert_eq!(
        text_record_indices(&db, &kf7).unwrap().len(),
        kf7.text_record_count
    );
    let kf8 = db.mobi_header(kf8_index).unwrap();
    assert_eq!(
        text_record_indices(&db, &kf8).unwrap().len(),
        kf8.text_record_count
    );
    assert!(target.header.version >= 8);
    let kf8_raw = reconstruct_text_raw(&db, &kf8).unwrap();
    let kf8_body_start = String::from_utf8_lossy(&kf8_raw)
        .find("<body")
        .expect("KF8 rendition must contain the semantic body start");
    let kf8_start = target.header.exth_u32(116).unwrap() as usize;
    let first_reading_section = target
        .sections
        .iter()
        .position(|section| !section.visible_text.trim().is_empty())
        .expect("KF8 rendition must have a semantic reading section");
    let html_start = target
        .rawml
        .match_indices("<html")
        .map(|(offset, _)| offset)
        .nth(first_reading_section)
        .expect("reading section must have an HTML root");
    let section_end = target.rawml[html_start..]
        .find("<?xml")
        .map(|offset| html_start + offset)
        .unwrap_or(target.rawml.len());
    let section = &target.rawml[html_start..section_end];
    let content_start = section
        .find("</html>")
        .map(|offset| offset + "</html>".len())
        .unwrap_or_else(|| section.find("<body").unwrap_or(0));
    assert!(
        kf8_start >= html_start && kf8_start <= html_start + content_start,
        "KF8 Go-to-Beginning must enter the first semantic reading section before its content: start={kf8_start}, section={html_start}, content={}",
        html_start + content_start
    );
    assert!(kf8_start < kf8_raw.len() && kf8_start >= kf8_body_start);
    for section in &target.sections {
        for image in &section.images {
            if let Some(number) = image
                .strip_prefix("kindle:embed:")
                .and_then(|value| decode_embed_number(value.split('?').next().unwrap_or(value)))
            {
                assert!(!target.embedded_bytes(number).unwrap().is_empty());
            }
        }
    }
}

#[test]
fn batch3_dual_common_invariants_run_identically_across_all_structural_inputs() {
    // REQ: DUAL-005, DUAL-006, DUAL-008, CORPUS-004, VAL-007.
    for (input, marker, label) in [
        (
            epub::minimal_reflowable(),
            "AUTH_MINIMAL_ALPHA",
            "batch3-dual-minimal",
        ),
        (
            epub::vertical_japanese(),
            "AUTH_JA_開始",
            "batch3-dual-vertical",
        ),
        (
            epub::resource_graph(),
            "AUTH_RESOURCE_TWO",
            "batch3-dual-resource",
        ),
        (epub::cover_png(), "AUTH_COVER_BODY", "batch3-dual-cover"),
        (
            epub::large_text(120_000),
            "AUTH_LARGE_END",
            "batch3-dual-large",
        ),
    ] {
        assert_dual_common_semantics(input, marker, label);
    }
    let bytes = dual(&epub::resource_graph(), "batch3-dual-pointer-sets");
    let db = PalmDb::parse(&bytes).unwrap();
    let kf7 = db.mobi_header(0).unwrap();
    let kf8_index = kf7.exth_u32(121).unwrap() as usize;
    let kf8 = db.mobi_header(kf8_index).unwrap();
    assert_eq!(kf7.version, 6);
    assert!(kf8.version >= 8);
    assert_eq!(kf7.content_record_range, Some((1, 2)));
    assert!(kf8.first_non_text.unwrap() < kf8.text_record_count as u32 + 4);
    assert!(
        kf8.global_record_index(kf8.fdst_record.unwrap(), "section-relative")
            .unwrap()
            != kf7.first_resource as usize
    );
}

#[test]
fn batch3_recipe_manifest_and_public_api_variant_surface_are_explicit() {
    // REQ: DUAL-009, VAL-003.
    let recipes = [
        (
            "minimal",
            epub::minimal_reflowable(),
            1usize,
            1usize,
            "AUTH_MINIMAL_ALPHA",
        ),
        ("vertical", epub::vertical_japanese(), 1, 1, "AUTH_JA_開始"),
        (
            "resource",
            epub::resource_graph(),
            2,
            2,
            "AUTH_RESOURCE_TWO",
        ),
        ("cover", epub::cover_png(), 2, 1, "AUTH_COVER_BODY"),
        ("large", epub::large_text(120_000), 1, 1, "AUTH_LARGE_END"),
    ];
    for (name, input, sections, toc, marker) in recipes {
        let source = SourceModel::parse(&input).unwrap();
        assert_eq!(
            source.sections.len(),
            sections,
            "frozen recipe {name} section identity"
        );
        assert_eq!(source.toc.len(), toc, "frozen recipe {name} TOC identity");
        assert!(
            source
                .sections
                .iter()
                .any(|section| section.visible_text.contains(marker))
        );
        let expected_resources = match name {
            "minimal" | "large" => 2,
            "vertical" => 3,
            "resource" => 6,
            "cover" => 4,
            _ => unreachable!(),
        };
        let expected_css = match name {
            "vertical" => 1,
            "resource" => 2,
            _ => 0,
        };
        let identity = format!(
            "{name}:sections={}:toc={}:resources={}:css={}:fixed={}:progression={}:marker={marker}",
            source.sections.len(),
            source.toc.len(),
            source.resources.len(),
            source.css.len(),
            source.rendition_layout.as_deref() == Some("pre-paginated"),
            source.progression,
        );
        let expected = format!(
            "{name}:sections={sections}:toc={toc}:resources={expected_resources}:css={expected_css}:fixed=false:progression=ltr:marker={marker}"
        );
        assert_eq!(
            identity, expected,
            "semantic recipe identity changed for {name}"
        );
    }
    let public_api = include_str!("../../src/lib.rs");
    for forbidden in [
        "MobiVariant",
        "characterization",
        "experimental variant selector",
    ] {
        assert!(
            !public_api.contains(forbidden),
            "public API must expose one canonical layout: {forbidden}"
        );
    }
    let traceability = include_str!("../audit_support/regression_traceability.md");
    for required in [
        "LAYOUT-008",
        "DUAL-005",
        "FMT-COVER-001",
        "AMZ-QA-001",
        "AMZ-QA-002",
        "PKG-003",
        "FMT-EXTH-009",
        "RES-009",
    ] {
        assert!(
            traceability.contains(required),
            "regression traceability missing {required}"
        );
    }
    let policy = include_str!("../README.authority-audit.md");
    assert!(policy.contains("does not impose an unproven"));
}
