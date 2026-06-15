pub mod archive;
pub mod file_jobs;
pub mod image_info;
pub mod listing;
pub mod media_info;
pub mod ops;
pub mod preview;
pub mod roots;
pub mod stream;
pub mod zip;

/// Sanitize a string for use as a filename in a "filename=\"...\"" header.
/// Replaces non-ASCII characters and special characters with underscores.
pub fn sanitize_header_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
