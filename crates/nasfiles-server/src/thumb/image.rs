use std::io::Cursor;
use std::path::Path;

use super::cache::ThumbError;
use super::cache::ThumbFormat;

/// Generate a JPEG thumbnail from an image file.
///
/// Decodes the image, resizes to `width` px (preserving aspect ratio),
/// and encodes as JPEG quality 80.
///
/// Runs in `spawn_blocking` to avoid blocking the async runtime.
pub async fn generate(
    source_path: &Path,
    width: u32,
    format: ThumbFormat,
    max_image_width: u32,
    max_image_height: u32,
    max_alloc: u64,
) -> Result<Option<Vec<u8>>, ThumbError> {
    let path = source_path.to_path_buf();

    let result = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ThumbError> {
        let mut reader =
            ::image::ImageReader::open(&path).map_err(|e| ThumbError::Image(e.to_string()))?;
        let mut limits = ::image::Limits::default();
        limits.max_image_width = Some(max_image_width);
        limits.max_image_height = Some(max_image_height);
        limits.max_alloc = Some(max_alloc);
        reader.limits(limits);
        let img = reader
            .decode()
            .map_err(|e| ThumbError::Image(e.to_string()))?;

        resize_and_encode(img, width, format)
    })
    .await
    .map_err(|e| ThumbError::Image(format!("task join error: {e}")))?;

    result.map(Some)
}

pub async fn generate_from_bytes(
    bytes: Vec<u8>,
    width: u32,
    max_image_width: u32,
    max_image_height: u32,
    max_alloc: u64,
) -> Result<Option<Vec<u8>>, ThumbError> {
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ThumbError> {
        let cursor = Cursor::new(bytes);
        let mut reader = ::image::ImageReader::new(cursor)
            .with_guessed_format()
            .map_err(|e| ThumbError::Image(format!("failed to guess image format: {e}")))?;
        let mut limits = ::image::Limits::default();
        limits.max_image_width = Some(max_image_width);
        limits.max_image_height = Some(max_image_height);
        limits.max_alloc = Some(max_alloc);
        reader.limits(limits);
        let img = reader
            .decode()
            .map_err(|e| ThumbError::Image(e.to_string()))?;

        resize_and_encode(img, width, ThumbFormat::Jpeg)
    })
    .await
    .map_err(|e| ThumbError::Image(format!("task join error: {e}")))?;

    result.map(Some)
}

fn resize_and_encode(
    img: ::image::DynamicImage,
    width: u32,
    format: ThumbFormat,
) -> Result<Vec<u8>, ThumbError> {
    let thumb = img.thumbnail(width, width);
    let mut buf = Cursor::new(Vec::new());
    let output = match format {
        ThumbFormat::Jpeg => ::image::DynamicImage::ImageRgb8(thumb.to_rgb8()),
        ThumbFormat::Png => ::image::DynamicImage::ImageRgba8(thumb.to_rgba8()),
    };
    let image_format = match format {
        ThumbFormat::Jpeg => ::image::ImageFormat::Jpeg,
        ThumbFormat::Png => ::image::ImageFormat::Png,
    };
    output
        .write_to(&mut buf, image_format)
        .map_err(|e| ThumbError::Image(e.to_string()))?;
    Ok(buf.into_inner())
}
