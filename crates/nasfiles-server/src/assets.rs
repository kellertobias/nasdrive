use axum::{
    body::Body,
    extract::State,
    http::{StatusCode, Uri, header},
    response::Response,
};

use crate::state::AppState;

/// Embedded frontend assets from the Vite build output.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist/"]
struct Assets;

/// Raw bytes of the bundled app icon, for rasterizing a static social-preview
/// image (see `api::og`).
pub fn favicon_svg_bytes() -> Option<Vec<u8>> {
    Assets::get("favicon.svg").map(|f| f.data.into_owned())
}

/// Serve embedded static assets with SPA fallback.
/// - If the requested path exists as an embedded file, serve it.
/// - Otherwise, serve index.html for client-side routing, with Open Graph /
///   Twitter Card metadata filled in so pasting a link into Teams, WhatsApp,
///   etc. shows a useful preview.
pub async fn static_handler(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try to find the exact file
    if let Some(file) = Assets::get(path) {
        let content_type = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, cache_control(path))
            .body(Body::from(file.data.into_owned()))
            .unwrap();
    }

    // SPA fallback: serve index.html
    match Assets::get("index.html") {
        Some(index) => {
            let template = String::from_utf8_lossy(&index.data).into_owned();
            let html = og::render(&template, og::build_context(&state, uri.path()).await);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(html))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(
                "frontend not built — run `npm run build` in web/",
            ))
            .unwrap(),
    }
}

/// Cache control based on file type.
/// Hashed assets (Vite output) get long-lived caching. HTML is never cached.
fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        // Vite content-hashed assets
        "public, max-age=31536000, immutable"
    } else if path.ends_with(".html") {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

/// Open Graph / Twitter Card metadata for the SPA shell, computed per-request
/// so pasting a share link into Teams, WhatsApp, etc. shows a useful preview
/// instead of generic app boilerplate.
mod og {
    use crate::shares::access;
    use crate::state::AppState;

    pub struct Context {
        pub title: String,
        pub description: String,
        pub image: String,
        pub url: String,
    }

    /// Substitute the `__OG_*__` placeholders in `index.html` with the given
    /// context. Values are HTML-escaped since they land inside both a text
    /// node (`<title>`) and attribute values (`content="..."`).
    pub fn render(template: &str, ctx: Context) -> String {
        template
            .replace("__OG_TITLE__", &escape(&ctx.title))
            .replace("__OG_DESCRIPTION__", &escape(&ctx.description))
            .replace("__OG_IMAGE__", &escape(&ctx.image))
            .replace("__OG_URL__", &escape(&ctx.url))
    }

    pub async fn build_context(state: &AppState, path: &str) -> Context {
        let base = state.config.base_url.trim_end_matches('/');
        let url = format!("{base}{path}");
        let app_icon = format!("{base}/api/public/og/app-icon.jpg");

        let Some(token) = share_token(path) else {
            return not_a_share(app_icon, url);
        };

        let Ok(share) = access::resolve_share_unchecked(&state.pool, token).await else {
            return not_a_share(app_icon, url);
        };

        let now = chrono::Utc::now().timestamp_millis();
        let is_expired =
            share.revoked_at.is_some() || share.expires_at.is_some_and(|exp| now > exp);

        if is_expired {
            let description = match share.expires_at {
                Some(ms) => format!(
                    "Share expired {}. Please ask the author to extend it or send a new link.",
                    format_expiry(ms)
                ),
                None => "This share is no longer available. Please ask the author to send a new link."
                    .to_string(),
            };
            return Context {
                title: "Share is expired".to_string(),
                description,
                image: app_icon,
                url,
            };
        }

        if share.password_hash.is_some() {
            let description = match share.expires_at {
                Some(ms) => format!(
                    "This share is password protected and expires {}.",
                    format_expiry(ms)
                ),
                None => "This share is password protected.".to_string(),
            };
            return Context {
                title: "Shared with you".to_string(),
                description,
                image: app_icon,
                url,
            };
        }

        let name = share.display_name();
        let description = match share.expires_at {
            Some(ms) => format!("{name} · Expires {}", format_expiry(ms)),
            None => format!("{name} · Never expires"),
        };
        Context {
            title: "Shared with You".to_string(),
            description,
            image: format!("{base}/api/public/shares/{token}/og-image"),
            url,
        }
    }

    fn not_a_share(app_icon: String, url: String) -> Context {
        Context {
            title: "NASDrive - Your Self Hosted Drive".to_string(),
            description: "Log in to see the files.".to_string(),
            image: app_icon,
            url,
        }
    }

    /// Extract the token from a `/s/{token}(/...)` share-viewer URL.
    fn share_token(path: &str) -> Option<&str> {
        let mut segments = path.trim_start_matches('/').splitn(3, '/');
        if segments.next()? != "s" {
            return None;
        }
        let token = segments.next()?;
        (!token.is_empty()).then_some(token)
    }

    fn format_expiry(timestamp_ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(timestamp_ms)
            .map(|dt| dt.format("%b %-d, %Y at %H:%M UTC").to_string())
            .unwrap_or_else(|| "soon".to_string())
    }

    fn escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn share_token_extracts_token_from_share_route() {
            assert_eq!(share_token("/s/abc123"), Some("abc123"));
            assert_eq!(share_token("/s/abc123/"), Some("abc123"));
            assert_eq!(share_token("/s/abc123/nested/path.txt"), Some("abc123"));
        }

        #[test]
        fn share_token_rejects_non_share_routes() {
            assert_eq!(share_token("/"), None);
            assert_eq!(share_token("/login"), None);
            assert_eq!(share_token("/s"), None);
            assert_eq!(share_token("/s/"), None);
            assert_eq!(share_token("/ss/abc123"), None);
        }

        #[test]
        fn escape_neutralizes_html_metacharacters() {
            assert_eq!(
                escape(r#"<script>&"pwned"</script>"#),
                "&lt;script&gt;&amp;&quot;pwned&quot;&lt;/script&gt;"
            );
        }

        #[test]
        fn format_expiry_renders_a_human_readable_utc_timestamp() {
            // 2026-01-15T09:05:00Z
            let ms = 1768467900000;
            assert_eq!(format_expiry(ms), "Jan 15, 2026 at 09:05 UTC");
        }

        #[test]
        fn render_substitutes_all_placeholders_and_escapes_values() {
            let template = "<title>__OG_TITLE__</title><meta content=\"__OG_DESCRIPTION__\"><meta content=\"__OG_IMAGE__\"><meta content=\"__OG_URL__\">";
            let ctx = Context {
                title: "A & B".to_string(),
                description: "quote \" test".to_string(),
                image: "https://example.com/i.jpg".to_string(),
                url: "https://example.com/s/tok".to_string(),
            };
            let html = render(template, ctx);
            assert!(html.contains("<title>A &amp; B</title>"));
            assert!(html.contains("content=\"quote &quot; test\""));
            assert!(html.contains("content=\"https://example.com/i.jpg\""));
            assert!(html.contains("content=\"https://example.com/s/tok\""));
            assert!(!html.contains("__OG_"));
        }
    }
}
