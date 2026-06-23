use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use nasfiles_core::models::ImageInfo;

use crate::thumb::cache::ThumbnailCache;

const MAX_EXIF_STRING_LEN: usize = 256;

pub struct ImageInfoProbeRequest<'a> {
    pub cache_dir: &'a Path,
    pub source_path: &'a Path,
    pub root_kind: &'a str,
    pub root_key: &'a str,
    pub relative_path: &'a str,
    pub max_image_width: u32,
    pub max_image_height: u32,
    pub max_alloc: u64,
}

pub async fn get_or_probe(
    request: ImageInfoProbeRequest<'_>,
) -> Result<Option<ImageInfo>, ImageInfoError> {
    if !is_image(request.source_path) {
        return Ok(None);
    }

    let metadata = tokio::fs::metadata(request.source_path)
        .await
        .map_err(|e| ImageInfoError::Io(e.to_string()))?;

    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let cache_root_kind = format!("image-info-v1:{}", request.root_kind);
    let key = ThumbnailCache::cache_key(
        &cache_root_kind,
        request.root_key,
        request.relative_path,
        mtime_ms,
        metadata.len(),
    );
    let cache_path = cache_path(request.cache_dir, &key);

    if cache_path.exists() {
        let bytes = tokio::fs::read(&cache_path)
            .await
            .map_err(|e| ImageInfoError::Io(e.to_string()))?;
        let info =
            serde_json::from_slice(&bytes).map_err(|e| ImageInfoError::Parse(e.to_string()))?;
        return Ok(Some(info));
    }

    let source = request.source_path.to_path_buf();
    let max_image_width = request.max_image_width;
    let max_image_height = request.max_image_height;
    let max_alloc = request.max_alloc;
    let info = tokio::task::spawn_blocking(move || {
        probe(&source, max_image_width, max_image_height, max_alloc)
    })
    .await
    .map_err(|e| ImageInfoError::Image(format!("task join error: {e}")))??;

    if let Some(parent) = cache_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ImageInfoError::Io(e.to_string()))?;
    }

    let bytes = serde_json::to_vec(&info).map_err(|e| ImageInfoError::Parse(e.to_string()))?;
    tokio::fs::write(cache_path, bytes)
        .await
        .map_err(|e| ImageInfoError::Io(e.to_string()))?;

    Ok(Some(info))
}

fn cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir
        .join("image-info")
        .join(&key[0..2])
        .join(&key[2..4])
        .join(format!("{key}.json"))
}

fn is_image(path: &Path) -> bool {
    mime_guess::from_path(path)
        .first()
        .is_some_and(|mime| mime.essence_str().starts_with("image/"))
}

fn probe(
    source_path: &Path,
    max_image_width: u32,
    max_image_height: u32,
    max_alloc: u64,
) -> Result<ImageInfo, ImageInfoError> {
    let mut reader = ::image::ImageReader::open(source_path)
        .map_err(|e| ImageInfoError::Image(e.to_string()))?;
    let mut limits = ::image::Limits::default();
    limits.max_image_width = Some(max_image_width);
    limits.max_image_height = Some(max_image_height);
    limits.max_alloc = Some(max_alloc);
    reader.limits(limits);

    let format = reader.format().map(|format| format!("{format:?}"));
    let image = reader
        .decode()
        .map_err(|e| ImageInfoError::Image(e.to_string()))?;
    let exif = read_jpeg_exif(source_path).unwrap_or_default();

    Ok(ImageInfo {
        width: image.width(),
        height: image.height(),
        format,
        has_alpha: image.color().has_alpha(),
        exif,
    })
}

fn read_jpeg_exif(source_path: &Path) -> Result<BTreeMap<String, String>, ImageInfoError> {
    let mut file =
        std::fs::File::open(source_path).map_err(|e| ImageInfoError::Io(e.to_string()))?;
    let mut marker = [0u8; 2];
    file.read_exact(&mut marker)
        .map_err(|e| ImageInfoError::Io(e.to_string()))?;
    if marker != [0xff, 0xd8] {
        return Ok(BTreeMap::new());
    }

    loop {
        file.read_exact(&mut marker)
            .map_err(|e| ImageInfoError::Io(e.to_string()))?;
        while marker[0] != 0xff {
            marker[0] = marker[1];
            file.read_exact(&mut marker[1..2])
                .map_err(|e| ImageInfoError::Io(e.to_string()))?;
        }

        if marker[1] == 0xda || marker[1] == 0xd9 {
            return Ok(BTreeMap::new());
        }

        let mut len_buf = [0u8; 2];
        file.read_exact(&mut len_buf)
            .map_err(|e| ImageInfoError::Io(e.to_string()))?;
        let segment_len = u16::from_be_bytes(len_buf) as usize;
        if segment_len < 2 {
            return Ok(BTreeMap::new());
        }

        let payload_len = segment_len - 2;
        let mut payload = vec![0u8; payload_len];
        file.read_exact(&mut payload)
            .map_err(|e| ImageInfoError::Io(e.to_string()))?;

        if marker[1] == 0xe1 && payload.starts_with(b"Exif\0\0") {
            return Ok(parse_tiff_exif(&payload[6..]));
        }
    }
}

fn parse_tiff_exif(data: &[u8]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if data.len() < 8 {
        return out;
    }

    let le = match &data[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return out,
    };
    if read_u16(data, 2, le) != Some(42) {
        return out;
    }

    if let Some(ifd0_offset) = read_u32(data, 4, le).and_then(|offset| usize::try_from(offset).ok())
    {
        parse_ifd(data, ifd0_offset, le, 0, &mut out);
    }

    out
}

