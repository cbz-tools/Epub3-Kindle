//! Lower source cover binaries into the representations expected by Kindle.
//!
//! Full-size cover lowering is required and may return a conversion error;
//! library thumbnail generation is separate and best-effort. Cover navigation
//! and XHTML suppression remain the responsibility of `normalize`.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, ImageReader, RgbImage};

use crate::error::{Error, Result};

use super::{KindleResource, image::normalize_small_cover_png};

const LIBRARY_THUMBNAIL_MAX_WIDTH: u32 = 330;
const LIBRARY_THUMBNAIL_MAX_HEIGHT: u32 = 470;
const LIBRARY_THUMBNAIL_JPEG_QUALITY: u8 = 80;

/// Prepare the native cover and, when possible, derive its library thumbnail
/// from the same decoded image.
pub(crate) fn prepare_cover_resource(
    resources: &mut [KindleResource],
    cover_resource_id: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    let Some(cover) =
        cover_resource_id.and_then(|id| resources.iter_mut().find(|resource| resource.id == id))
    else {
        return Ok(None);
    };
    if cover.media_type.eq_ignore_ascii_case("image/png") {
        let (jpeg, thumbnail) =
            normalize_small_cover_png(&cover.data, encode_thumbnail).map_err(|error| {
                Error::Output(format!("PNG cover JPEG normalization failed: {error}"))
            })?;
        cover.data = jpeg;
        cover.media_type = "image/jpeg".to_owned();
        return Ok(thumbnail);
    }
    if cover.media_type.eq_ignore_ascii_case("image/jpeg") {
        return Ok(decode_any_image(&cover.data)
            .ok()
            .and_then(|image| encode_thumbnail(&image)));
    }
    Ok(None)
}

fn decode_any_image(data: &[u8]) -> Result<DynamicImage> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|error| Error::Output(format!("cover format detection failed: {error}")))?;
    if !matches!(reader.format(), Some(ImageFormat::Jpeg | ImageFormat::Png)) {
        return Err(Error::Output("cover data is not JPEG or PNG".to_owned()));
    }
    reader
        .decode()
        .map_err(|error| Error::Output(format!("cover decode failed: {error}")))
}

fn encode_thumbnail(image: &DynamicImage) -> Option<Vec<u8>> {
    let (width, height) = thumbnail_dimensions(image.width(), image.height());
    let resized = image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
    let rgb = flatten_thumbnail_on_white(&resized);
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, LIBRARY_THUMBNAIL_JPEG_QUALITY)
        .encode_image(&DynamicImage::ImageRgb8(rgb))
        .ok()?;
    Some(encoded)
}

fn flatten_thumbnail_on_white(image: &DynamicImage) -> RgbImage {
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

fn thumbnail_dimensions(width: u32, height: u32) -> (u32, u32) {
    if width <= LIBRARY_THUMBNAIL_MAX_WIDTH && height <= LIBRARY_THUMBNAIL_MAX_HEIGHT {
        return (width.max(1), height.max(1));
    }
    let width64 = u64::from(width);
    let height64 = u64::from(height);
    let max_width64 = u64::from(LIBRARY_THUMBNAIL_MAX_WIDTH);
    let max_height64 = u64::from(LIBRARY_THUMBNAIL_MAX_HEIGHT);
    if width64 * max_height64 > height64 * max_width64 {
        (
            LIBRARY_THUMBNAIL_MAX_WIDTH,
            (height64 * max_width64 / width64).max(1) as u32,
        )
    } else {
        (
            (width64 * max_height64 / height64).max(1) as u32,
            LIBRARY_THUMBNAIL_MAX_HEIGHT,
        )
    }
}
