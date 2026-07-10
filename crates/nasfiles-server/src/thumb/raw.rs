use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

use super::{cache::ThumbError, image as img_thumb};

pub async fn generate(
    source_path: &Path,
    width: u32,
    max_image_width: u32,
    max_image_height: u32,
    max_alloc: u64,
) -> Result<Option<Vec<u8>>, ThumbError> {
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        if let Some(bytes) = run_dcraw(source_path, &["-e", "-c"]).await? {
            match resize_preview(bytes, width, max_image_width, max_image_height, max_alloc).await {
                Ok(thumb) => return Ok(thumb),
                Err(e) => {
                    tracing::warn!(
                        "embedded RAW thumbnail decode failed for {}: {e}",
                        source_path.display()
                    );
                }
            }
        }

        let Some(bytes) = run_dcraw(source_path, &["-w", "-c", "-h"]).await? else {
            return Ok(None);
        };
        resize_preview(bytes, width, max_image_width, max_image_height, max_alloc).await
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_) => {
            tracing::warn!(
                "dcraw_emu thumbnail timed out for {}",
                source_path.display()
            );
            Ok(None)
        }
    }
}

async fn run_dcraw(source_path: &Path, args: &[&str]) -> Result<Option<Vec<u8>>, ThumbError> {
    let output = Command::new("dcraw_emu")
        .args(args)
        .arg(source_path)
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => Ok(Some(out.stdout)),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::warn!(
                "dcraw_emu thumbnail failed for {}: args={:?} status={} stderr={}",
                source_path.display(),
                args,
                out.status,
                stderr.trim()
            );
            Ok(None)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ThumbError::Image(e.to_string())),
    }
}

async fn resize_preview(
    bytes: Vec<u8>,
    width: u32,
    max_image_width: u32,
    max_image_height: u32,
    max_alloc: u64,
) -> Result<Option<Vec<u8>>, ThumbError> {
    img_thumb::generate_from_bytes(bytes, width, max_image_width, max_image_height, max_alloc).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_raw_returns_no_thumbnail() {
        let path = Path::new("/definitely/missing.arw");
        let result = generate(path, 480, 20_000, 20_000, 268_435_456).await;
        assert!(result.is_ok() || matches!(result, Err(ThumbError::Image(_))));
    }
}
