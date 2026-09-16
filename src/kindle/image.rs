//! Lower ordinary JPEG and PNG resources to bounded JPEG representations.
//!
//! This module owns only the byte-level image mechanism. Publication policy
//! (comic exclusions, logical covers, and resource selection) remains in
//! `normalize`.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, ImageFormat, ImageReader, RgbImage};

use crate::error::{Error, Result};

pub(crate) const KINDLE_LD_IMAGE_MAX_BYTES: usize = 131_072;

const LD_JPEG_MAX_QUALITY: u8 = 80;
const LD_JPEG_MIN_QUALITY: u8 = 50;
const LD_PREDICTIVE_TARGET_BYTES: usize = 116_736;
const LD_DIMENSION_REFINEMENT_MIN_BYTES: usize = 102_400;
const MAX_FURTHER_RESIZE_CORRECTIONS: usize = 3;
const MAX_DIMENSION_REFINEMENT_PROBES: usize = 4;
const JPEG_MAX_DIMENSION: u32 = u16::MAX as u32;
const COVER_JPEG_QUALITY: u8 = 100;

/// Convert one ordinary JPEG or PNG to a valid bounded JPEG.
///
/// The source is decoded once into RGB pixels. The Q80 original-size probe is
/// followed by a predictive resize and a bounded, sequential quality search;
/// quality trials borrow the resized RGB buffer and never clone it.
pub(crate) fn convert_large_image_to_jpeg(data: &[u8], media_type: &str) -> Result<Vec<u8>> {
    let rgb = decode_source(data, media_type)?;
    let original_width = rgb.width();
    let original_height = rgb.height();

    let probe = encode_rgb(&rgb, LD_JPEG_MAX_QUALITY)?;
    if probe.len() <= KINDLE_LD_IMAGE_MAX_BYTES {
        return Ok(probe);
    }

    let (mut width, mut height) = predicted_dimensions(
        original_width,
        original_height,
        predicted_scale(probe.len()),
    );
    let mut resized = resize_rgb(&rgb, width, height);

    for correction in 0..=MAX_FURTHER_RESIZE_CORRECTIONS {
        let q80 = encode_rgb(&resized, LD_JPEG_MAX_QUALITY)?;
        if q80.len() <= KINDLE_LD_IMAGE_MAX_BYTES {
            if correction == 0 && q80.len() < LD_DIMENSION_REFINEMENT_MIN_BYTES {
                drop(resized);
                return refine_predicted_dimensions(
                    &rgb,
                    original_width,
                    original_height,
                    width,
                    height,
                    q80,
                );
            }
            return Ok(q80);
        }

        let search = bounded_quality_search(&resized)?;
        if let Some(accepted) = search.accepted {
            return Ok(accepted);
        }

        if correction == MAX_FURTHER_RESIZE_CORRECTIONS {
            break;
        }

        // The quality search reports the Q_MIN size, which is the safest
        // current estimate for the next scale correction.
        let (next_width, next_height) =
            predicted_dimensions(width, height, predicted_scale(search.minimum_size));
        if (next_width, next_height) == (width, height) {
            break;
        }
        width = next_width;
        height = next_height;
        resized = resize_rgb(&rgb, width, height);
    }

    Err(Error::Output(format!(
        "large {media_type} resource could not be encoded as a valid JPEG of {KINDLE_LD_IMAGE_MAX_BYTES} bytes or less"
    )))
}

/// Normalize a small logical PNG cover using the established cover JPEG policy.
///
/// The cover module owns the decision to apply this policy and thumbnail
/// orchestration; this module owns the PNG decode, alpha flattening, and JPEG
/// encode shared by all full-size raster paths.
pub(crate) fn normalize_small_cover_png(
    data: &[u8],
    make_thumbnail: impl FnOnce(&DynamicImage) -> Option<Vec<u8>>,
) -> Result<(Vec<u8>, Option<Vec<u8>>)> {
    let image = decode_image(data, "image/png")?;
    let thumbnail = make_thumbnail(&image);
    let rgb = flatten_on_white(&image);
    let encoded = encode_rgb(&rgb, COVER_JPEG_QUALITY)?;
    Ok((encoded, thumbnail))
}

fn decode_source(data: &[u8], media_type: &str) -> Result<RgbImage> {
    let expected = expected_format(media_type)?;
    let image = decode_image(data, media_type)?;
    if expected == ImageFormat::Png {
        Ok(flatten_on_white(&image))
    } else {
        Ok(image.to_rgb8())
    }
}

fn decode_image(data: &[u8], media_type: &str) -> Result<DynamicImage> {
    let expected = expected_format(media_type)?;

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|error| Error::Output(format!("large image format detection failed: {error}")))?;
    if reader.format() != Some(expected) {
        return Err(Error::Output(format!(
            "large image data is not {}",
            match expected {
                ImageFormat::Jpeg => "JPEG",
                ImageFormat::Png => "PNG",
                _ => "the expected format",
            }
        )));
    }

    reader
        .decode()
        .map_err(|error| Error::Output(format!("large image decode failed: {error}")))
}

