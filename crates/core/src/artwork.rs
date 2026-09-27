//! Artwork URL policy shared by every importer.
//!
//! Providers ship logos and posters in many shapes: plain `.png` files, but
//! also extension-less CDN and resize endpoints (`/logo/1234`,
//! `/image?id=5&w=300`). Rejecting everything without an image extension threw
//! those valid logos away. The rule here is permissive about *form* and strict
//! about *obvious garbage*: an absolute http(s) URL with a host and a path that
//! names something, which is not a known-incomplete base URL and not a media or
//! playlist file. The final say is the image loader (a broken image falls back
//! to a placeholder in the UI).

const IMAGE_EXTENSIONS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".webp", ".gif", ".svg", ".avif", ".bmp", ".ico",
];

/// Extensions that are certainly not images (streams, playlists, pages).
const NON_IMAGE_EXTENSIONS: &[&str] = &[
    ".m3u", ".m3u8", ".ts", ".mp4", ".mkv", ".avi", ".mov", ".mpd", ".xml", ".txt", ".html",
    ".htm", ".php",
];

/// The trimmed URL when it plausibly points at an image, else `None`.
pub fn plausible_image_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    // Some providers ship unencoded spaces in file names; encode them rather
    // than lose the logo. Any other whitespace means a broken value.
    if trimmed.chars().any(|c| c.is_whitespace() && c != ' ') {
        return None;
    }
    let url = trimmed.replace(' ', "%20");
    let scheme_len = match url.get(..8).map(str::to_ascii_lowercase) {
        Some(p) if p.starts_with("https://") => 8,
        Some(p) if p.starts_with("http://") => 7,
        _ => return None,
    };
    let rest = &url[scheme_len..];
    let (host, path_and_query) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if host.is_empty() {
        return None;
    }
    let path = path_and_query
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let has_query = path_and_query.contains('?');
    let last = path.rsplit('/').next().unwrap_or("");
    let lower_last = last.to_ascii_lowercase();

    if IMAGE_EXTENSIONS.iter().any(|ext| lower_last.ends_with(ext)) {
        return Some(url);
    }
    if NON_IMAGE_EXTENSIONS
        .iter()
        .any(|ext| lower_last.ends_with(ext))
    {
        return None;
    }
    // A TMDB image URL is only complete with a file name; a bare size base
    // (`/t/p/w600_and_h900_bestv2`) is a provider's unfilled template.
    if host.eq_ignore_ascii_case("image.tmdb.org") {
        return None;
    }
    // An extension-less path must name something, or carry a query that does.
    if last.is_empty() && !has_query {
        return None;
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_regular_image_files() {
        for url in [
            "https://cdn.example.com/logos/bein.png",
            "http://x.tv/a/b.JPG?v=2",
            "https://image.tmdb.org/t/p/w500/abc123.jpg",
            "https://x.tv/logo.svg",
        ] {
            assert_eq!(plausible_image_url(url).as_deref(), Some(url), "{url}");
        }
    }

    #[test]
    fn accepts_extensionless_cdn_logos() {
        for url in [
            "https://logos.provider.tv/channel/1234",
            "http://panel.example:8080/images/abcdef0123",
            "https://img.example.com/resize?id=5&w=300",
            "https://i.imgur.com/Xyz12",
        ] {
            assert_eq!(plausible_image_url(url).as_deref(), Some(url), "{url}");
        }
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            plausible_image_url("  https://x.tv/l.png \n").as_deref(),
            Some("https://x.tv/l.png")
        );
    }

    #[test]
    fn encodes_spaces_in_file_names() {
        assert_eq!(
            plausible_image_url("http://x.tv/logos/Bein Sports.png").as_deref(),
            Some("http://x.tv/logos/Bein%20Sports.png")
        );
    }

    #[test]
    fn rejects_garbage_and_templates() {
        for url in [
            "",
            "   ",
            "null",
            "N/A",
            "logo.png",
            "ftp://x.tv/logo.png",
            "data:image/png;base64,AAAA",
            "https://",
            "https://x.tv",
            "https://x.tv/",
            "https://image.tmdb.org/t/p/w600_and_h900_bestv2",
            "https://image.tmdb.org/t/p/w500/",
            "http://x.tv/live/u/p/123.ts",
            "http://x.tv/get.php?username=a",
            "http://x.tv/list.m3u8",
            "https://x.tv/a\tlogo.png",
        ] {
            assert_eq!(plausible_image_url(url), None, "{url:?}");
        }
    }
}
