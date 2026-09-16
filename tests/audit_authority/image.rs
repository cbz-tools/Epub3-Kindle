use epub3_kindle::{Compression, ConvertOptions, convert_bytes};

use crate::audit_support::epub;
use crate::audit_support::semantic::{SourceModel, TargetProjection};

const LD_IMAGE_MAX_BYTES: usize = 131_072;

fn plain() -> ConvertOptions {
    ConvertOptions {
        compression: Compression::None,
    }
}

fn source_and_target(comic: bool) -> (SourceModel, TargetProjection<'static>) {
    let input = epub::large_image_resources(comic);
    let source = SourceModel::parse(&input).expect("LD image source model");
    let output = convert_bytes(&input, &plain()).expect("LD image fixture converts");
    let output: &'static [u8] = Box::leak(output.into_boxed_slice());
    let target = TargetProjection::parse(output).expect("LD image target projection");
    (source, target)
}

#[test]
fn non_comic_large_logical_cover_jpeg_and_png_are_bounded_and_controls_are_byte_identical() {
    let (source, target) = source_and_target(false);
    // `large-jpeg` is explicitly the logical cover in the fixture metadata.
    // Its emitted KF8 resource is read independently from the image-record inventory.
    let source_images = [
        "EPUB/images/large.jpg",
        "EPUB/images/large.png",
        "EPUB/images/undershoot.jpg",
        "EPUB/images/boundary.png",
        "EPUB/images/medium.png",
        "EPUB/images/small.jpg",
        "EPUB/images/small.png",
    ];
    let records = manifest_image_records(&target, source_images.len());
    assert_eq!(
        records.len(),
        source_images.len(),
        "KF8 manifest image record inventory"
    );

    let first_image = target.header.first_resource as usize;
    let image_records = target.image_records();
    let cover_offset = target
        .header
        .exth_u32(201)
        .expect("EXTH 201 logical cover offset") as usize;
    let cover_record_index = first_image
        .checked_add(cover_offset)
        .expect("EXTH 201 cover record index");
    assert_eq!(
        cover_record_index, records[0],
        "EXTH 201 must point to the fixture's large-jpeg logical cover resource"
    );
    assert!(
        image_records.contains(&cover_record_index),
        "EXTH 201 logical cover must resolve to an independent image record"
    );
    let cover_bytes = target
        .db
        .record(cover_record_index)
        .expect("EXTH 201 logical cover record");
    assert_eq!(
        image::guess_format(cover_bytes).unwrap(),
        image::ImageFormat::Jpeg,
        "EXTH 201 logical cover must be emitted as JPEG"
    );
    let source_cover = image::load_from_memory(
        &source
            .resources
            .get("EPUB/images/large.jpg")
            .expect("source logical cover resource")
            .1,
    )
    .expect("source logical cover must decode");
    let target_cover =
        image::load_from_memory(cover_bytes).expect("EXTH 201 logical cover must decode");
    assert_aspect_ratio_preserved(
        source_cover.width(),
        source_cover.height(),
        target_cover.width(),
        target_cover.height(),
    );

    if let Some(thumbnail_offset) = target.header.exth_u32(202) {
        let thumbnail_record_index = first_image
            .checked_add(thumbnail_offset as usize)
            .expect("EXTH 202 thumbnail record index");
        assert_ne!(
            thumbnail_record_index, cover_record_index,
            "EXTH 202 thumbnail must not alias the EXTH 201 logical cover"
        );
        assert!(
            image_records.contains(&thumbnail_record_index),
            "EXTH 202 thumbnail must resolve to an independent image record"
        );
        assert_eq!(
            Some(thumbnail_record_index),
            image_records.last().copied(),
            "EXTH 202 must point to the generated thumbnail resource"
        );
        let thumbnail_bytes = target
            .db
            .record(thumbnail_record_index)
            .expect("EXTH 202 thumbnail record");
        assert_eq!(
            image::guess_format(thumbnail_bytes).unwrap(),
            image::ImageFormat::Jpeg,
            "EXTH 202 thumbnail must be emitted as JPEG"
        );
        image::load_from_memory(thumbnail_bytes)
            .expect("EXTH 202 thumbnail must be independently decodable");
    }

    for (index, href) in source_images.iter().enumerate() {
        let source_bytes = &source.resources.get(*href).unwrap().1;
        let target_bytes = target
            .db
            .record(records[index])
            .expect("KF8 image record must be readable");
        match index {
            0 | 1 => {
                assert!(source_bytes.len() > LD_IMAGE_MAX_BYTES);
                assert!(target_bytes.len() <= LD_IMAGE_MAX_BYTES);
                assert_eq!(&target_bytes[..3], &[0xff, 0xd8, 0xff]);
                assert_eq!(
                    image::guess_format(target_bytes).unwrap(),
                    image::ImageFormat::Jpeg,
                    "large image must be emitted as JPEG"
                );
                let source_image = image::load_from_memory(source_bytes).unwrap();
                let target_image = image::load_from_memory(target_bytes).unwrap();
                if index == 0 {
                    assert!(
                        target_bytes.len() <= LD_IMAGE_MAX_BYTES,
                        "non-comic logical cover JPEG must be bounded"
                    );
                    assert_eq!(
                        image::guess_format(target_bytes).unwrap(),
                        image::ImageFormat::Jpeg,
                        "non-comic logical cover must remain a valid JPEG"
                    );
                    image::load_from_memory(target_bytes)
                        .expect("non-comic logical cover must remain decodable");
                }
                assert_aspect_ratio_preserved(
                    source_image.width(),
                    source_image.height(),
                    target_image.width(),
                    target_image.height(),
                );
                if index == 1 {
                    let source_rgba = source_image.to_rgba8();
                    assert!(
                        source_rgba.pixels().all(|pixel| pixel[3] == 0),
                        "large PNG source must be fully transparent"
                    );
                    let target_rgb = target_image.to_rgb8();
                    assert!(
                        target_rgb
                            .pixels()
                            .all(|pixel| pixel.0.iter().all(|channel| *channel >= 245)),
                        "transparent PNG must flatten near the safe white background"
                    );
                }
            }
            2 => {
                assert!(source_bytes.len() > LD_IMAGE_MAX_BYTES);
                assert!(target_bytes.len() <= LD_IMAGE_MAX_BYTES);
                assert_eq!(
                    image::guess_format(target_bytes).unwrap(),
                    image::ImageFormat::Jpeg
                );
                let source_image = image::load_from_memory(source_bytes).unwrap();
                let target_image = image::load_from_memory(target_bytes).unwrap();
                let initial_width = independent_initial_predicted_width(source_bytes);
                assert!(
                    target_image.width() > initial_width,
                    "dimension refinement must exceed the initial predicted width"
                );
                assert_aspect_ratio_preserved(
                    source_image.width(),
                    source_image.height(),
                    target_image.width(),
                    target_image.height(),
                );
            }
            3 => {
                assert_eq!(source_bytes.len(), LD_IMAGE_MAX_BYTES);
                assert_eq!(
                    target_bytes, source_bytes,
                    "boundary PNG must remain unchanged"
                );
                assert_eq!(
                    image::guess_format(target_bytes).unwrap(),
                    image::ImageFormat::Png
                );
                image::load_from_memory(target_bytes).expect("boundary PNG must remain decodable");
            }
            4 => {
                assert_eq!(source_bytes.len(), 120_000);
                assert_eq!(
                    target_bytes, source_bytes,
                    "100..128 KiB ordinary image must remain byte-identical"
                );
                image::load_from_memory(target_bytes).expect("medium image must remain decodable");
            }
            5 | 6 => {
                assert!(source_bytes.len() <= LD_IMAGE_MAX_BYTES);
                assert_eq!(
                    target_bytes, source_bytes,
                    "small image must remain byte-identical"
                );
                image::load_from_memory(target_bytes).expect("small image must remain decodable");
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn comic_large_logical_cover_and_page_images_remain_byte_identical() {
    let (source, target) = source_and_target(true);
    // The same large JPEG is the logical cover, while the large PNG is a comic page.
    let source_images = [
        "EPUB/images/large.jpg",
        "EPUB/images/large.png",
        "EPUB/images/undershoot.jpg",
        "EPUB/images/boundary.png",
        "EPUB/images/medium.png",
        "EPUB/images/small.jpg",
        "EPUB/images/small.png",
    ];
    let records = manifest_image_records(&target, source_images.len());
    assert_eq!(
        records.len(),
        source_images.len(),
        "comic manifest image record inventory"
    );
    for (index, href) in source_images.iter().enumerate() {
        let source_bytes = &source.resources.get(*href).unwrap().1;
        let target_bytes = target
            .db
            .record(records[index])
            .expect("comic KF8 image record must be readable");
        let role = match index {
            0 => "comic logical cover JPEG",
            1 => "comic large PNG page image",
            _ => "comic control image",
        };
        assert_eq!(
            target_bytes, source_bytes,
            "{role} must remain byte-identical and must not be generically re-encoded"
        );
    }
}

fn manifest_image_records(target: &TargetProjection<'_>, expected_count: usize) -> Vec<usize> {
    let records = target.image_records();
    let first_image = target.header.first_resource as usize;
    let thumbnail = target
        .header
        .exth_u32(202)
        .map(|offset| first_image + offset as usize);
    let manifest_records = records
        .into_iter()
        .filter(|record| Some(*record) != thumbnail)
        .collect::<Vec<_>>();
    assert_eq!(
        manifest_records.len(),
        expected_count,
        "generated cover thumbnail must not be counted as a manifest image"
    );
    manifest_records
}

fn assert_aspect_ratio_preserved(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) {
    let source_ratio = source_width as f64 / source_height as f64;
    let target_ratio = target_width as f64 / target_height as f64;
    assert!(
        (source_ratio - target_ratio).abs() <= 0.01,
        "aspect ratio changed from {source_width}x{source_height} to {target_width}x{target_height}"
    );
}

fn independent_initial_predicted_width(source_bytes: &[u8]) -> u32 {
    let source = image::load_from_memory(source_bytes).unwrap().to_rgb8();
    let source_width = source.width();
    let mut q80 = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut q80, 80)
        .encode_image(&image::DynamicImage::ImageRgb8(source))
        .unwrap();
    assert!(q80.len() > LD_IMAGE_MAX_BYTES);
    let scale = (116_736.0 / q80.len() as f64).sqrt().clamp(0.0, 1.0);
    (source_width as f64 * scale).round() as u32
}

#[test]
fn small_logical_cover_png_uses_independent_jpeg_normalization_and_jpeg_is_unchanged() {
    for (input, href, source_is_png) in [
        (epub::cover_png(), "EPUB/images/cover.png", true),
        (epub::cover_jpeg(), "EPUB/images/cover.jpg", false),
    ] {
        let source = SourceModel::parse(&input).expect("small cover source model");
        let output = convert_bytes(&input, &plain()).expect("small cover fixture converts");
        let output: &'static [u8] = Box::leak(output.into_boxed_slice());
        let target = TargetProjection::parse(output).expect("small cover target projection");
        let first_image = target.header.first_resource as usize;
        let cover_offset = target
            .header
            .exth_u32(201)
            .expect("EXTH logical cover offset") as usize;
        let target_bytes = target
            .db
            .record(first_image + cover_offset)
            .expect("logical cover record");
        let source_bytes = &source.resources.get(href).unwrap().1;
        assert!(source_bytes.len() <= LD_IMAGE_MAX_BYTES);
        if source_is_png {
            assert_eq!(
                image::guess_format(target_bytes).unwrap(),
                image::ImageFormat::Jpeg
            );
            assert_ne!(target_bytes, source_bytes.as_slice());
            image::load_from_memory(target_bytes).expect("normalized PNG cover must decode");
        } else {
            assert_eq!(target_bytes, source_bytes.as_slice());
        }
    }
}