fn expected_format(media_type: &str) -> Result<ImageFormat> {
    let expected = if media_type.eq_ignore_ascii_case("image/jpeg") {
        ImageFormat::Jpeg
    } else if media_type.eq_ignore_ascii_case("image/png") {
        ImageFormat::Png
    } else {
        return Err(Error::Output(format!(
            "large image conversion does not support media type {media_type}"
        )));
    };
    Ok(expected)
}

fn flatten_on_white(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    let mut rgb = RgbImage::new(image.width(), image.height());
    for (source, destination) in rgba.pixels().zip(rgb.pixels_mut()) {
        let alpha = u16::from(source[3]);
        for channel in 0..3 {
            let foreground = u32::from(source[channel]);
            destination[channel] =
                ((foreground * u32::from(alpha) + 255 * u32::from(255 - alpha) + 127) / 255) as u8;
        }
    }
    rgb
}

fn encode_rgb(rgb: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, quality)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|error| Error::Output(format!("large image JPEG encode failed: {error}")))?;
    Ok(encoded)
}

fn resize_rgb(image: &RgbImage, width: u32, height: u32) -> RgbImage {
    image::imageops::resize(image, width, height, image::imageops::FilterType::Lanczos3)
}

fn refine_predicted_dimensions(
    source: &RgbImage,
    original_width: u32,
    original_height: u32,
    initial_width: u32,
    initial_height: u32,
    initial_q80: Vec<u8>,
) -> Result<Vec<u8>> {
    let mut accepted = AcceptedDimensions {
        scale: (f64::from(initial_width) / f64::from(original_width))
            .min(f64::from(initial_height) / f64::from(original_height)),
        width: initial_width,
        height: initial_height,
        encoded: initial_q80,
    };
    let mut oversized_scale = 1.0;

    for _ in 0..MAX_DIMENSION_REFINEMENT_PROBES {
        let candidate_scale = (accepted.scale + oversized_scale) / 2.0;
        let (candidate_width, candidate_height) =
            predicted_dimensions(original_width, original_height, candidate_scale);
        if (candidate_width, candidate_height) == (accepted.width, accepted.height) {
            break;
        }

        let candidate_q80 = {
            let candidate_resized = resize_rgb(source, candidate_width, candidate_height);
            encode_rgb(&candidate_resized, LD_JPEG_MAX_QUALITY)?
        };
        if candidate_q80.len() <= KINDLE_LD_IMAGE_MAX_BYTES {
            accepted = AcceptedDimensions {
                scale: candidate_scale,
                width: candidate_width,
                height: candidate_height,
                encoded: candidate_q80,
            };
            if accepted.encoded.len() >= LD_DIMENSION_REFINEMENT_MIN_BYTES {
                break;
            }
        } else {
            oversized_scale = candidate_scale;
        }
    }

    Ok(accepted.encoded)
}

struct AcceptedDimensions {
    scale: f64,
    width: u32,
    height: u32,
    encoded: Vec<u8>,
}

struct QualitySearch {
    accepted: Option<Vec<u8>>,
    minimum_size: usize,
}

fn bounded_quality_search(image: &RgbImage) -> Result<QualitySearch> {
    let minimum = encode_rgb(image, LD_JPEG_MIN_QUALITY)?;
    let minimum_size = minimum.len();
    let mut accepted = (minimum_size <= KINDLE_LD_IMAGE_MAX_BYTES).then_some(minimum);

    let mut low = LD_JPEG_MIN_QUALITY.saturating_add(1);
    let mut high = LD_JPEG_MAX_QUALITY.saturating_sub(1);
    while low <= high {
        let quality = low + (high - low) / 2;
        let candidate = encode_rgb(image, quality)?;
        if candidate.len() <= KINDLE_LD_IMAGE_MAX_BYTES {
            accepted = Some(candidate);
            low = quality.saturating_add(1);
        } else {
            high = quality.saturating_sub(1);
        }
    }
    Ok(QualitySearch {
        accepted,
        minimum_size,
    })
}

fn predicted_scale(encoded_size: usize) -> f64 {
    if encoded_size == 0 {
        return 1.0;
    }
    let ratio = (LD_PREDICTIVE_TARGET_BYTES as f64) / (encoded_size as f64);
    if ratio.is_finite() && ratio > 0.0 {
        ratio.sqrt().clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn predicted_dimensions(width: u32, height: u32, scale: f64) -> (u32, u32) {
    let mut scale = if scale.is_finite() && scale > 0.0 {
        scale.min(1.0)
    } else {
        0.5
    };
    let dimension_limit_scale = (f64::from(JPEG_MAX_DIMENSION) / f64::from(width))
        .min(f64::from(JPEG_MAX_DIMENSION) / f64::from(height));
    scale = scale.min(dimension_limit_scale);

    let width = (f64::from(width) * scale)
        .round()
        .clamp(1.0, f64::from(width)) as u32;
    let height = (f64::from(height) * scale)
        .round()
        .clamp(1.0, f64::from(height)) as u32;
    (width, height)
}
