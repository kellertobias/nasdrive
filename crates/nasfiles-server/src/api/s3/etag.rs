use std::path::Path;

// @tour comment ETags read the whole file, and are not S3-shaped
// This reads the entire file into memory for every ETag, and the listing code calls it once
// per file — so a ListObjectsV2 over a large root reads every byte it lists.
//
// On error it falls back to hashing the path string, so an unreadable file still yields a
// plausible-looking ETag. And real S3 marks multipart objects with a `-{partcount}` suffix,
// which this does not produce, so clients that validate that format will disagree.

/// Compute MD5 ETag for a file, returned as a hex string.
/// Returns a random-looking fallback if the file can't be read.
pub async fn compute_etag(path: &Path) -> String {
    match tokio::fs::read(path).await {
        Ok(data) => format!("{:x}", md5::compute(&data)),
        Err(_) => {
            // Fall back to a deterministic placeholder based on path
            let placeholder = format!("{}", path.display());
            format!("{:x}", md5::compute(placeholder.as_bytes()))
        }
    }
}
