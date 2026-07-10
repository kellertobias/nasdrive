use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThumbnailKind {
    Image,
    Svg,
    Video,
    Audio,
    Pdf,
    Text,
    Epub,
    Raw,
}

impl ThumbnailKind {
    pub fn as_key(self) -> &'static str {
        match self {
            ThumbnailKind::Image => "image",
            ThumbnailKind::Svg => "svg",
            ThumbnailKind::Video => "video",
            ThumbnailKind::Audio => "audio",
            ThumbnailKind::Pdf => "pdf",
            ThumbnailKind::Text => "text",
            ThumbnailKind::Epub => "epub",
            ThumbnailKind::Raw => "raw",
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_text_filename)
        {
            return Some(Self::Text);
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        Self::from_extension(&ext)
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "tif" => Some(Self::Image),
            "arw" => Some(Self::Raw),
            "svg" => Some(Self::Svg),
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v" | "wmv" | "flv" => Some(Self::Video),
            "mp3" | "ogg" | "flac" | "aac" | "wav" | "m4a" | "wma" | "m4b" => Some(Self::Audio),
            "pdf" => Some(Self::Pdf),
            "epub" => Some(Self::Epub),
            ext if is_text_extension(ext) => Some(Self::Text),
            _ => None,
        }
    }
}

pub fn supports_thumbnail_path(path: &Path, thumbnails_enabled: bool) -> bool {
    thumbnails_enabled && ThumbnailKind::from_path(path).is_some()
}

fn is_text_filename(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "dockerfile" | "makefile" | "readme" | "license"
    )
}

fn is_text_extension(ext: &str) -> bool {
    matches!(
        ext,
        "txt"
            | "md"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "csv"
            | "log"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "rs"
            | "go"
            | "java"
            | "c"
            | "cpp"
            | "h"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "css"
            | "scss"
            | "html"
            | "htm"
            | "sql"
            | "rb"
            | "php"
            | "swift"
            | "kt"
            | "lua"
            | "r"
            | "pl"
            | "conf"
            | "ini"
            | "env"
            | "vtt"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_thumbnail_kinds() {
        assert_eq!(
            ThumbnailKind::from_extension("jpg"),
            Some(ThumbnailKind::Image)
        );
        assert_eq!(
            ThumbnailKind::from_extension("m4v"),
            Some(ThumbnailKind::Video)
        );
        assert_eq!(
            ThumbnailKind::from_extension("mp3"),
            Some(ThumbnailKind::Audio)
        );
        assert_eq!(
            ThumbnailKind::from_extension("pdf"),
            Some(ThumbnailKind::Pdf)
        );
        assert_eq!(
            ThumbnailKind::from_extension("rs"),
            Some(ThumbnailKind::Text)
        );
        assert_eq!(
            ThumbnailKind::from_extension("epub"),
            Some(ThumbnailKind::Epub)
        );
        assert_eq!(
            ThumbnailKind::from_extension("arw"),
            Some(ThumbnailKind::Raw)
        );
        assert_eq!(ThumbnailKind::from_extension("zip"), None);
        assert_eq!(
            ThumbnailKind::from_path(Path::new("Makefile")),
            Some(ThumbnailKind::Text)
        );
    }
}
