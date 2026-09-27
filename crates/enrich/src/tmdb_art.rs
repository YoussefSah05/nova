//! Title artwork from TMDB's `/{movie,tv}/{id}/images`: a title logo (the
//! clear-logo wordmark) in the metadata language and a text-free backdrop.
//!
//! Pure selection over the images body. Language precedence for logos: the
//! metadata language, then English, then language-neutral; a logo in any other
//! language is never shown (a Korean wordmark on a French UI reads as a bug).
//! Backdrops prefer text-free art (`iso_639_1: null`) so a logo can sit on top;
//! when there is none the caller keeps the backdrop it already has.

use serde::Deserialize;

use crate::EnrichError;

/// Bump when the selection below changes; stored art older than this is
/// fetched again when next shown.
pub const ARTWORK_VERSION: i64 = 1;

const IMG_BASE: &str = "https://image.tmdb.org/t/p";
const LOGO_SIZE: &str = "w500";
const BACKDROP_SIZE: &str = "w1280";

#[derive(Debug, Clone, Deserialize)]
pub struct ImageEntry {
    pub file_path: String,
    #[serde(default)]
    pub iso_639_1: Option<String>,
    #[serde(default)]
    pub vote_average: f64,
    #[serde(default)]
    pub vote_count: i64,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TmdbImages {
    #[serde(default)]
    pub logos: Vec<ImageEntry>,
    #[serde(default)]
    pub backdrops: Vec<ImageEntry>,
}

pub fn parse_images(body: &str) -> Result<TmdbImages, EnrichError> {
    serde_json::from_str(body).map_err(|e| EnrichError::Parse(e.to_string()))
}

/// A chosen image, ready to display.
#[derive(Debug, Clone, PartialEq)]
pub struct PickedImage {
    pub url: String,
    /// ISO 639-1 of the text in the image; `None` when it has none.
    pub language: Option<String>,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TitleArt {
    pub logo: Option<PickedImage>,
    pub backdrop: Option<PickedImage>,
}

/// The ISO 639-1 part of a BCP-47 tag (`fr-FR` → `fr`).
pub fn language_code(bcp47: &str) -> String {
    bcp47
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// `include_image_language` for a metadata language: its own, English and
/// language-neutral images.
pub fn include_image_language(bcp47: &str) -> String {
    let code = language_code(bcp47);
    if code.is_empty() || code == "en" {
        "en,null".into()
    } else {
        format!("{code},en,null")
    }
}

fn picked(e: &ImageEntry, size: &str) -> PickedImage {
    PickedImage {
        url: format!("{IMG_BASE}/{size}{}", e.file_path),
        language: e.iso_639_1.clone().filter(|l| !l.is_empty()),
        width: e.width,
        height: e.height,
    }
}

/// Best-first: TMDB community votes, then vote count, then resolution.
fn quality(e: &ImageEntry) -> (i64, i64, i64) {
    ((e.vote_average * 1000.0) as i64, e.vote_count, e.width)
}

fn usable(e: &ImageEntry) -> bool {
    e.file_path.starts_with('/') && e.file_path.len() > 1
}

/// Choose the title logo and text-free backdrop for `metadata_language`.
pub fn select_title_art(images: &TmdbImages, metadata_language: &str) -> TitleArt {
    let want = language_code(metadata_language);
    let lang_rank = |e: &ImageEntry| -> Option<u8> {
        match e.iso_639_1.as_deref().filter(|l| !l.is_empty()) {
            Some(l) if l.eq_ignore_ascii_case(&want) => Some(0),
            Some(l) if l.eq_ignore_ascii_case("en") => Some(1),
            None => Some(2),
            Some(_) => None,
        }
    };
    let logo = images
        .logos
        .iter()
        .filter(|e| usable(e))
        .filter_map(|e| lang_rank(e).map(|rank| (rank, e)))
        // Raster before SVG: sizes are known and every webview renders them.
        .min_by_key(|(rank, e)| {
            let svg = e.file_path.to_ascii_lowercase().ends_with(".svg");
            let (votes, count, width) = quality(e);
            (*rank, svg, -votes, -count, -width)
        })
        .map(|(_, e)| picked(e, LOGO_SIZE));
    let backdrop = images
        .backdrops
        .iter()
        .filter(|e| usable(e) && e.iso_639_1.as_deref().is_none_or(str::is_empty))
        .max_by_key(|e| quality(e))
        .map(|e| picked(e, BACKDROP_SIZE));
    TitleArt { logo, backdrop }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{
      "id": 238,
      "backdrops": [
        {"file_path": "/text-en.jpg", "iso_639_1": "en", "vote_average": 9.0, "vote_count": 50, "width": 3840, "height": 2160},
        {"file_path": "/clean-low.jpg", "iso_639_1": null, "vote_average": 5.2, "vote_count": 3, "width": 1920, "height": 1080},
        {"file_path": "/clean-best.jpg", "iso_639_1": null, "vote_average": 5.6, "vote_count": 10, "width": 3840, "height": 2160}
      ],
      "logos": [
        {"file_path": "/logo-en.png", "iso_639_1": "en", "vote_average": 5.8, "vote_count": 12, "width": 1000, "height": 400},
        {"file_path": "/logo-fr.svg", "iso_639_1": "fr", "vote_average": 5.5, "vote_count": 2, "width": 800, "height": 300},
        {"file_path": "/logo-fr.png", "iso_639_1": "fr", "vote_average": 5.3, "vote_count": 1, "width": 800, "height": 300},
        {"file_path": "/logo-ko.png", "iso_639_1": "ko", "vote_average": 10.0, "vote_count": 99, "width": 2000, "height": 800}
      ],
      "posters": []
    }"#;

    #[test]
    fn prefers_the_metadata_language_logo_then_english() {
        let images = parse_images(BODY).unwrap();
        let fr = select_title_art(&images, "fr-FR");
        let logo = fr.logo.unwrap();
        assert_eq!(logo.url, "https://image.tmdb.org/t/p/w500/logo-fr.png");
        assert_eq!(logo.language.as_deref(), Some("fr"));

        let de = select_title_art(&images, "de-DE");
        assert_eq!(
            de.logo.unwrap().url,
            "https://image.tmdb.org/t/p/w500/logo-en.png",
            "no German logo: English, never the higher-voted Korean one"
        );
    }

    #[test]
    fn picks_the_best_text_free_backdrop() {
        let art = select_title_art(&parse_images(BODY).unwrap(), "en-US");
        let backdrop = art.backdrop.unwrap();
        assert_eq!(
            backdrop.url,
            "https://image.tmdb.org/t/p/w1280/clean-best.jpg"
        );
        assert_eq!(backdrop.language, None);
    }

    #[test]
    fn foreign_only_art_yields_nothing() {
        let body = r#"{"logos": [{"file_path": "/ko.png", "iso_639_1": "ko"}],
                       "backdrops": [{"file_path": "/ko.jpg", "iso_639_1": "ko"}]}"#;
        assert_eq!(
            select_title_art(&parse_images(body).unwrap(), "en-US"),
            TitleArt::default()
        );
        assert_eq!(
            select_title_art(&parse_images("{}").unwrap(), "en-US"),
            TitleArt::default()
        );
    }

    #[test]
    fn language_neutral_logo_is_the_last_resort() {
        let body = r#"{"logos": [{"file_path": "/neutral.png", "iso_639_1": null},
                                 {"file_path": "", "iso_639_1": "en"}]}"#;
        let art = select_title_art(&parse_images(body).unwrap(), "fr-FR");
        assert_eq!(art.logo.unwrap().language, None);
    }

    #[test]
    fn include_languages_follow_the_metadata_language() {
        assert_eq!(include_image_language("fr-FR"), "fr,en,null");
        assert_eq!(include_image_language("en-US"), "en,null");
        assert_eq!(include_image_language("pt-BR"), "pt,en,null");
    }
}
