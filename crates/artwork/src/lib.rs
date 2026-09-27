//! Artwork: validated, on-disk cached posters, backdrops and logos.
//!
//! The desktop app serves this cache to the webview through the `nova-art://`
//! protocol, so every image the UI shows is fetched at most once, checked for
//! being a real image of sane size, and available offline afterwards.

pub mod cache;
pub mod directory;
pub mod validate;

pub use cache::{ArtworkCache, ArtworkError, CacheStats, CachedImage, DEFAULT_MAX_BYTES};
pub use validate::{sniff, validate, ImageFormat, Rejection, Validated};