/// Maximum IFD nesting we will follow. A well-formed EXIF blob only nests one
/// level (IFD0 → Exif sub-IFD via tag 0x8769); a small cap leaves slack while
/// preventing a maliciously chained or self-referential 0x8769 offset from
/// driving unbounded recursion (which would overflow the stack and abort the
/// whole process — a remotely triggerable DoS on the image-info/preview path).
const MAX_EXIF_IFD_DEPTH: u8 = 4;

fn parse_ifd(data: &[u8], offset: usize, le: bool, depth: u8, out: &mut BTreeMap<String, String>) {
    let Some(count) = read_u16(data, offset, le).map(usize::from) else {
        return;
    };

    let entries_start = offset.saturating_add(2);
    for idx in 0..count {
        let entry_offset = entries_start.saturating_add(idx.saturating_mul(12));
        if entry_offset.saturating_add(12) > data.len() {
            break;
        }

        let tag = read_u16(data, entry_offset, le).unwrap_or_default();
        let field_type = read_u16(data, entry_offset + 2, le).unwrap_or_default();
        let count = read_u32(data, entry_offset + 4, le).unwrap_or_default();
        let value_or_offset = &data[entry_offset + 8..entry_offset + 12];

        if tag == 0x8769 {
            if depth >= MAX_EXIF_IFD_DEPTH {
                continue;
            }
            let Some(exif_offset) =
                read_u32(value_or_offset, 0, le).and_then(|value| usize::try_from(value).ok())
            else {
                continue;
            };
            parse_ifd(data, exif_offset, le, depth + 1, out);
            continue;
        }

        let Some(name) = exif_tag_name(tag) else {
            continue;
        };

        if let Some(value) = exif_value(data, field_type, count, value_or_offset, le) {
            out.insert(name.to_string(), value);
        }
    }
}

fn exif_value(
    data: &[u8],
    field_type: u16,
    count: u32,
    value_or_offset: &[u8],
    le: bool,
) -> Option<String> {
    match field_type {
        2 => {
            let count = usize::try_from(count).ok()?;
            let bytes = if count <= 4 {
                &value_or_offset[..count]
            } else {
                let offset = usize::try_from(read_u32(value_or_offset, 0, le)?).ok()?;
                data.get(offset..offset.saturating_add(count))?
            };
            let value = std::str::from_utf8(bytes).ok()?.trim_matches('\0').trim();
            (!value.is_empty()).then(|| truncate_exif_value(value))
        }
        3 => {
            if count == 1 {
                read_u16(value_or_offset, 0, le).map(|value| value.to_string())
            } else {
                None
            }
        }
        4 => {
            if count == 1 {
                read_u32(value_or_offset, 0, le).map(|value| value.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn exif_tag_name(tag: u16) -> Option<&'static str> {
    match tag {
        0x010f => Some("Make"),
        0x0110 => Some("Model"),
        0x0112 => Some("Orientation"),
        0x0131 => Some("Software"),
        0x0132 => Some("DateTime"),
        0x829a => Some("ExposureTime"),
        0x829d => Some("FNumber"),
        0x8827 => Some("ISO"),
        0x9003 => Some("DateTimeOriginal"),
        0xa002 => Some("PixelXDimension"),
        0xa003 => Some("PixelYDimension"),
        _ => None,
    }
}

fn truncate_exif_value(value: &str) -> String {
    value.chars().take(MAX_EXIF_STRING_LEN).collect()
}

fn read_u16(data: &[u8], offset: usize, le: bool) -> Option<u16> {
    let bytes: [u8; 2] = data.get(offset..offset + 2)?.try_into().ok()?;
    Some(if le {
        u16::from_le_bytes(bytes)
    } else {
        u16::from_be_bytes(bytes)
    })
}

fn read_u32(data: &[u8], offset: usize, le: bool) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    Some(if le {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ImageInfoError {
    #[error("io error: {0}")]
    Io(String),
    #[error("image processing error: {0}")]
    Image(String),
    #[error("parse error: {0}")]
    Parse(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_referential_exif_ifd_terminates() {
        // Minimal little-endian TIFF whose IFD0 holds a single Exif-IFD pointer
        // (tag 0x8769) pointing back at IFD0 itself. Without the recursion depth
        // cap this recurses forever and overflows the stack, aborting the whole
        // process — a remotely triggerable DoS. With the cap, parsing must
        // terminate. (If the bug regressed, this test would crash the runner
        // rather than fail.)
        let mut data = vec![0u8; 22];
        data[0] = b'I'; // little-endian byte order
        data[1] = b'I';
        data[2] = 42; // TIFF magic (u16 LE)
        data[4] = 8; // IFD0 offset = 8 (u32 LE)
        data[8] = 1; // IFD0 entry count = 1 (u16 LE)
        // Entry at offset 10:
        data[10] = 0x69; // tag 0x8769 (u16 LE) — Exif sub-IFD pointer
        data[11] = 0x87;
        data[12] = 4; // field type LONG (u16 LE)
        data[14] = 1; // value count = 1 (u32 LE)
        data[18] = 8; // pointer value = 8, i.e. back to IFD0 (u32 LE)

        let out = parse_tiff_exif(&data);
        assert!(out.is_empty());
    }
}
