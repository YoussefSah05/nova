//! `nova-art://` — the webview's route to the artwork cache.
//!
//! The frontend turns a provider/TMDB/EPG image URL into
//! `convertFileSrc(url, "nova-art")` (`nova-art://localhost/<encoded url>` on
//! macOS, `http://nova-art.localhost/<encoded url>` on Windows). The handler
//! serves the validated body from `nova_artwork::ArtworkCache`, downloading it
//! on first use; anything unavailable or not an image is a 404, and the UI then
//! tries the original URL once before showing its placeholder.

use std::sync::Arc;

use nova_artwork::ArtworkCache;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{AppHandle, Manager, UriSchemeResponder};

pub const SCHEME: &str = "nova-art";

/// The original image URL carried in a `nova-art` request path
/// (`/` + `encodeURIComponent(url)`).
pub fn decode_request_path(path: &str) -> Option<String> {
    let encoded = path.strip_prefix('/').unwrap_or(path);
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = encoded.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    let url = String::from_utf8(out).ok()?;
    (!url.is_empty()).then_some(url)
}

fn response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        // Images never run code; an SVG opened on its own must not either.
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'",
        )
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// Protocol entry point: resolve off the IPC thread and answer when ready.
pub fn handle(app: AppHandle, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let url = decode_request_path(request.uri().path());
    tauri::async_runtime::spawn(async move {
        let not_found = || response(StatusCode::NOT_FOUND, "text/plain", Vec::new());
        let Some(url) = url else {
            responder.respond(not_found());
            return;
        };
        let Some(cache) = app
            .try_state::<Arc<ArtworkCache>>()
            .map(|s| s.inner().clone())
        else {
            responder.respond(not_found());
            return;
        };
        match cache.get(&url).await {
            Ok(image) => {
                responder.respond(response(StatusCode::OK, image.format.mime(), image.bytes))
            }
            Err(e) => {
                tracing::debug!(error = %e, "artwork unavailable");
                responder.respond(not_found());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_encode_uri_component_paths() {
        assert_eq!(
            decode_request_path("/https%3A%2F%2Fimage.tmdb.org%2Ft%2Fp%2Fw500%2Fa.jpg").as_deref(),
            Some("https://image.tmdb.org/t/p/w500/a.jpg")
        );
        assert_eq!(
            decode_request_path("/http%3A%2F%2Fx.tv%2Flogo%3Fid%3D5%26w%3D300").as_deref(),
            Some("http://x.tv/logo?id=5&w=300")
        );
        assert_eq!(
            decode_request_path("/http%3A%2F%2Fx.tv%2F%C3%A9t%C3%A9.png").as_deref(),
            Some("http://x.tv/été.png")
        );
    }

    #[test]
    fn rejects_malformed_paths() {
        assert_eq!(decode_request_path("/"), None);
        assert_eq!(decode_request_path("/%zz"), None);
        assert_eq!(decode_request_path("/abc%2"), None);
        assert_eq!(decode_request_path("/%FF%FE"), None);
    }
}
