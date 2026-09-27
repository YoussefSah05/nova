//! Content validation for downloaded artwork.
//!
//! A URL's extension or a server's `Content-Type` says nothing reliable about
//! the bytes (IPTV panels answer dead logos with HTML error pages, 1×1
//! placeholders or JSON). Every body is identified by its magic bytes, bounded
//! in size and checked for sane dimensions before it is cached or served.

/// Largest artwork body accepted, in bytes.
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
/// Smallest accepted side, in pixels: rejects tracking pixels and 1×1
/// "no image" placeholders.
pub const MIN_SIDE: usize = 8;
/// Largest accepted side, in pixels.
pub const MAX_SIDE: usize = 12_000;

/// An image format the webview can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Avif,
    Bmp,
    Ico,
    Svg,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
            Self::Bmp => "image/bmp",
            Self::Ico => "image/x-icon",
            Self::Svg => "image/svg+xml",
        }
    }
}

/// Why a body was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    Empty,
    TooLarge,
    NotAnImage,
    Unreadable,
    BadDimensions { width: usize, height: usize },
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty body"),
            Self::TooLarge => write!(f, "larger than {MAX_BYTES} bytes"),
            Self::NotAnImage => write!(f, "not an image"),
            Self::Unreadable => write!(f, "image header unreadable"),
            Self::BadDimensions { width, height } => {
                write!(f, "unusable dimensions {width}x{height}")
            }
        }
    }
}

/// A body that passed validation. Dimensions are unknown for SVG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Validated {
    pub format: ImageFormat,
    pub width: Option<usize>,
    pub height: Option<usize>,
}

/// Identify the image format from the leading bytes.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(ImageFormat::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ImageFormat::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(ImageFormat::Gif);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(ImageFormat::Webp);
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis") {
        return Some(ImageFormat::Avif);
    }
    if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        return Some(ImageFormat::Bmp);
    }
    if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() >= 6 {
        return Some(ImageFormat::Ico);
    }
    if is_svg(bytes) {
        return Some(ImageFormat::Svg);
    }
    None
}

/// An XML document whose root element is `<svg>` (within the first KiB, after
/// an optional BOM, XML declaration, comments or doctype) and that is not an
/// HTML page.
fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let Ok(text) = std::str::from_utf8(head).or_else(|e| {
        // A multi-byte character cut at the 1 KiB boundary is fine.
        std::str::from_utf8(&head[..e.valid_up_to()])
    }) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let lower = text.to_ascii_lowercase();
    let xml_start = ["<svg", "<?xml", "<!--", "<!doctype svg"];
    if !xml_start.iter().any(|p| lower.starts_with(p)) {
        return false;
    }
    lower.contains("<svg") && !lower.contains("<html")
}

/// Validate a downloaded body.
pub fn validate(bytes: &[u8]) -> Result<Validated, Rejection> {
    if bytes.is_empty() {
        return Err(Rejection::Empty);
    }
    if bytes.len() > MAX_BYTES {
        return Err(Rejection::TooLarge);
    }
    let format = sniff(bytes).ok_or(Rejection::NotAnImage)?;
    if format == ImageFormat::Svg {
        return Ok(Validated {
            format,
            width: None,
            height: None,
        });
    }
    let size = imagesize::blob_size(bytes).map_err(|_| Rejection::Unreadable)?;
    let (width, height) = (size.width, size.height);
    if width < MIN_SIDE || height < MIN_SIDE || width > MAX_SIDE || height > MAX_SIDE {
        return Err(Rejection::BadDimensions { width, height });
    }
    Ok(Validated {
        format,
        width: Some(width),
        height: Some(height),
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// A minimal PNG header (signature + IHDR) of the given size. Enough for
    /// sniffing and dimension probing.
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b.extend_from_slice(&[0, 0, 0, 0]); // CRC (not checked)
        b
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::png;
    use super::*;

    #[test]
    fn accepts_real_images_with_dimensions() {
        let v = validate(&png(300, 200)).unwrap();
        assert_eq!(v.format, ImageFormat::Png);
        assert_eq!((v.width, v.height), (Some(300), Some(200)));
    }

    #[test]
    fn sniffs_formats_by_magic_bytes() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageFormat::Jpeg));
        assert_eq!(sniff(b"GIF89a......"), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(sniff(b"\0\0\0\x1cftypavif\0\0"), Some(ImageFormat::Avif));
        assert_eq!(
            sniff(br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#),
            Some(ImageFormat::Svg)
        );
        assert_eq!(
            sniff(b"\xef\xbb\xbf  <svg viewBox='0 0 1 1'/>"),
            Some(ImageFormat::Svg)
        );
    }

    #[test]
    fn rejects_pages_placeholders_and_oversize_bodies() {
        assert_eq!(validate(b""), Err(Rejection::Empty));
        assert_eq!(
            validate(b"<!DOCTYPE html><html><body>404</body></html>"),
            Err(Rejection::NotAnImage)
        );
        assert_eq!(
            validate(br#"{"error":"not found"}"#),
            Err(Rejection::NotAnImage)
        );
        assert_eq!(
            validate(b"<?xml version='1.0'?><html><svg/></html>"),
            Err(Rejection::NotAnImage)
        );
        assert_eq!(
            validate(&png(1, 1)),
            Err(Rejection::BadDimensions {
                width: 1,
                height: 1
            })
        );
        assert_eq!(validate(b"\x89PNG\r\n\x1a\n"), Err(Rejection::Unreadable));
        let mut huge = png(100, 100);
        huge.resize(MAX_BYTES + 1, 0);
        assert_eq!(validate(&huge), Err(Rejection::TooLarge));
    }
}
